use async_trait::async_trait;
use crate::core::{CommandExecutor, Package, PackageManager, Result};
use once_cell::sync::OnceCell;
use tracing::info;

/// Flatpak package manager
pub struct FlatpakManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl FlatpakManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("flatpak")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for FlatpakManager {
    fn name(&self) -> &str {
        "flatpak"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via flatpak", packages.len());

        let mut args = vec!["install", "-y"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via flatpak", packages.len());

        let mut args = vec!["uninstall", "-y"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("flatpak", &["list", "--app", "--columns=application,version"], false)
            .await?;

        let packages = output
            .lines()
            .skip(1) // Skip header
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 2 {
                    Some(Package {
                        name: parts[0].trim().to_string(),
                        version: Some(parts[1].trim().to_string()),
                        backend: self.name().to_string(),
                        description: None,
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

    async fn update(&self, sudo: bool) -> Result<()> {
        info!("Updating flatpak packages");
        self.executor.run("flatpak", &["update", "-y"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all flatpak packages");
        self.executor.run("flatpak", &["update", "-y"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("flatpak", &["search", "--columns=name,version"], false)
            .await?;

        let packages = output
            .lines()
            .skip(1) // Skip header
            .filter(|line| line.to_lowercase().contains(&query.to_lowercase()))
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 2 {
                    Some(Package {
                        name: parts[0].trim().to_string(),
                        version: Some(parts[1].trim().to_string()),
                        backend: self.name().to_string(),
                        description: None,
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
        true
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        info!("Cleaning unused flatpak data");
        let _ = sudo; // flatpak doesn't need sudo for user installs
        self.executor.run("flatpak", &["uninstall", "--unused", "-y"], false).await?;
        Ok(())
    }
}
