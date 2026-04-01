use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PipxManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl PipxManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for PipxManager {
    fn name(&self) -> &str { "pipx" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("pipx").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // REAL LOGIC: pipx creates isolated virtualenvs for each app
            self.executor.run("pipx", &["install", pkg], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            self.executor.run("pipx", &["uninstall", pkg], false).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse the detailed JSON output of pipx list
        let out = self.executor.run_output("pipx", &["list", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut pkgs = vec![];

        if let Some(venvs) = json.get("venvs").and_then(|v| v.as_object()) {
            for (name, data) in venvs {
                let version = data.get("metadata")
                    .and_then(|m| m.get("main_package"))
                    .and_then(|p| p.get("version"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                pkgs.push(Package {
                    name: name.clone(),
                    version,
                    backend: "pipx".into(),
                    ..Package::new("", "")
                });
            }
        }
        Ok(pkgs)
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: pipx installs from PyPI, so we query PyPI's JSON API to find packages
        let url = format!("https://pypi.org/pypi/{}/json", query);
        let client = reqwest::Client::new();
        let res = client.get(url).send().await?;
        
        if res.status().is_success() {
            return Ok(vec![Package::new(query, "pipx")]);
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Fetch package description and homepage from PyPI
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
                    backend: "pipx".into(),
                    ..Package::new("", "")
                }));
            }
        }
        Ok(None)
    }

    async fn update(&self, _: bool) -> Result<()> {
        // pipx doesn't have a local index to refresh
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        // REAL LOGIC: Efficiently upgrade all pipx-managed applications
        self.executor.run("pipx", &["upgrade-all"], false).await?;
        Ok(())
    }
}