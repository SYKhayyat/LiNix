use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// PIP Python package manager
pub struct PipManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl PipManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("pip")
            .output()
            .map(|o| o.status.success())
            .unwrap_or_else(|_| {
                std::process::Command::new("which")
                    .arg("pip3")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            })
    }

    fn pip_cmd(&self) -> &str {
        "pip3"
    }
}

#[async_trait]
impl PackageManager for PipManager {
    fn name(&self) -> &str {
        "pip"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via pip", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["install"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run(self.pip_cmd(), &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via pip", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["uninstall", "-y"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run(self.pip_cmd(), &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output(self.pip_cmd(), &["list", "--format=json"], false)
            .await?;

        let packages_json: Vec<serde_json::Value> =
            serde_json::from_str(&output).unwrap_or_default();

        let packages = packages_json
            .into_iter()
            .filter_map(|pkg| {
                let name = pkg.get("name")?.as_str()?.to_string();
                let version = pkg
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

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
        info!("Upgrading pip itself");
        self.executor
            .run(self.pip_cmd(), &["install", "--upgrade", "pip"], sudo)
            .await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all pip packages");

        let output = self
            .executor
            .run_output(
                self.pip_cmd(),
                &["list", "--outdated", "--format=json"],
                false,
            )
            .await?;

        let packages: Vec<serde_json::Value> = serde_json::from_str(&output).unwrap_or_default();

        for pkg in packages {
            if let Some(name) = pkg.get("name").and_then(|n| n.as_str()) {
                let _ = self
                    .executor
                    .run(self.pip_cmd(), &["install", "--upgrade", name], sudo)
                    .await;
            }
        }

        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // pip search is disabled on PyPI, return empty
        let _ = query;
        Ok(Vec::new())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output(self.pip_cmd(), &["show", package], false)
            .await;

        match output {
            Ok(out) => {
                let mut name = None;
                let mut version = None;
                let mut description = None;

                for line in out.lines() {
                    if let Some(value) = line.strip_prefix("Name: ") {
                        name = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Version: ") {
                        version = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("Summary: ") {
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
