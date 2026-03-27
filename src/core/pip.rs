use async_trait::async_trait;
use crate::core::{CommandExecutor, Package, PackageManager, Result};
use once_cell::sync::OnceCell;
use tracing::info;

/// PIP Python package manager
pub struct PipManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl PipManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("pip")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for PipManager {
    fn name(&self) -> &str {
        "pip"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via pip", packages.len());

        let mut args = vec!["install"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("pip", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via pip", packages.len());

        let mut args = vec!["uninstall", "-y"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("pip", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("pip", &["list", "--format=json"], false)
            .await?;

        let packages: Vec<serde_json::Value> = serde_json::from_str(&output)?;
        
        let result = packages
            .into_iter()
            .filter_map(|pkg| {
                let name = pkg.get("name")?.as_str()?.to_string();
                let version = pkg.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                
                Some(Package {
                    name,
                    version,
                    backend: self.name().to_string(),
                    description: None,
                    repository: None,
                    size: None,
                })
            })
            .collect();

        Ok(result)
    }

    async fn update(&self, sudo: bool) -> Result<()> {
        info!("Upgrading pip itself");
        self.executor.run("pip", &["install", "--upgrade", "pip"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all pip packages");
        
        // Get list of upgradeable packages
        let output = self.executor
            .run_output("pip", &["list", "--outdated", "--format=json"], false)
            .await?;

        let packages: Vec<serde_json::Value> = serde_json::from_str(&output).unwrap_or_default();
        
        for pkg in packages {
            if let Some(name) = pkg.get("name").and_then(|n| n.as_str()) {
                self.executor.run("pip", &["install", "--upgrade", name], sudo).await.ok();
            }
        }

        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // Note: pip search is disabled on PyPI, so we'll just return empty
        // In a real implementation, we'd query PyPI API
        let _ = query;
        Ok(Vec::new())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }
}
