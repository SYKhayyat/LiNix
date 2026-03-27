use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// APK package manager for Alpine Linux
pub struct ApkManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl ApkManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }

    fn check_available(&self) -> bool {
        std::process::Command::new("which")
            .arg("apk")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn parse_name_version(s: &str) -> (String, Option<String>) {
        // APK format: "name-version-rX"
        let parts: Vec<&str> = s.rsplitn(3, '-').collect();

        if parts.len() >= 2 {
            let potential_version = parts[1];
            if potential_version
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                let version = format!("{}-{}", parts[1], parts[0]);
                let name = if parts.len() > 2 {
                    parts[2..].join("-")
                } else {
                    parts[1].to_string()
                };
                return (name, Some(version));
            }
        }

        (s.to_string(), None)
    }
}

#[async_trait]
impl PackageManager for ApkManager {
    fn name(&self) -> &str {
        "apk"
    }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.check_available())
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Installing {} packages via apk", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["add"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("apk", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }

        info!("Removing {} packages via apk", packages.len());
        debug!("Packages: {:?}", packages);

        let mut args = vec!["del"];
        let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
        args.extend(pkg_refs);

        self.executor.run("apk", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("apk", &["list", "--installed"], false)
            .await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if !parts.is_empty() {
                    let (name, version) = Self::parse_name_version(parts[0]);
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

    async fn update(&self, sudo: bool) -> Result<()> {
        info!("Updating apk package cache");
        self.executor.run("apk", &["update"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Upgrading all apk packages");
        self.executor.run("apk", &["upgrade"], sudo).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("apk", &["search", "-v", query], false)
            .await?;

        let packages = output
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(2, " - ").collect();
                if !parts.is_empty() {
                    let (name, version) = Self::parse_name_version(parts[0]);
                    Some(Package {
                        name,
                        version,
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

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let output = self
            .executor
            .run_output("apk", &["info", "-a", package], false)
            .await;

        match output {
            Ok(out) => {
                let mut description = None;
                let mut size = None;

                for line in out.lines() {
                    if let Some(value) = line.strip_prefix("description:") {
                        description = Some(value.trim().to_string());
                    } else if let Some(value) = line.strip_prefix("installed size:") {
                        let parts: Vec<&str> = value.trim().split_whitespace().collect();
                        if let Some(num) = parts.first() {
                            if let Ok(n) = num.parse::<u64>() {
                                size = Some(n);
                            }
                        }
                    }
                }

                Ok(Some(Package {
                    name: package.to_string(),
                    version: None,
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
