use crate::core::Result;

#[derive(Debug, Clone)]
pub struct RateLimiter {
    _private: (),
}

impl RateLimiter {
    pub fn new(_requests_per_minute: u32) -> Self {
        Self { _private: () }
    }

    pub fn github() -> Self {
        Self::new(1)
    }

    pub fn github_authenticated() -> Self {
        Self::new(80)
    }

    pub fn vscode_marketplace() -> Self {
        Self::new(30)
    }

    pub async fn wait(&self) -> Result<()> {
        Ok(())
    }

    pub fn try_request(&self) -> Result<()> {
        Ok(())
    }

    pub async fn execute<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        self.wait().await?;
        f().await
    }
}