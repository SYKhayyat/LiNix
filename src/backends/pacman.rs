use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info, warn};

/// Pacman package manager for Arch Linux
pub struct PacmanManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl PacmanManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("pacman")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for PacmanManager {
    fn name(&self) -> &str {
        "pacman"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via pacman", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["-S", "--noconfirm", "--needed"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("pacman", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via pacman", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["-Rs", "--noconfirm"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("pacman", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor.run_output("pacman", &["-Q"], false).await?;

        let packages = output
            .lines()
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
        info!("Updating pacman package database");
        self.executor.run("pacman", &["-Sy"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all pacman packages");
        self.executor
            .run("pacman", &["-Syu", "--noconfirm"], sudo)
            .await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("pacman", &["-Ss", query], false)
            .await?;

        let mut packages = Vec::new();
        let mut lines = output.lines().peekable();

        while let Some(line) = lines.next() {
            if line.starts_with(' ') {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let name_part = parts[0];
                let name = name_part.split('/').last().unwrap_or(name_part);

                let description = lines
                    .peek()
                    .filter(|l| l.starts_with("    "))
                    .map(|l| l.trim().to_string());

                if description.is_some() {
                    lines.next();
                }

                packages.push(Package {
                    name: name.to_string(),
                    version: Some(parts[1].to_string()),
                    backend: self.name().to_string(),
                    description,
                    repository: None,
                    size: None,
                });
            }
        }

        Ok(packages)
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        info!("Cleaning orphaned pacman packages");

        let output = self.executor.run_output("pacman", &["-Qdtq"], false).await;

        if let Ok(orphans) = output {
            let orphan_list: Vec<&str> = orphans.lines().filter(|l| !l.is_empty()).collect();
            if !orphan_list.is_empty() {
                let mut args = vec!["-Rs", "--noconfirm"];
                args.extend(orphan_list);
                self.executor.run("pacman", &args, sudo).await?;
            }
        }

        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        true
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output("pacman", &["-Qi", package], false)
            .await;

        match output {
            Ok(out) => {
                let mut name = None;
                let mut version = None;
                let mut description = None;
                let mut size = None;

                for line in out.lines() {
                    if let Some(value) = line.strip_prefix("Name            : ") {
                        name = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Version         : ") {
                        version = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Description     : ") {
                        description = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Installed Size  : ") {
                        // Parse size like "10.5 MiB"
                        let parts: Vec<&str> = value.trim().split_whitespace().collect();
                        if let Some(num) = parts.first() {
                            if let Ok(n) = num.parse::<f64>() {
                                let multiplier = match parts.get(1) {
                                    Some(&"KiB") => 1024.0,
                                    Some(&"MiB") => 1024.0 * 1024.0,
                                    Some(&"GiB") => 1024.0 * 1024.0 * 1024.0,
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
