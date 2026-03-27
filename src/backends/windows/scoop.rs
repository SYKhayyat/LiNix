use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// Scoop package manager for Windows
pub struct ScoopManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl ScoopManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("where")
            .arg("scoop")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for ScoopManager {
    fn name(&self) -> &str {
        "scoop"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], _sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via scoop", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["install"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("scoop", &args, false).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], _sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via scoop", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["uninstall"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("scoop", &args, false).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor.run_output("scoop", &["list"], false).await?;

        let packages = output
            .lines()
            .skip(2) // Skip headers
            .filter(|line| !line.trim().is_empty() && !line.starts_with('-'))
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

    async fn update(&self, _sudo: bool) -> Result<()> {
        info!("Updating scoop");
        self.executor.run("scoop", &["update"], false).await?;
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("Upgrading all scoop packages");
        self.executor.run("scoop", &["update", "*"], false).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("scoop", &["search", query], false)
            .await?;

        let packages = output
            .lines()
            .skip(2) // Skip headers
            .filter(|line| !line.trim().is_empty() && !line.starts_with('-'))
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if !parts.is_empty() {
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

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        info!("Cleaning scoop cache");
        self.executor.run("scoop", &["cleanup", "*"], false).await?;
        self.executor
            .run("scoop", &["cache", "rm", "*"], false)
            .await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        true
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output("scoop", &["info", package], false)
            .await;

        match output {
            Ok(out) => {
                let mut name = None;
                let mut version = None;
                let mut description = None;

                for line in out.lines() {
                    let line = line.trim();
                    if let Some(value) = line.strip_prefix("Name: ") {
                        name = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Version: ") {
                        version = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Description: ") {
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
