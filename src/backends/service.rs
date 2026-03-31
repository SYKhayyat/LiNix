use crate::core::{CommandExecutor, Package, PackageManager, Result, PackageSpec};
use async_trait::async_trait;
use std::collections::HashMap;
use tracing::info;

pub struct ServiceManager {
    executor: CommandExecutor,
}

impl ServiceManager {
    pub fn new(executor: CommandExecutor, _settings: Option<HashMap<String, String>>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl PackageManager for ServiceManager {
    fn name(&self) -> &str { "service" }
    fn is_available(&self) -> bool { true }

    async fn install_with_options(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            let action = spec.options.get("status").map(|s| s.as_str()).unwrap_or("running");
            let enable = spec.options.get("enabled").map(|s| s == "true").unwrap_or(true);

            #[cfg(unix)] {
                if enable { self.executor.run("systemctl", &["enable", &spec.name], true).await?; }
                let cmd = match action { "stopped" => "stop", _ => "start" };
                self.executor.run("systemctl", &[cmd, &spec.name], true).await?;
            }
            #[cfg(windows)] {
                let start_type = if enable { "auto" } else { "demand" };
                self.executor.run("sc", &["config", &spec.name, "start=", start_type], false).await?;
                let cmd = match action { "stopped" => "stop", _ => "start" };
                self.executor.run("net", &[cmd, &spec.name], false).await?;
            }
            info!("Service {} set to {} (enabled: {})", spec.name, action, enable);
        }
        Ok(())
    }
    async fn install(&self, _: &[String], _: bool) -> Result<()> { Ok(()) }
    async fn remove(&self, _: &[String], _: bool) -> Result<()> { Ok(()) }
    async fn list_installed(&self) -> Result<Vec<Package>> { Ok(vec![]) }
}