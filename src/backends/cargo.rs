use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// Cargo Rust package manager
pub struct CargoManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl CargoManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("cargo")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for CargoManager {
    fn name(&self) -> &str {
        "cargo"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via cargo", packages.len());
        debug!("Packages: {:?}", packages);

        for package in packages {
            self.executor
                .run("cargo", &["install", package], sudo)
                .await?;
        }

        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via cargo", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["uninstall"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("cargo", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("cargo", &["install", "--list"], false)
            .await?;

        let mut packages = Vec::new();
        let mut current_package: Option<(String, String)> = None;

        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if !line.starts_with('-') && !line.starts_with(' ') {
                // New package line: "package_name v1.2.3:"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let name = parts[0].to_string();
                    let version = parts[1]
                        .trim_start_matches('v')
                        .trim_end_matches(':')
                        .to_string();

                    if let Some((pkg_name, pkg_version)) = current_package.take() {
                        packages.push(Package {
                            name: pkg_name,
                            version: Some(pkg_version),
                            backend: self.name().to_string(),
                            description: None,
                            repository: None,
                            size: None,
                        });
                    }

                    current_package = Some((name, version));
                }
            }
        }

        if let Some((pkg_name, pkg_version)) = current_package {
            packages.push(Package {
                name: pkg_name,
                version: Some(pkg_version),
                backend: self.name().to_string(),
                description: None,
                repository: None,
                size: None,
            });
        }

        Ok(packages)
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("cargo", &["search", query, "--limit", "20"], false)
            .await?;

        let packages = output
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .filter_map(|line| {
                // Format: name = "version"    # description
                let parts: Vec<&str> = line.splitn(2, '#').collect();
                let name_version: Vec<&str> = parts[0].splitn(2, '=').collect();

                if !name_version.is_empty() {
                    let name = name_version[0].trim().to_string();
                    let version = name_version
                        .get(1)
                        .map(|v| v.trim().trim_matches('"').to_string());
                    let description = parts.get(1).map(|d| d.trim().to_string());

                    Some(Package {
                        name,
                        version,
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

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let installed = self.list_installed().await?;
        Ok(installed.into_iter().find(|p| p.name == package))
    }
}
