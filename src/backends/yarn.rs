use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// Yarn JavaScript package manager
pub struct YarnManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl YarnManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("yarn")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for YarnManager {
    fn name(&self) -> &str {
        "yarn"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via yarn", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["global", "add"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("yarn", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via yarn", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["global", "remove"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("yarn", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("yarn", &["global", "list", "--depth=0"], false)
            .await?;

        let packages = output
            .lines()
            .filter(|line| line.contains("@"))
            .filter_map(|line| {
                let line = line
                    .trim()
                    .trim_start_matches("├── ")
                    .trim_start_matches("└── ");
                let parts: Vec<&str> = line.rsplitn(2, '@').collect();
                if parts.len() == 2 {
                    Some(Package {
                        name: parts[1].to_string(),
                        version: Some(parts[0].to_string()),
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
        info!("Upgrading all yarn packages");
        self.executor
            .run("yarn", &["global", "upgrade"], sudo)
            .await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }
}
