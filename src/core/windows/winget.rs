use async_trait::async_trait;
use crate::core::{CommandExecutor, Package, PackageManager, Result};
use once_cell::sync::OnceCell;
use tracing::info;

/// Windows Package Manager (winget)
pub struct WingetManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl WingetManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("where")
            .arg("winget")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for WingetManager {
    fn name(&self) -> &str {
        "winget"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], _sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via winget", packages.len());

        for package in packages {
            self.executor
                .run("winget", &["install", "--silent", "--exact", "--accept-source-agreements", "--accept-package-agreements", package], false)
                .await?;
        }

        Ok(())
    }

    async fn remove(&self, packages: &[String], _sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via winget", packages.len());

        for package in packages {
            self.executor
                .run("winget", &["uninstall", "--silent", package], false)
                .await?;
        }

        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("winget", &["list", "--accept-source-agreements"], false)
            .await?;

        let packages = output
            .lines()
            .skip(2) // Skip headers
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some(Package {
                        name: parts[0].to_string(),
                        version: parts.get(1).map(|s| s.to_string()),
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

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("Upgrading all winget packages");
        self.executor
            .run("winget", &["upgrade", "--all", "--silent", "--accept-source-agreements", "--accept-package-agreements"], false)
            .await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("winget", &["search", query, "--accept-source-agreements"], false)
            .await?;

        let packages = output
            .lines()
            .skip(2)
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some(Package {
                        name: parts[0].to_string(),
                        version: parts.get(1).map(|s| s.to_string()),
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
        false
    }
}
