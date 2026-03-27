use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info, warn};

/// DNF package manager for Fedora/RHEL
pub struct DnfManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl DnfManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("dnf")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for DnfManager {
    fn name(&self) -> &str {
        "dnf"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via dnf", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["install", "-y"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("dnf", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via dnf", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["remove", "-y"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("dnf", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("dnf", &["list", "installed"], false)
            .await?;

        let packages = output
            .lines()
            .skip(1) // Skip header
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    // Format: "package.arch version repo"
                    let name = parts[0].split('.').next()?;
                    Some(Package {
                        name: name.to_string(),
                        version: Some(parts[1].to_string()),
                        backend: self.name().to_string(),
                        description: None,
                        repository: parts.get(2).map(|s| s.to_string()),
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
        info!("Updating dnf package cache");
        // check-update returns exit code 100 if updates are available
        let _ = self.executor.run("dnf", &["check-update"], sudo).await;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all dnf packages");
        self.executor.run("dnf", &["upgrade", "-y"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("dnf", &["search", query], false)
            .await?;

        let packages = output
            .lines()
            .filter(|line| {
                !line.starts_with("=") && !line.is_empty() && !line.starts_with("Last metadata")
            })
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(2, " : ").collect();
                if !parts.is_empty() {
                    let name = parts[0].split('.').next()?.trim().to_string();
                    Some(Package {
                        name,
                        version: None,
                        backend: self.name().to_string(),
                        description: parts.get(1).map(|s| s.to_string()),
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
        info!("Cleaning orphaned dnf packages");
        self.executor
            .run("dnf", &["autoremove", "-y"], sudo)
            .await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        true
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output("dnf", &["info", package], false)
            .await;

        match output {
            Ok(out) => {
                let mut name = None;
                let mut version = None;
                let mut description = None;
                let mut size = None;

                for line in out.lines() {
                    if let Some(value) = line.strip_prefix("Name         : ") {
                        name = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Version      : ") {
                        version = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Description  : ") {
                        description = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Size         : ") {
                        let parts: Vec<&str> = value.trim().split_whitespace().collect();
                        if let Some(num) = parts.first() {
                            if let Ok(n) = num.parse::<f64>() {
                                let multiplier = match parts.get(1) {
                                    Some(&"k") => 1024.0,
                                    Some(&"M") => 1024.0 * 1024.0,
                                    Some(&"G") => 1024.0 * 1024.0 * 1024.0,
                                    _ => 1.0,
                                };
                                size = Some((n * multiplier) as u64);
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
