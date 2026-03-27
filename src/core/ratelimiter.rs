use crate::core::Result;
use governor::{Quota, RateLimiter as GovernorRateLimiter};
use nonzero_ext::nonzero;
use std::num::NonZeroU32;
use std::sync::Arc;

/// Rate limiter for API calls (especially GitHub)
#[derive(Clone)]
pub struct RateLimiter {
    limiter: Arc<GovernorRateLimiter<governor::state::direct::NotKeyed, governor::state::InMemoryState, governor::clock::DefaultClock>>,
}

impl RateLimiter {
    /// Create a new rate limiter
    pub fn new(requests_per_minute: u32) -> Self {
        let quota = Quota::per_minute(NonZeroU32::new(requests_per_minute).unwrap_or(nonzero!(60u32)));
        let limiter = GovernorRateLimiter::direct(quota);

        Self {
            limiter: Arc::new(limiter),
        }
    }

    /// Create a rate limiter for GitHub API (60 requests per hour for unauthenticated)
    pub fn github() -> Self {
        Self::new(60)
    }

    /// Create a rate limiter for authenticated GitHub API (5000 requests per hour)
    pub fn github_authenticated() -> Self {
        Self::new(80)
    }

    /// Wait until a request can be made
    pub async fn wait(&self) -> Result<()> {
        self.limiter.until_ready().await;
        Ok(())
    }

    /// Try to make a request immediately, return error if rate limited
    pub fn try_request(&self) -> Result<()> {
        match self.limiter.check() {
            Ok(_) => Ok(()),
            Err(_) => Err(crate::core::Error::RateLimit),
        }
    }

    /// Execute a function with rate limiting
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
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(2);

        assert!(limiter.try_request().is_ok());
        assert!(limiter.try_request().is_ok());
        assert!(limiter.try_request().is_err());
    }

    #[tokio::test]
    async fn test_rate_limiter_wait() {
        let limiter = RateLimiter::new(60);

        let result = limiter.execute(|| async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }
}
