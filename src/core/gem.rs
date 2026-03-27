use async_trait::async_trait;
use crate::core::{CommandExecutor, Package, PackageManager, Result};
use once_cell::sync::OnceCell;
use tracing::info;

/// RubyGems package manager
pub struct GemManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl GemManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("gem")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for GemManager {
    fn name(&self) -> &str {
        "gem"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via gem", packages.len());

        let mut args = vec!["install", "--no-document"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("gem", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via gem", packages.len());

        let mut args = vec!["uninstall", "-x"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("gem", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("gem", &["list", "--local"], false)
            .await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with("***") {
                    return None;
                }

                // Format: "name (version, version2, ...)"
                let parts: Vec<&str> = line.splitn(2, '(').collect();
                if parts.len() >= 2 {
                    let name = parts[0].trim().to_string();
                    let version = parts[1]
                        .trim_end_matches(')')
                        .split(',')
                        .next()
                        .map(|v| v.trim().to_string());

                    Some(Package {
                        name,
                        version,
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

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all gem packages");
        self.executor.run("gem", &["update"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self.executor
            .run_output("gem", &["search", query], false)
            .await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with("***") {
                    return None;
                }

                let parts: Vec<&str> = line.splitn(2, '(').collect();
                if parts.len() >= 2 {
                    let name = parts[0].trim().to_string();
                    let version = parts[1]
                        .trim_end_matches(')')
                        .split(',')
                        .next()
                        .map(|v| v.trim().to_string());

                    Some(Package {
                        name,
                        version,
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
        info!("Cleaning old gem versions");
        self.executor.run("gem", &["cleanup"], sudo).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        true
    }
}
