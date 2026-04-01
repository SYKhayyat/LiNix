use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PoetryManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl PoetryManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for PoetryManager {
    fn name(&self) -> &str { "poetry" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("poetry").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // REAL LOGIC: 'poetry self add' manages global plugins and dependencies
            self.executor.run("poetry", &["self", "add", pkg, "--non-interactive"], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            self.executor.run("poetry", &["self", "remove", pkg, "--non-interactive"], false).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse the output of 'poetry self show'
        // Format: "package-name (version) Description"
        let out = self.executor.run_output("poetry", &["self", "show"], false).await?;
        Ok(out.lines()
            .filter(|l| !l.is_empty() && !l.starts_with(' '))
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some(Package {
                        name: parts[0].to_string(),
                        version: Some(parts[1].trim_matches(|c| c == '(' || c == ')').to_string()),
                        backend: "poetry".into(),
                        ..Package::new("", "")
                    })
                } else { None }
            }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // Since Poetry self-management is entirely user-driven, installed == manual
        self.list_installed().await
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Query the PyPI search API
        // Poetry plugins usually start with "poetry-" or contain "poetry"
        let url = format!("https://pypi.org/pypi/{}/json", query);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        let mut results = Vec::new();
        // If exact match found
        if res.status().is_success() {
            results.push(Package::new(query, "poetry"));
        }
        
        // Also check common plugin naming conventions
        let plugin_query = if query.starts_with("poetry-") { query.to_string() } else { format!("poetry-{}", query) };
        let url_plugin = format!("https://pypi.org/pypi/{}/json", plugin_query);
        if client.get(url_plugin).send().await?.status().is_success() {
            results.push(Package::new(plugin_query, "poetry"));
        }

        Ok(results)
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Fetch high-quality metadata from the PyPI JSON API
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
                    backend: "poetry".into(),
                    ..Package::new("", "")
                }));
            }
        }
        Ok(None)
    }

    async fn update(&self, _: bool) -> Result<()> {
        // REAL LOGIC: 'self update' upgrades the Poetry tool itself
        self.executor.run("poetry", &["self", "update"], false).await?;
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        // REAL LOGIC: Iteratively update all global plugins
        let installed = self.list_installed().await?;
        for pkg in installed {
            let _ = self.executor.run("poetry", &["self", "add", &format!("{}@latest", pkg.name)], false).await;
        }
        Ok(())
    }
}