use crate::core::Result;
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Jitter, Quota, RateLimiter as GovRateLimiter};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// A thread-safe, high-performance rate limiter implementation using the governor crate.
///
/// This implementation provides true backpressure and asynchronous waiting,
/// ensuring LiNix respects API quotas for GitHub, VS Code Marketplace, and other
/// remote backends without wasting CPU cycles.
///
/// Hardened for Phase 1.3: Provides real backpressure for API-driven backends.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<GovRateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    description: String,
}

impl RateLimiter {
    /// Creates a new rate limiter with a specific request-per-minute quota.
    pub fn new(requests_per_minute: u32, description: &str) -> Self {
        // Ensure we have at least 1 request per minute to avoid division by zero errors
        let rpm = requests_per_minute.max(1);
        let quota = Quota::per_minute(NonZeroU32::new(rpm).expect("RPM is guaranteed > 0"));

        Self {
            inner: Arc::new(GovRateLimiter::direct(quota)),
            description: description.to_string(),
        }
    }

    /// Optimized rate limiter for unauthenticated GitHub API access.
    pub fn github() -> Self {
        // GitHub allows 60 requests per hour for unauthenticated IPs.
        // We set a strict 1 request per minute limit here.
        Self::new(1, "GitHub (Unauthenticated)")
    }

    /// Optimized rate limiter for authenticated GitHub API access.
    pub fn github_authenticated() -> Self {
        // GitHub allows 5,000 requests per hour for authenticated users.
        // We set a limit of ~80 per minute to stay safely within the window.
        Self::new(80, "GitHub (Authenticated)")
    }

    /// Optimized rate limiter for Visual Studio Code Marketplace.
    pub fn vscode_marketplace() -> Self {
        Self::new(30, "VS Code Marketplace")
    }

    /// Asynchronously waits until a request permit is available.
    pub async fn wait(&self) -> Result<()> {
        debug!("RateLimiter [{}]: Waiting for permit...", self.description);
        self.inner.until_ready().await;
        Ok(())
    }

    /// Executes an asynchronous operation while respecting the rate limit.
    pub async fn execute<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        // 1. Wait for permit with a small jitter (up to 150ms) to desynchronize parallel workers
        let jitter = Jitter::up_to(Duration::from_millis(150));

        self.inner.until_ready_with_jitter(jitter).await;

        // 2. Execute the task
        match f().await {
            Ok(val) => Ok(val),
            Err(e) => {
                let err_msg = format!("{:?}", e);
                if err_msg.contains("429") || err_msg.contains("RateLimit") {
                    warn!(
                        "RateLimiter [{}]: Remote API returned 429 (Too Many Requests). Local limits may need tightening.",
                        self.description
                    );
                }
                Err(e)
            }
        }
    }

    /// Returns a reference to the description of this limiter.
    pub fn description(&self) -> &str {
        &self.description
    }
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("description", &self.description)
            .finish()
    }
}
