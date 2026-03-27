use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// Homebrew package manager (macOS/Linux)
pub struct BrewManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl BrewManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("brew")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for BrewManager {
    fn name(&self) -> &str {
        "brew"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via brew", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["install"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("brew", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via brew", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["uninstall"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("brew", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("brew", &["list", "--versions"], false)
            .await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if !parts.is_empty() {
                    Some(Package {
                        name: parts[0].to_string(),
                        version: parts.get(1..).map(|p| p.join(" ")),
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
        info!("Updating brew package cache");
        self.executor.run("brew", &["update"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all brew packages");
        self.executor.run("brew", &["upgrade"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("brew", &["search", query], false)
            .await?;

        let packages = output
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with("="))
            .map(|line| Package {
                name: line.trim().to_string(),
                version: None,
                backend: self.name().to_string(),
                description: None,
                repository: None,
                size: None,
            })
            .collect();

        Ok(packages)
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        info!("Cleaning unused brew dependencies");
        self.executor.run("brew", &["autoremove"], sudo).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        true
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output("brew", &["info", package], false)
            .await;

        match output {
            Ok(out) => {
                let lines: Vec<&str> = out.lines().collect();
                if lines.is_empty() {
                    return Ok(None);
                }

                // First line format: "name: version"
                let first_line = lines[0];
                let parts: Vec<&str> = first_line.splitn(2, ':').collect();
                let name = parts[0].trim().to_string();
                let version = parts
                    .get(1)
                    .map(|s| s.trim().split_whitespace().next().unwrap_or("").to_string());
                let description = lines.get(1).map(|s| s.trim().to_string());

                Ok(Some(Package {
                    name,
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
