use crate::core::{CommandExecutor, Package, PackageManager, Result};
use crate::core::manager::HealthStatus;
use crate::core::manager::HealthReport;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use tracing::info;

pub struct PnpmManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
	    #[allow(dead_code)] 
    settings: Option<HashMap<String, String>>,
}

impl PnpmManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
    fn check_available(&self) -> bool {
        std::process::Command::new("which").arg("pnpm").output().map(|o| o.status.success()).unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for PnpmManager {
    fn name(&self) -> &str { "pnpm" }
    fn is_available(&self) -> bool { *self.available.get_or_init(|| self.check_available()) }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() { return Ok(()); }
        info!("Installing {} packages via pnpm", packages.len());
        let mut args = vec!["add", "-g"];
        args.extend(packages.iter().map(|s| s.as_str()));
        self.executor.run("pnpm", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() { return Ok(()); }
        info!("Removing {} packages via pnpm", packages.len());
        let mut args = vec!["remove", "-g"];
        args.extend(packages.iter().map(|s| s.as_str()));
        self.executor.run("pnpm", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor.run_output("pnpm", &["list", "-g", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&output)?;
        let mut packages = Vec::new();
        if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
            for (name, data) in deps {
                let version = data.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                packages.push(Package { name: name.clone(), version, backend: self.name().to_string(), description: None, repository: None, size: None });
            }
        }
        Ok(packages)
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        self.executor.run("pnpm", &["update", "-g"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self.executor.run_output("pnpm", &["search", query], false).await?;
        let packages = output.lines().filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                Some(Package {
                    name: parts[0].to_string(),
                    version: Some(parts[1].to_string()),
                    backend: self.name().to_string(),
                    description: parts.get(2..).map(|p| p.join(" ")),
                    repository: None,
                    size: None,
                })
            } else { None }
        }).collect();
        Ok(packages)
    }

    fn supports_orphan_cleanup(&self) -> bool { false }
	async fn check_health(&self) -> Result<HealthReport> {
        // USE SETTINGS: Allow user to override brew path for health check
        let bin_name = self.settings.as_ref()
            .and_then(|s| s.get("binary_path"))
            .map(|s| s.as_str())
            .unwrap_or("pnpm");

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