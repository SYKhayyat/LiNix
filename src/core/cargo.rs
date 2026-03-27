use async_trait::async_trait;
use crate::core::{CommandExecutor, Package, PackageManager, Result};
use once_cell::sync::OnceCell;
use tracing::info;

/// Cargo Rust package manager
pub struct CargoManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl CargoManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("cargo")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for CargoManager {
    fn name(&self) -> &str {
        "cargo"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via cargo", packages.len());

        let mut args = vec!["install"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("cargo", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via cargo", packages.len());

        let mut args = vec!["uninstall"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("cargo", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("cargo", &["install", "--list"], false)
            .await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if !line.is_empty() && !line.starts_with('-') {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        return Some(Package {
                            name: parts[0].to_string(),
                            version: Some(parts[1].to_string()),
                            backend: self.name().to_string(),
                            description: None,
                            repository: None,
                            size: None,
                        });
                    }
                }
                None
            })
            .collect();

        Ok(packages)
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all cargo packages");
        self.executor.run("cargo", &["install-update", "-a"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("cargo", &["search", query], false)
            .await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if !line.starts_with('#') && !line.is_empty() {
                    let parts: Vec<&str> = line.split('=').collect();
                    if parts.len() >= 2 {
                        let name = parts[0].trim().to_string();
                        let version = parts[1].trim().trim_matches('"').to_string();
                        
                        return Some(Package {
                            name,
                            version: Some(version),
                            backend: self.name().to_string(),
                            description: None,
                            repository: None,
                            size: None,
                        });
                    }
                }
                None
            })
            .collect();

        Ok(packages)
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }
}
