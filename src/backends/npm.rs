use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// NPM JavaScript package manager
pub struct NpmManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl NpmManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("npm")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for NpmManager {
    fn name(&self) -> &str {
        "npm"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via npm", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["install", "-g"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("npm", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via npm", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["uninstall", "-g"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("npm", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("npm", &["list", "-g", "--json", "--depth=0"], false)
            .await?;

        let json: serde_json::Value = serde_json::from_str(&output)?;

        let mut packages = Vec::new();
        if let Some(dependencies) = json.get("dependencies").and_then(|d| d.as_object()) {
            for (name, data) in dependencies {
                let version = data
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                packages.push(Package {
                    name: name.clone(),
                    version,
                    backend: self.name().to_string(),
                    description: None,
                    repository: None,
                    size: None,
                });
            }
        }

        Ok(packages)
    }

    async fn update(&self, sudo: bool) -> Result<()> {
        info!("Updating npm itself");
        self.executor
            .run("npm", &["install", "-g", "npm@latest"], sudo)
            .await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all npm packages");
        self.executor.run("npm", &["update", "-g"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("npm", &["search", query, "--json"], false)
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
                let description = pkg
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(|s| s.to_string());

                Some(Package {
                    name,
                    version,
                    backend: self.name().to_string(),
                    description,
                    repository: None,
                    size: None,
                })
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
            .run_output("npm", &["view", package, "--json"], false)
            .await;

        match output {
            Ok(out) => {
                let json: serde_json::Value = serde_json::from_str(&out)?;

                let name = json
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string());
                let version = json
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let description = json
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(|s| s.to_string());

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
