use async_trait::async_trait;
use crate::core::{CommandExecutor, Package, PackageManager, Result};
use once_cell::sync::OnceCell;
use tracing::info;

/// Yarn JavaScript package manager
pub struct YarnManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl YarnManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("yarn")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for YarnManager {
    fn name(&self) -> &str {
        "yarn"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via yarn", packages.len());

        let mut args = vec!["global", "add"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("yarn", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via yarn", packages.len());

        let mut args = vec!["global", "remove"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("yarn", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("yarn", &["global", "list", "--json"], false)
            .await?;

        let json: serde_json::Value = serde_json::from_str(&output)?;
        
        let mut packages = Vec::new();
        if let Some(data) = json.get("data") {
            if let Some(trees) = data.get("trees").and_then(|t| t.as_array()) {
                for tree in trees {
                    if let Some(name) = tree.get("name").and_then(|n| n.as_str()) {
                        // Parse "name@version" format
                        let parts: Vec<&str> = name.split('@').collect();
                        if !parts.is_empty() {
                            let pkg_name = parts[0].to_string();
                            let version = parts.get(1).map(|s| s.to_string());

                            packages.push(Package {
                                name: pkg_name,
                                version,
                                backend: self.name().to_string(),
                                description: None,
                                repository: None,
                                size: None,
                            });
                        }
                    }
                }
            }
        }

        Ok(packages)
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all yarn packages");
        self.executor.run("yarn", &["global", "upgrade"], sudo).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }
}
