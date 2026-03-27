use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::path::PathBuf;
use tracing::{debug, info};

/// Go package manager
pub struct GoManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl GoManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("go")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn get_gobin_path() -> PathBuf {
        if let Ok(gobin) = std::env::var("GOBIN") {
            PathBuf::from(gobin)
        } else if let Ok(gopath) = std::env::var("GOPATH") {
            PathBuf::from(gopath).join("bin")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join("go").join("bin")
        } else {
            PathBuf::from("/usr/local/go/bin")
        }
    }
}

#[async_trait]
impl PackageManager for GoManager {
    fn name(&self) -> &str {
        "go"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via go", packages.len());
        debug!("Packages: {:?}", packages);

        for package in packages {
            let pkg_with_version = if package.contains('@') {
                package.clone()
            } else {
                format!("{}@latest", package)
            };

            self.executor
                .run("go", &["install", &pkg_with_version], sudo)
                .await?;
        }

        Ok(())
    }

    async fn remove(&self, packages: &[String], _sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via go", packages.len());
        debug!("Packages: {:?}", packages);

        let gobin = Self::get_gobin_path();

        for package in packages {
            let binary_name = package.split('/').last().unwrap_or(package);
            let binary_path = gobin.join(binary_name);

            if binary_path.exists() {
                std::fs::remove_file(&binary_path).ok();
            }
        }

        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let gobin = Self::get_gobin_path();

        if !gobin.exists() {
            return Ok(Vec::new());
        }

        let packages = std::fs::read_dir(&gobin)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();

                if path.is_file() {
                    let name = path.file_name()?.to_string_lossy().to_string();
                    Some(Package {
                        name,
                        version: None,
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

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("Go packages require reinstalling with @latest to upgrade");
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }
}
