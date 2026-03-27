use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// Snap package manager
pub struct SnapManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl SnapManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("snap")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for SnapManager {
    fn name(&self) -> &str {
        "snap"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via snap", packages.len());
        debug!("Packages: {:?}", packages);

        for package in packages {
            self.executor
                .run("snap", &["install", package], sudo)
                .await?;
        }

        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via snap", packages.len());
        debug!("Packages: {:?}", packages);

        for package in packages {
            self.executor
                .run("snap", &["remove", package], sudo)
                .await?;
        }

        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor.run_output("snap", &["list"], false).await?;

        let packages = output
            .lines()
            .skip(1) // Skip header
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some(Package {
                        name: parts[0].to_string(),
                        version: Some(parts[1].to_string()),
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
        info!("Refreshing snap packages");
        self.executor.run("snap", &["refresh"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all snap packages");
        self.executor.run("snap", &["refresh"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("snap", &["search", query], false)
            .await?;

        let packages = output
            .lines()
            .skip(1) // Skip header
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some(Package {
                        name: parts[0].to_string(),
                        version: Some(parts[1].to_string()),
                        backend: self.name().to_string(),
                        description: parts.get(4..).map(|p| p.join(" ")),
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

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output("snap", &["info", package], false)
            .await;

        match output {
            Ok(out) => {
                let mut name = None;
                let mut version = None;
                let mut description = None;

                for line in out.lines() {
                    let line = line.trim();
                    if let Some(value) = line.strip_prefix("name:") {
                        name = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("installed:") {
                        version = Some(
                            value
                                .trim()
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .to_string(),
                        );
                    } else if let Some(value) = line.strip_prefix("summary:") {
                        description = Some(value.trim().to_string());
                    }
                }

                Ok(name.map(|n| Package {
                    name: n,
                    version,
                    backend: self.name().to_string(),
                    description,
                    repository: None,
                    size: None,
                }))
            }
            Err(_) => Ok(None),
        }
    }
}
