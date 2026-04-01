use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct CargoManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl CargoManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for CargoManager {
    fn name(&self) -> &str { "cargo" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("cargo").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        for name in p {
            // run_exclusive handles the cargo registry lock automatically
            self.executor.run_exclusive("cargo", &["install", name], s).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        for name in p {
            self.executor.run_exclusive("cargo", &["uninstall", name], s).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'cargo install --list'
        // Output looks like: "package-name v1.2.3:" followed by paths
        let out = self.executor.run_output("cargo", &["install", "--list"], false).await?;
        Ok(out.lines()
            .filter(|l| !l.starts_with(' ') && l.contains(" v") && l.ends_with(':'))
            .filter_map(|l| {
                let parts: Vec<&str> = l.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some(Package {
                        name: parts[0].to_string(),
                        version: Some(parts[1].trim_start_matches('v').trim_end_matches(':').into()),
                        backend: "cargo".into(),
                        ..Package::new("", "")
                    })
                } else { None }
            }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Cargo CLI search is often slow/deprecated, so we query Crates.io API directly
        let url = format!("https://crates.io/api/v1/crates?q={}&per_page=20", query);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(crates) = json.get("crates").and_then(|c| c.as_array()) {
                return Ok(crates.iter().filter_map(|c| {
                    let name = c.get("name")?.as_str()?;
                    let mut pkg = Package::new(name, "cargo");
                    pkg.description = c.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                    pkg.version = c.get("max_version").and_then(|v| v.as_str()).map(|s| s.to_string());
                    Some(pkg)
                }).collect());
            }
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Query detailed metadata from Crates.io
        let url = format!("https://crates.io/api/v1/crates/{}", package);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(c) = json.get("crate") {
                return Ok(Some(Package {
                    name: package.to_string(),
                    version: c.get("max_version").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    description: c.get("description").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    repository: c.get("repository").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    backend: "cargo".into(),
                    ..Package::new("", "")
                }));
            }
        }
        Ok(None)
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        // REAL LOGIC: Check for 'cargo-install-update' utility, or fallback to re-installing
        if self.executor.command_exists("cargo-install-update").await {
            self.executor.run_exclusive("cargo", &["install-update", "-a"], s).await?;
        } else {
            let installed = self.list_installed().await?;
            for pkg in installed {
                self.executor.run_exclusive("cargo", &["install", &pkg.name], s).await?;
            }
        }
        Ok(())
    }
}