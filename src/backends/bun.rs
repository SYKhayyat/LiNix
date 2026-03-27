use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// Bun JavaScript package manager/runtime
pub struct BunManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl BunManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("bun")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for BunManager {
    fn name(&self) -> &str {
        "bun"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via bun", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["add", "-g"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("bun", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via bun", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["remove", "-g"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("bun", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // Bun doesn't have a built-in global list command yet
        Ok(Vec::new())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading bun");
        self.executor.run("bun", &["upgrade"], sudo).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }
}
