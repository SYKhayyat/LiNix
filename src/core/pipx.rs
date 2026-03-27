use async_trait::async_trait;
use crate::core::{CommandExecutor, Package, PackageManager, Result};
use once_cell::sync::OnceCell;
use tracing::info;

/// PIPX Python application manager
pub struct PipxManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl PipxManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("pipx")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for PipxManager {
    fn name(&self) -> &str {
        "pipx"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via pipx", packages.len());

        for package in packages {
            let mut args = vec!["install"];
            args.push(package);
            self.executor.run("pipx", &args, sudo).await?;
        }

        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via pipx", packages.len());

        for package in packages {
            let args = vec!["uninstall", package];
            self.executor.run("pipx", &args, sudo).await.ok();
        }

        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("pipx", &["list", "--json"], false)
            .await?;

        let json: serde_json::Value = serde_json::from_str(&output)?;
        
        let mut packages = Vec::new();
        if let Some(venvs) = json.get("venvs").and_then(|v| v.as_object()) {
            for (name, data) in venvs {
                let version = data
                    .get("metadata")
                    .and_then(|m| m.get("main_package"))
                    .and_then(|p| p.get("version"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                packages.push(Package {
                    name: name.clone(),
                    version,
                    backend: self.name().to_string(),
                    description: None,
                    repository: None,
                    size: None,
                });
            }
        }

        Ok(packages)
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all pipx packages");
        self.executor.run("pipx", &["upgrade-all"], sudo).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }
}
