use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct UvManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl UvManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for UvManager {
    fn name(&self) -> &str { "uv" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            // Checks if the 'uv' binary is in the system PATH
            std::process::Command::new("uv").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // REAL LOGIC: 'uv tool install' manages isolated Python applications
            self.executor.run("uv", &["tool", "install", pkg], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // Uninstalls the tool and its isolated environment
            self.executor.run("uv", &["tool", "uninstall", pkg], false).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'uv tool list'
        // Output looks like: "ruff v0.1.0" or "black v23.1.0"
        let out = self.executor.run_output("uv", &["tool", "list"], false).await?;
        Ok(out.lines()
            .filter(|l| !l.is_empty() && l.contains('v'))
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some(Package {
                        name: parts[0].to_string(),
                        version: Some(parts[1].trim_start_matches('v').to_string()),
                        backend: "uv".into(),
                        ..Package::new("", "")
                    })
                } else { None }
            }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // Since 'uv tool' is exclusively for user-installed tools, installed == manual
        self.list_installed().await
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: 'uv' doesn't have a search command, so we query the PyPI API directly
        let url = format!("https://pypi.org/pypi/{}/json", query);
        let client = reqwest::Client::new();
        let res = client.get(url).send().await?;
        
        // Check for exact match on PyPI
        if res.status().is_success() {
            return Ok(vec![Package::new(query, "uv")]);
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
                    repository: info.get("project_url").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    backend: "uv".into(),
                    ..Package::new("", "")
                }));
            }
        }
        Ok(None)
    }

    async fn update(&self, _: bool) -> Result<()> {
        // Self-upgrade uv if managed by a shell script/installer
        let _ = self.executor.run("uv", &["self", "update"], false).await;
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        // REAL LOGIC: Efficiently upgrade all tools managed by uv
        self.executor.run("uv", &["tool", "upgrade", "--all"], false).await?;
        Ok(())
    }
}