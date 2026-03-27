use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// Zypper package manager for openSUSE
pub struct ZypperManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl ZypperManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("zypper")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for ZypperManager {
    fn name(&self) -> &str {
        "zypper"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via zypper", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["install", "-y", "--no-confirm"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("zypper", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via zypper", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["remove", "-y", "--no-confirm"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("zypper", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("zypper", &["search", "--installed-only", "-s"], false)
            .await?;

        let packages = output
            .lines()
            .skip(4) // Skip headers
            .filter(|line| line.starts_with("i") || line.starts_with("i+"))
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 4 {
                    Some(Package {
                        name: parts[1].trim().to_string(),
                        version: Some(parts[3].trim().to_string()),
                        backend: self.name().to_string(),
                        description: None,
                        repository: parts.get(5).map(|s| s.trim().to_string()),
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
        info!("Updating zypper package cache");
        self.executor.run("zypper", &["refresh"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all zypper packages");
        self.executor.run("zypper", &["update", "-y"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("zypper", &["search", "-s", query], false)
            .await?;

        let packages = output
            .lines()
            .skip(4) // Skip headers
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 4 {
                    Some(Package {
                        name: parts[1].trim().to_string(),
                        version: Some(parts[3].trim().to_string()),
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

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        info!("Cleaning unused zypper packages");
        // Zypper doesn't have a direct autoremove, but we can try
        let _ = self
            .executor
            .run("zypper", &["packages", "--orphaned"], sudo)
            .await;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output("zypper", &["info", package], false)
            .await;

        match output {
            Ok(out) => {
                let mut name = None;
                let mut version = None;
                let mut description = None;

                for line in out.lines() {
                    if let Some(value) = line.strip_prefix("Name        : ") {
                        name = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Version     : ") {
                        version = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Description : ") {
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
