use crate::core::{CommandExecutor, Package, PackageManager, Result};
use crate::core::manager::HealthStatus;
use crate::core::manager::HealthReport;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct MiseManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    #[allow(dead_code)] settings: Option<HashMap<String, String>>,
}

impl MiseManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
}

#[async_trait]
impl PackageManager for MiseManager {
    fn name(&self) -> &str { "mise" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("mise").arg("--version").output().is_ok())
    }
    async fn install(&self, packages: &[String], _sudo: bool) -> Result<()> {
        for p in packages { self.executor.run("mise", &["tool", "install", p], false).await?; }
        Ok(())
    }
    async fn remove(&self, packages: &[String], _sudo: bool) -> Result<()> {
        for p in packages { self.executor.run("mise", &["tool", "uninstall", p], false).await?; }
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("mise", &["tool", "list", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut pkgs = Vec::new();
        if let Some(arr) = json.as_array() {
            for v in arr {
                if let Some(n) = v.get("name").and_then(|n| n.as_str()) {
                    let ver = v.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                    pkgs.push(Package { name: n.to_string(), version: ver, backend: "mise".to_string(), description: None, repository: None, size: None });
                }
            }
        }
        Ok(pkgs)
    }
	async fn check_health(&self) -> Result<HealthReport> {
        // USE SETTINGS: Allow user to override brew path for health check
        let bin_name = self.settings.as_ref()
            .and_then(|s| s.get("binary_path"))
            .map(|s| s.as_str())
            .unwrap_or("mise");

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