use crate::core::{CommandExecutor, Package, PackageManager, Result};
use crate::core::manager::HealthStatus;
use crate::core::manager::HealthReport;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};
use std::collections::HashMap;

/// Composer PHP package manager
pub struct ComposerManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
	    #[allow(dead_code)] 

	    settings: Option<HashMap<String, String>>,
}

impl ComposerManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
    Self { executor, available: OnceCell::new(), settings }
}

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("composer")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for ComposerManager {
    fn name(&self) -> &str {
        "composer"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via composer", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["global", "require"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("composer", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via composer", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["global", "remove"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("composer", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("composer", &["global", "show", "--format=json"], false)
            .await?;

        let json: serde_json::Value = serde_json::from_str(&output)?;

        let mut packages = Vec::new();
        if let Some(installed) = json.get("installed").and_then(|i| i.as_array()) {
            for pkg in installed {
                if let Some(name) = pkg.get("name").and_then(|n| n.as_str()) {
                    let version = pkg
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let description = pkg
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(|s| s.to_string());

                    packages.push(Package {
                        name: name.to_string(),
                        version,
                        backend: self.name().to_string(),
                        description,
                        repository: None,
                        size: None,
                    });
                }
            }
        }

        Ok(packages)
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all composer packages");
        self.executor
            .run("composer", &["global", "update"], sudo)
            .await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("composer", &["search", query], false)
            .await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if !parts.is_empty() {
                    Some(Package {
                        name: parts[0].trim().to_string(),
                        version: None,
                        backend: self.name().to_string(),
                        description: parts.get(1).map(|s| s.trim().to_string()),
                        repository: None,
                        size: None,
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(packages)
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }
	async fn check_health(&self) -> Result<HealthReport> {
        // USE SETTINGS: Allow user to override brew path for health check
        let bin_name = self.settings.as_ref()
            .and_then(|s| s.get("binary_path"))
            .map(|s| s.as_str())
            .unwrap_or("composer");

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
