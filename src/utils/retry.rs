use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Configuration for exponential backoff retry behavior.
/// Used to handle transient network failures in GitHub, Web, and Marketplace backends.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Configuration for quick, high-frequency retries.
    pub fn quick() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
        }
    }

    /// Configuration for persistent retries on unreliable connections.
    pub fn persistent() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
        }
    }
}

/// Retries a fallible async operation with exponential backoff.
/// This is used by the high-performance engine to ensure that transient 
/// IO errors don't crash long-running system transactions.
pub async fn retry<F, Fut, T, E>(config: RetryConfig, mut operation: F) -> std::result::Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0;
    let mut delay = config.initial_delay;

    loop {
        attempt += 1;

        match operation().await {
            Ok(result) => {
                if attempt > 1 {
                    debug!("Retry: Operation succeeded on attempt {}", attempt);
                }
                return Ok(result);
            }
            Err(err) => {
                if attempt >= config.max_attempts {
                    warn!(
                        "Retry: Operation failed after {} attempts. Final error: {}",
                        config.max_attempts, err
                    );
                    return Err(err);
                }

                warn!(
                    "Retry: Attempt {} failed: {}. Retrying in {:?}...",
                    attempt, err, delay
                );

                sleep(delay).await;

                // Calculate next delay using multiplier, capped at max_delay
                delay = Duration::from_secs_f64(
                    (delay.as_secs_f64() * config.backoff_multiplier)
                        .min(config.max_delay.as_secs_f64()),
                );
            }
        }
    }
}

/// Convenience wrapper for retrying with default settings.
pub async fn retry_default<F, Fut, T, E>(operation: F) -> std::result::Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
    E: std::fmt::Display,
{
    retry(RetryConfig::default(), operation).await
}

/// Synchronous retry logic for non-async system calls.
pub fn retry_sync<F, T, E>(max_attempts: u32, mut operation: F) -> std::result::Result<T, E>
where
    F: FnMut() -> std::result::Result<T, E>,
    E: std::fmt::Display,
{
    let mut last_error = None;

    for attempt in 1..=max_attempts {
        match operation() {
            Ok(result) => return Ok(result),
            Err(err) => {
                warn!("Retry Sync: Attempt {} failed: {}", attempt, err);
                last_error = Some(err);
            }
        }
    }

    // Safety: at least one attempt is made, so last_error will be Some if we get here.
    Err(last_error.expect("At least one retry attempt failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_retry_eventual_success() {
        let config = RetryConfig::quick();
        let counter = AtomicU32::new(0);

        let result: std::result::Result<i32, &str> = retry(config, || {
            let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
            async move {
                if attempt < 3 {
                    Err("temporary failure")
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}