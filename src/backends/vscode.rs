use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde_json::json;
use std::collections::HashMap;

pub struct VscodeManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl VscodeManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }

    /// Helper: Queries the VS Code Marketplace API for extension metadata
    async fn query_marketplace(&self, query: &str) -> Result<serde_json::Value> {
        let client = reqwest::Client::new();
        // Marketplace API requires a specific POST structure
        let body = json!({
            "filters": [{
                "criteria": [
                    { "filterType": 10, "value": query }, // Search text
                    { "filterType": 8, "value": "Microsoft.VisualStudio.Code" } // Target VS Code
                ],
                "pageSize": 20
            }],
            "flags": 0x21c // Flags to include metadata and latest version info
        });

        let res = client.post("https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery")
            .header("Accept", "application/json;api-version=3.0-preview.1")
            .header("Content-Type", "application/json")
            .header("User-Agent", "linix-manager")
            .json(&body)
            .send().await?;

        Ok(res.json().await?)
    }
}

#[async_trait]
impl PackageManager for VscodeManager {
    fn name(&self) -> &str { "vscode" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            // Checks for the 'code' binary (VS Code CLI)
            std::process::Command::new("code").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // --force ensures it updates if already installed
            self.executor.run("code", &["--install-extension", pkg, "--force"], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            self.executor.run("code", &["--uninstall-extension", pkg], false).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'code --list-extensions --show-versions'
        // Format: "publisher.extension-name@1.2.3"
        let out = self.executor.run_output("code", &["--list-extensions", "--show-versions"], false).await?;
        Ok(out.lines()
            .filter(|l| !l.is_empty() && l.contains('@'))
            .filter_map(|line| {
                let (name, ver) = line.split_once('@')?;
                Some(Package {
                    name: name.trim().to_string(),
                    version: Some(ver.trim().to_string()),
                    backend: "vscode".into(),
                    ..Package::new("", "")
                })
            }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Hit the Visual Studio Marketplace API
        let json = self.query_marketplace(query).await?;
        let mut results = Vec::new();

        if let Some(extensions) = json["results"][0]["extensions"].as_array() {
            for ext in extensions {
                let publisher = ext["publisher"]["publisherName"].as_str().unwrap_or("");
                let name = ext["extensionName"].as_str().unwrap_or("");
                let mut p = Package::new(format!("{}.{}", publisher, name), "vscode");
                p.description = ext["shortDescription"].as_str().map(|s| s.to_string());
                results.push(p);
            }
        }
        Ok(results)
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Use the Marketplace query logic for exact package match
        let json = self.query_marketplace(package).await?;
        
        if let Some(ext) = json["results"][0]["extensions"].as_array().and_then(|a| a.first()) {
            let publisher = ext["publisher"]["publisherName"].as_str().unwrap_or("");
            let name = ext["extensionName"].as_str().unwrap_or("");
            return Ok(Some(Package {
                name: format!("{}.{}", publisher, name),
                version: ext["versions"][0]["version"].as_str().map(|s| s.to_string()),
                description: ext["shortDescription"].as_str().map(|s| s.to_string()),
                repository: Some(format!("https://marketplace.visualstudio.com/items?itemName={}.{}", publisher, name)),
                backend: "vscode".into(),
                ..Package::new("", "")
            }));
        }
        Ok(None)
    }

    async fn update(&self, _: bool) -> Result<()> {
        // VS Code handles its own binary updates; no CLI action needed
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        // REAL LOGIC: VS Code CLI doesn't have "upgrade-all". 
        // We re-install every extension using the --force flag to trigger an update.
        let installed = self.list_installed().await?;
        let names: Vec<String> = installed.into_iter().map(|p| p.name).collect();
        self.install(&names, false).await
    }
}