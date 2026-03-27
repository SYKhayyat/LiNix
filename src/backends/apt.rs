use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info, warn};

/// APT package manager for Debian/Ubuntu systems
pub struct AptManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl AptManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("apt")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for AptManager {
    fn name(&self) -> &str {
        "apt"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via apt", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["install", "-y"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("apt", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via apt", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["remove", "-y"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("apt", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("apt", &["list", "--installed"], false)
            .await?;

        let packages = output
            .lines()
            .skip(1) // Skip "Listing..." header
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }

                // Format: "package/release version arch [status]"
                let slash_pos = line.find('/')?;
                let name = line[..slash_pos].to_string();

                // Extract version
                let rest = &line[slash_pos + 1..];
                let version = rest.split_whitespace().nth(1).map(|s| s.to_string());

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

        Ok(packages)
    }

    async fn update(&self, sudo: bool) -> Result<()> {
        info!("Updating apt package cache");
        self.executor.run("apt", &["update"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all apt packages");
        self.executor.run("apt", &["upgrade", "-y"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("apt-cache", &["search", query], false)
            .await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }

                // Format: "package - description"
                let parts: Vec<&str> = line.splitn(2, " - ").collect();
                if !parts.is_empty() {
                    let name = parts[0].trim().to_string();
                    let description = parts.get(1).map(|s| s.trim().to_string());

                    Some(Package {
                        name,
                        version: None,
                        backend: self.name().to_string(),
                        description,
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
        info!("Cleaning orphaned apt packages");
        self.executor
            .run("apt", &["autoremove", "-y"], sudo)
            .await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        true
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output("apt-cache", &["show", package], false)
            .await;

        match output {
            Ok(out) => {
                let mut name = None;
                let mut version = None;
                let mut description = None;
                let mut size = None;

                for line in out.lines() {
                    if let Some(value) = line.strip_prefix("Package: ") {
                        name = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Version: ") {
                        version = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Description: ") {
                        description = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Installed-Size: ") {
                        size = value.trim().parse::<u64>().ok().map(|s| s * 1024);
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
