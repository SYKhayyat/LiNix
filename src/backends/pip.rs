use crate::core::{CommandExecutor, Package, PackageManager, Result};
use crate::core::manager::HealthStatus;
use crate::core::manager::HealthReport;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PipManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    #[allow(dead_code)] settings: Option<HashMap<String, String>>,
}

impl PipManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
    async fn get_cmd(&self) -> &str {
        if self.executor.command_exists("pip3").await { "pip3" } else { "pip" }
    }
}

#[async_trait]
impl PackageManager for PipManager {
    fn name(&self) -> &str { "pip" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("pip3").arg("--version").output().is_ok() ||
            std::process::Command::new("pip").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        let cmd = self.get_cmd().await;
        let mut args = vec!["install", "--upgrade"];
        args.extend(packages.iter().map(|s| s.as_str()));
        self.executor.run(cmd, &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        let cmd = self.get_cmd().await;
        let mut args = vec!["uninstall", "-y"];
        args.extend(packages.iter().map(|s| s.as_str()));
        self.executor.run(cmd, &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let cmd = self.get_cmd().await;
        let output = self.executor.run_output(cmd, &["list", "--format=json"], false).await?;
        let json: Vec<serde_json::Value> = serde_json::from_str(&output).unwrap_or_default();
        Ok(json.into_iter().filter_map(|p| {
            let name = p.get("name")?.as_str()?.to_string();
            let version = p.get("version")?.as_str().map(|s| s.to_string());
            Some(Package { name, version, backend: self.name().to_string(), description: None, repository: None, size: None })
        }).collect())
    }
	async fn check_health(&self) -> Result<HealthReport> {
        // USE SETTINGS: Allow user to override brew path for health check
        let bin_name = self.settings.as_ref()
            .and_then(|s| s.get("binary_path"))
            .map(|s| s.as_str())
            .unwrap_or("pip");

        if self.executor.command_exists(bin_name).await {
            Ok(HealthReport { status: HealthStatus::Ok, message: None })
        } else {
            Ok(HealthReport { 
                status: HealthStatus::Error, 
                message: Some(format!("{} not found", bin_name)) 
            })
        }
    }
}