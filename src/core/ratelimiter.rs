use crate::core::{Error, Result};
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Jitter, Quota, RateLimiter as GovRateLimiter};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<GovRateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    description: String,
}

impl RateLimiter {
    pub fn new(requests_per_minute: u32, description: &str) -> Self {
        // The clamp is what makes the `expect` below unreachable: a caller-supplied 0 is a
        // configuration mistake, not a request to block every call forever.
        let rpm = requests_per_minute.max(1);
        let quota = Quota::per_minute(NonZeroU32::new(rpm).expect("RPM is guaranteed > 0"));

        Self {
            inner: Arc::new(GovRateLimiter::direct(quota)),
            description: description.to_string(),
        }
    }

    pub fn github() -> Self {
        // GitHub allows 60 requests per hour for unauthenticated IPs.
        Self::new(1, "GitHub (Unauthenticated)")
    }

    pub fn github_authenticated() -> Self {
        // GitHub allows 5,000 requests per hour for authenticated users; ~80/min stays inside
        // the window even if every minute is used to the limit.
        Self::new(80, "GitHub (Authenticated)")
    }

    pub fn vscode_marketplace() -> Self {
        Self::new(30, "VS Code Marketplace")
    }

    pub async fn wait(&self) -> Result<()> {
        debug!("RateLimiter [{}]: Waiting for permit...", self.description);
        self.inner.until_ready().await;
        Ok(())
    }

    pub async fn execute<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        // Jitter desynchronizes parallel workers that would otherwise all wake on the same
        // permit boundary and burst.
        let jitter = Jitter::up_to(Duration::from_millis(150));

        self.inner.until_ready_with_jitter(jitter).await;

        match f().await {
            Ok(val) => Ok(val),
            Err(e) => {
                // Read off the variant, not off the rendered message: `format!("{:?}")` also
                // matched any error that happened to contain "429" — a version string, a
                // package name — and missed a real rate limit whose text said neither.
                if matches!(e, Error::RateLimit(_)) {
                    warn!(
                        "RateLimiter [{}]: Remote API returned 429 (Too Many Requests). Local limits may need tightening.",
                        self.description
                    );
                }
                Err(e)
            }
        }
    }

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
