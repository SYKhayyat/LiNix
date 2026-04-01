use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct ComposerManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl ComposerManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }

    fn get_global_config_path() -> PathBuf {
        // Standard Composer global config locations
        let home = dirs::home_dir().unwrap_or_default();
        let xdg_config = home.join(".config").join("composer").join("composer.json");
        if xdg_config.exists() { return xdg_config; }
        home.join(".composer").join("composer.json")
    }
}

#[async_trait]
impl PackageManager for ComposerManager {
    fn name(&self) -> &str { "composer" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("composer").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["global", "require", "--no-interaction"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("composer", &args, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["global", "remove", "--no-interaction"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("composer", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse the JSON output of composer global show
        let out = self.executor.run_output("composer", &["global", "show", "--format=json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut pkgs = vec![];
        
        if let Some(installed) = json.get("installed").and_then(|i| i.as_array()) {
            for pkg in installed {
                if let Some(name) = pkg.get("name").and_then(|n| n.as_str()) {
                    pkgs.push(Package {
                        name: name.to_string(),
                        version: pkg.get("version").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        backend: "composer".into(),
                        ..Package::new("", "")
                    });
                }
            }
        }
        Ok(pkgs)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // NUCLEAR DELETE FIX: Parse the actual composer.json to see what the user explicitly required
        let path = Self::get_global_config_path();
        if !path.exists() { return Ok(vec![]); }
        
        let content = tokio::fs::read_to_string(path).await.unwrap_or_default();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
        
        let mut manual = vec![];
        if let Some(reqs) = json.get("require").and_then(|r| r.as_object()) {
            for name in reqs.keys() {
                if name.contains('/') { // Composer packages must have a vendor/name format
                    manual.push(Package::new(name, "composer"));
                }
            }
        }
        Ok(manual)
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Query Packagist API (much faster than CLI search)
        let url = format!("https://packagist.org/search.json?q={}", query);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(results) = json.get("results").and_then(|r| r.as_array()) {
                return Ok(results.iter().filter_map(|r| {
                    let name = r.get("name")?.as_str()?;
                    let mut p = Package::new(name, "composer");
                    p.description = r.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                    Some(p)
                }).collect());
            }
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Query Packagist for specific package metadata
        let url = format!("https://packagist.org/packages/{}.json", package);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(p) = json.get("package") {
                return Ok(Some(Package {
                    name: package.to_string(),
                    description: p.get("description").and_then(|d| d.as_str()).map(|s| s.to_string()),
                    repository: p.get("repository").and_then(|r| r.as_str()).map(|s| s.to_string()),
                    backend: "composer".into(),
                    ..Package::new("", "")
                }));
            }
        }
        Ok(None)
    }

    async fn update(&self, s: bool) -> Result<()> {
        // Upgrade Composer itself
        self.executor.run("composer", &["self-update"], s).await?;
        Ok(())
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        // Upgrade all global packages
        self.executor.run("composer", &["global", "update", "--no-interaction"], s).await?;
        Ok(())
    }
}