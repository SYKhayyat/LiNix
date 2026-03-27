use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

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
        debug!("Packages: {:?}", packages);

        for package in packages {
            self.executor
                .run("flatpak", &["install", "-y", package], sudo)
                .await?;
        }

        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via flatpak", packages.len());
        debug!("Packages: {:?}", packages);

        for package in packages {
            self.executor
                .run("flatpak", &["uninstall", "-y", package], sudo)
                .await?;
        }

        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output(
                "flatpak",
                &["list", "--app", "--columns=application,version"],
                false,
            )
            .await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('\t').collect();
                if !parts.is_empty() {
                    Some(Package {
                        name: parts[0].trim().to_string(),
                        version: parts.get(1).map(|s| s.trim().to_string()),
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
        self.executor
            .run("flatpak", &["update", "-y"], sudo)
            .await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all flatpak packages");
        self.executor
            .run("flatpak", &["update", "-y"], sudo)
            .await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("flatpak", &["search", query], false)
            .await?;

        let packages = output
            .lines()
            .skip(1) // Skip header if present
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('\t').collect();
                if !parts.is_empty() {
                    Some(Package {
                        name: parts.get(1).unwrap_or(&parts[0]).trim().to_string(),
                        version: parts.get(2).map(|s| s.trim().to_string()),
                        backend: self.name().to_string(),
                        description: parts.first().map(|s| s.trim().to_string()),
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

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        info!("Cleaning unused flatpak data");
        self.executor
            .run("flatpak", &["uninstall", "--unused", "-y"], sudo)
            .await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        true
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output("flatpak", &["info", package], false)
            .await;

        match output {
            Ok(out) => {
                let mut name = None;
                let mut version = None;
                let mut description = None;
                let mut size = None;

                for line in out.lines() {
                    let line = line.trim();
                    if let Some(value) = line.strip_prefix("ID:") {
                        name = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Version:") {
                        version = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Description:") {
                        description = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Installed:") {
                        let parts: Vec<&str> = value.trim().split_whitespace().collect();
                        if let Some(num) = parts.first() {
                            if let Ok(n) = num.parse::<u64>() {
                                size = Some(n);
                            }
                        }
                    }
                }

                Ok(name.map(|n| Package {
                    name: n,
                    version,
                    backend: self.name().to_string(),
                    description,
                    repository: None,
                    size,
                }))
            }
            Err(_) => Ok(None),
        }
    }
}
