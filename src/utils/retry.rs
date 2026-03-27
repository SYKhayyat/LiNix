use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Configuration for retry behavior
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
    pub fn quick() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
        }
    }

    pub fn persistent() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
        }
    }
}

/// Retry a fallible async operation with exponential backoff
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
                    debug!("Operation succeeded on attempt {}", attempt);
                }
                return Ok(result);
            }
            Err(err) => {
                if attempt >= config.max_attempts {
                    warn!(
                        "Operation failed after {} attempts: {}",
                        config.max_attempts, err
                    );
                    return Err(err);
                }

                warn!(
                    "Attempt {} failed: {}. Retrying in {:?}...",
                    attempt, err, delay
                );

                sleep(delay).await;

                delay = Duration::from_secs_f64(
                    (delay.as_secs_f64() * config.backoff_multiplier)
                        .min(config.max_delay.as_secs_f64()),
                );
            }
        }
    }
}

/// Retry with default config
pub async fn retry_default<F, Fut, T, E>(operation: F) -> std::result::Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, E>>,
    E: std::fmt::Display,
{
    retry(RetryConfig::default(), operation).await
}

/// Retry with a simple counter (no async)
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
                warn!("Attempt {} failed: {}", attempt, err);
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_retry_success_first_attempt() {
        let config = RetryConfig::quick();

        let result: std::result::Result<i32, &str> = retry(config, || async { Ok(42) }).await;

        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_success_after_failures() {
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

    #[tokio::test]
    async fn test_retry_all_attempts_fail() {
        let config = RetryConfig {
            max_attempts: 2,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(100),
            backoff_multiplier: 2.0,
        };

        let counter = AtomicU32::new(0);

        let result: std::result::Result<i32, &str> = retry(config, || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Err("permanent failure") }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }
}
