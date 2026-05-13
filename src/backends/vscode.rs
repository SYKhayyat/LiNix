use crate::core::{CommandExecutor, Package, Result, PackageSpec, Backend, Installable, Queryable, Searchable};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde_json::json;
use std::collections::HashMap;
use tracing::{debug, info};

/// Specialized manager for Visual Studio Code extensions.
/// Communicates with the local 'code' binary for operations and the 
/// official VS Code Marketplace API for search and metadata discovery.
pub struct VscodeManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl VscodeManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self { 
            executor, 
            available: OnceCell::new() 
        }
    }

    /// Internal helper to query the official VS Code Marketplace API.
    /// This is used to implement high-performance remote search and rich info metadata.
    async fn query_marketplace(&self, query: &str) -> Result<serde_json::Value> {
        let client = reqwest::Client::builder()
            .user_agent("linix-manager")
            .build()?;

        // The Marketplace API uses a specific POST structure for extension queries.
        // filterType 10 = Search Text
        // filterType 8  = Target Platform (VS Code)
        let body = json!({
            "filters": [{
                "criteria": [
                    { "filterType": 10, "value": query },
                    { "filterType": 8, "value": "Microsoft.VisualStudio.Code" }
                ],
                "pageSize": 20,
                "pageNumber": 1
            }],
            "flags": 0x21c // Flags for metadata, versions, and stats
        });

        let res = client.post("https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery")
            .header("Accept", "application/json;api-version=3.0-preview.1")
            .header("Content-Type", "application/json")
            .json(&body)
            .send().await?;

        if !res.status().is_success() {
            return Err(crate::core::Error::Other(format!("Marketplace API error: {}", res.status())));
        }

        Ok(res.json().await?)
    }
}

impl Backend for VscodeManager {
    fn name(&self) -> &str { "vscode" }
    
    fn is_available(&self) -> bool {
        // VS Code CLI can be named 'code' or 'code-insiders'
        *self.available.get_or_init(|| self.executor.command_exists_sync("code"))
    }

    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
    fn as_searchable(&self) -> Option<&dyn Searchable> { Some(self) }
}

#[async_trait]
impl Installable for VscodeManager {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            info!("VSCode: Installing extension '{}'...", spec.name);
            // --force ensures the extension is updated if already present.
            // Extensions are installed to the user's home dir, so sudo is false.
            self.executor.run("code", &["--install-extension", &spec.name, "--force"], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
            info!("VSCode: Uninstalling extension '{}'...", name);
            self.executor.run("code", &["--uninstall-extension", name], false).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Queryable for VscodeManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("code", &["--list-extensions", "--show-versions"], false).await?;
        let mut extensions = Vec::new();

        for line in out.lines() {
            if let Some((name, version)) = line.split_once('@') {
                extensions.push(Package::with_version(name.trim(), version.trim(), "vscode"));
            } else if !line.trim().is_empty() {
                extensions.push(Package::new(line.trim(), "vscode"));
            }
        }
        Ok(extensions)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // VS Code CLI doesn't distinguish between manual and auto-installed dependencies.
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let json = self.query_marketplace(name).await?;
        
        // Traverse the Marketplace response structure
        if let Some(ext) = json["results"][0]["extensions"].as_array().and_then(|a| a.first()) {
            let publisher = ext["publisher"]["publisherName"].as_str().unwrap_or("unknown");
            let ext_name = ext["extensionName"].as_str().unwrap_or("unknown");
            
            let mut p = Package::new(format!("{}.{}", publisher, ext_name), "vscode");
            p.version = ext["versions"][0]["version"].as_str().map(|s| s.to_string());
            
            if let Some(desc) = ext["shortDescription"].as_str() {
                p.properties.insert("description".into(), desc.to_string());
            }
            
            p.properties.insert("publisher".into(), publisher.to_string());
            p.properties.insert("url".into(), format!(
                "https://marketplace.visualstudio.com/items?itemName={}.{}", 
                publisher, ext_name
            ));
            
            return Ok(Some(p));
        }
        Ok(None)
    }
}

#[async_trait]
impl Searchable for VscodeManager {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let json = self.query_marketplace(query).await?;
        let mut results = Vec::new();

        if let Some(extensions) = json["results"][0]["extensions"].as_array() {
            for ext in extensions {
                let publisher = ext["publisher"]["publisherName"].as_str().unwrap_or("");
                let name = ext["extensionName"].as_str().unwrap_or("");
                
                let mut p = Package::new(format!("{}.{}", publisher, name), "vscode");
                if let Some(desc) = ext["shortDescription"].as_str() {
                    p.properties.insert("description".into(), desc.to_string());
                }
                results.push(p);
            }
        }
        Ok(results)
    }
}