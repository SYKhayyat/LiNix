use crate::core::Result;
use governor::{Quota, RateLimiter as GovernorRateLimiter};
use nonzero_ext::nonzero;
use std::num::NonZeroU32;
use std::sync::Arc;

/// A high-performance, thread-safe rate limiter for remote API calls.
/// Utilizes the token-bucket algorithm via the 'governor' crate.
/// This is essential for Phase 2 parallel search and Phase 4 API integrations 
/// to ensure LiNix does not get IP-banned by GitHub, VS Code Marketplace, or Crates.io.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<GovernorRateLimiter<governor::state::direct::NotKeyed, governor::state::InMemoryState, governor::clock::DefaultClock>>,
}

impl RateLimiter {
    /// Creates a new rate limiter with a specified number of requests allowed per minute.
    pub fn new(requests_per_minute: u32) -> Self {
        let quota = Quota::per_minute(NonZeroU32::new(requests_per_minute).unwrap_or(nonzero!(60u32)));
        let limiter = GovernorRateLimiter::direct(quota);

        Self {
            inner: Arc::new(limiter),
        }
    }

    /// Pre-configured rate limiter for the unauthenticated GitHub API.
    /// GitHub allows 60 requests per hour for unauthenticated IPs.
    pub fn github() -> Self {
        Self::new(1) 
    }

    /// Pre-configured rate limiter for the authenticated GitHub API.
    /// Authenticated users typically get 5000 requests per hour.
    pub fn github_authenticated() -> Self {
        Self::new(80)
    }

    /// Asynchronously waits until a rate-limit permit is available.
    /// This is the preferred method for use in the tokio-based worker pool.
    pub async fn wait(&self) -> Result<()> {
        self.inner.until_ready().await;
        Ok(())
    }

    /// Non-blocking check for a permit. 
    /// Returns an Err(Error::RateLimit) immediately if the bucket is empty.
    pub fn try_request(&self) -> Result<()> {
        match self.inner.check() {
            Ok(_) => Ok(()),
            Err(_) => Err(crate::core::Error::RateLimit),
        }
    }

    /// A high-level execution wrapper that handles the waiting logic automatically.
    pub async fn execute<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        self.wait().await?;
        f().await
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_blocking() {
        // Limit to 2 per minute
        let limiter = RateLimiter::new(2);

        // First two should succeed immediately
        assert!(limiter.try_request().is_ok());
        assert!(limiter.try_request().is_ok());
        
        // Third should fail immediately
        assert!(limiter.try_request().is_err());
    }

    #[tokio::test]
    async fn test_rate_limiter_wrapper() {
        let limiter = RateLimiter::new(60);
        let result = limiter.execute(|| async { Ok(100) }).await;
        assert_eq!(result.unwrap(), 100);
    }
}