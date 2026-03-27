use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// Poetry Python package manager
pub struct PoetryManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl PoetryManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("poetry")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for PoetryManager {
    fn name(&self) -> &str {
        "poetry"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via poetry", packages.len());
        debug!("Packages: {:?}", packages);

        for package in packages {
            self.executor.run("poetry", &["add", package], sudo).await?;
        }

        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via poetry", packages.len());
        debug!("Packages: {:?}", packages);

        for package in packages {
            self.executor
                .run("poetry", &["remove", package], sudo)
                .await?;
        }

        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor.run_output("poetry", &["show"], false).await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some(Package {
                        name: parts[0].to_string(),
                        version: Some(parts[1].to_string()),
                        backend: self.name().to_string(),
                        description: parts.get(2..).map(|p| p.join(" ")),
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
        info!("Updating poetry lock file");
        self.executor.run("poetry", &["lock"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all poetry packages");
        self.executor.run("poetry", &["update"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // Poetry doesn't have a built-in search command
        // Would need to query PyPI API
        let _ = query;
        Ok(Vec::new())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output("poetry", &["show", package], false)
            .await;

        match output {
            Ok(out) => {
                let mut name = None;
                let mut version = None;
                let mut description = None;

                for line in out.lines() {
                    let line = line.trim();
                    if let Some(value) = line.strip_prefix("name") {
                        name = Some(value.trim().trim_start_matches(':').trim().to_string());
                    } else if let Some(value) = line.strip_prefix("version") {
                        version = Some(value.trim().trim_start_matches(':').trim().to_string());
                    } else if let Some(value) = line.strip_prefix("description") {
                        description = Some(value.trim().trim_start_matches(':').trim().to_string());
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
