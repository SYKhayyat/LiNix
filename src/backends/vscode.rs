use crate::core::{
    CommandExecutor, Package, Result, PackageSpec, 
    BackendCore, Installable, Queryable, Searchable, RateLimiter,
    HealthReport, HealthStatus
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for Visual Studio Code extensions.
pub struct VscodeBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub rate_limiter: RateLimiter,
}

impl VscodeBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "vscode".to_string(),
            rate_limiter: RateLimiter::vscode_marketplace(),
        }
    }

    pub async fn query_marketplace(&self, query: &str) -> Result<serde_json::Value> {
        self.rate_limiter.execute(|| async {
            let client = reqwest::Client::builder()
                .user_agent("linix-manager")
                .build()
                .map_err(crate::core::Error::from)?;

            let body = json!({
                "filters": [{
                    "criteria": [
                        { "filterType": 10, "value": query },
                        { "filterType": 8, "value": "Microsoft.VisualStudio.Code" }
                    ],
                    "pageSize": 20,
                    "pageNumber": 1
                }],
                "flags": 0x21c
            });

            let res = client.post("https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery")
                .header("Accept", "application/json;api-version=3.0-preview.1")
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(crate::core::Error::from)?;

            if !res.status().is_success() {
                return Err(crate::core::Error::Other(format!("Marketplace API error: {}", res.status())));
            }

            res.json().await.map_err(crate::core::Error::from)
        }).await
    }
}

#[async_trait]
impl BackendCore for VscodeBackendCore {
    fn name(&self) -> &str { &self.name }
    fn is_available(&self) -> bool { self.executor.command_exists_sync("code") }
    async fn check_health(&self) -> Result<HealthReport> {
        Ok(HealthReport { status: HealthStatus::Ok, message: None })
    }
}

pub struct VscodeInstallable {
    pub core: Arc<VscodeBackendCore>,
}

#[async_trait]
impl Installable for VscodeInstallable {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            info!("VSCode: Installing extension '{}'...", spec.name);
            self.core.executor.run("code", &["--install-extension", &spec.name, "--force"], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
            info!("VSCode: Uninstalling extension '{}'...", name);
            self.core.executor.run("code", &["--uninstall-extension", name], false).await?;
        }
        Ok(())
    }
}

pub struct VscodeQueryable {
    pub core: Arc<VscodeBackendCore>,
}

#[async_trait]
impl Queryable for VscodeQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.core.executor.run_output("code", &["--list-extensions", "--show-versions"], false).await?;
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
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let json = self.core.query_marketplace(name).await?;
        
        if let Some(ext) = json["results"][0]["extensions"].as_array().and_then(|a| a.first()) {
            let publisher = ext["publisher"]["publisherName"].as_str().unwrap_or("unknown");
            let ext_name = ext["extensionName"].as_str().unwrap_or("unknown");
            
            let mut p = Package::new(format!("{}.{}", publisher, ext_name), "vscode");
            p.version = ext["versions"][0]["version"].as_str().map(|s| s.to_string());
            
            if let Some(desc) = ext["shortDescription"].as_str() {
                p.properties.insert("description".into(), desc.to_string());
            }
            
            p.properties.insert("publisher".into(), publisher.to_string());
            return Ok(Some(p));
        }
        Ok(None)
    }
}

pub struct VscodeSearchable {
    pub core: Arc<VscodeBackendCore>,
}

#[async_trait]
impl Searchable for VscodeSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let json = self.core.query_marketplace(query).await?;
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