use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PipManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    settings: Option<HashMap<String, String>>,
}

impl PipManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }

    /// REAL LOGIC: Detects if the system uses 'pip3' or 'pip'
    async fn get_cmd(&self) -> &str {
        if self.executor.command_exists("pip3").await { "pip3" } else { "pip" }
    }
}

#[async_trait]
impl PackageManager for PipManager {
    fn name(&self) -> &str { "pip" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("pip").arg("--version").output().is_ok() ||
            std::process::Command::new("pip3").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let cmd = self.get_cmd().await;
        let mut args = vec!["install".to_string(), "--upgrade".into(), "--no-input".into()];
        
        if let Some(set) = &self.settings {
            if let Some(url) = set.get("index_url") { 
                args.extend(["--index-url".into(), url.clone()]); 
            }
        }
        
        args.extend(p.iter().cloned());
        let refs: Vec<&str> = args.iter().map(|x| x.as_str()).collect();
        self.executor.run(cmd, &refs, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let cmd = self.get_cmd().await;
        let mut args = vec!["uninstall", "-y"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run(cmd, &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse the JSON output of pip list
        let cmd = self.get_cmd().await;
        let out = self.executor.run_output(cmd, &["list", "--format=json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        
        Ok(json.as_array().unwrap_or(&vec![]).iter().filter_map(|p| {
            let name = p.get("name")?.as_str()?.to_string();
            let version = p.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
            Some(Package { name, version, backend: "pip".to_string(), ..Package::new("", "") })
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // NUCLEAR DELETE FIX: Use --not-required to find packages that aren't dependencies 
        // of other packages. This prevents LiNix from trying to delete Python's core libs.
        let cmd = self.get_cmd().await;
        let out = self.executor.run_output(cmd, &["list", "--not-required", "--format=json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        
        Ok(json.as_array().unwrap_or(&vec![]).iter().filter_map(|p| {
            let name = p.get("name")?.as_str()?.to_string();
            Some(Package::new(name, "pip"))
        }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: 'pip search' is broken/disabled on PyPI. 
        // We use a simple query to the PyPI autocomplete API instead.
        let url = format!("https://pypi.org/pypi/{}/json", query);
        let client = reqwest::Client::new();
        let res = client.get(url).send().await?;
        
        // Since PyPI doesn't have a clean "search" JSON API, we check if the exact name exists
        if res.status().is_success() {
            return Ok(vec![Package::new(query, "pip")]);
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Fetch rich metadata from the PyPI JSON API
        let url = format!("https://pypi.org/pypi/{}/json", package);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(info) = json.get("info") {
                return Ok(Some(Package {
                    name: package.to_string(),
                    version: info.get("version").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    description: info.get("summary").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    repository: info.get("home_page").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    backend: "pip".into(),
                    ..Package::new("", "")
                }));
            }
        }
        Ok(None)
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        // REAL LOGIC: PIP upgrade is done by running install --upgrade on all manual packages
        let manual = self.list_manual().await?;
        let names: Vec<String> = manual.into_iter().map(|p| p.name).collect();
        self.install(&names, s).await
    }
}