use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct NpmManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    settings: Option<HashMap<String, String>>,
}

impl NpmManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
}

#[async_trait]
impl PackageManager for NpmManager {
    fn name(&self) -> &str { "npm" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            // Checks if node and npm are available in the system path
            std::process::Command::new("npm").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["install", "-g"];
        if let Some(reg) = self.settings.as_ref().and_then(|s| s.get("registry")) {
            args.extend(["--registry", reg]);
        }
        args.extend(p.iter().map(|x| x.as_str()));
        // Use run_exclusive to prevent NPM cache lock errors during parallel syncs
        self.executor.run_exclusive("npm", &args, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["uninstall", "-g"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run_exclusive("npm", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse the JSON output of npm list at depth 0
        let out = self.executor.run_output("npm", &["list", "-g", "--depth=0", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut res = vec![];
        
        if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
            for (name, val) in deps {
                let version = val.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                res.push(Package { 
                    name: name.clone(), 
                    version, 
                    backend: "npm".into(), 
                    ..Package::new("", "") 
                });
            }
        }
        Ok(res)
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Query the NPM Registry API directly (instant vs 10s CLI search)
        let url = format!("https://registry.npmjs.org/-/v1/search?text={}&size=20", query);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(objects) = json.get("objects").and_then(|o| o.as_array()) {
                return Ok(objects.iter().filter_map(|obj| {
                    let pkg = obj.get("package")?;
                    let name = pkg.get("name")?.as_str()?;
                    let mut p = Package::new(name, "npm");
                    p.description = pkg.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                    p.version = pkg.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                    Some(p)
                }).collect());
            }
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Fetch full package details from the registry API
        let url = format!("https://registry.npmjs.org/{}", package);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            return Ok(Some(Package {
                name: package.to_string(),
                version: json.get("dist-tags").and_then(|t| t.get("latest")).and_then(|v| v.as_str()).map(|s| s.to_string()),
                description: json.get("description").and_then(|v| v.as_str()).map(|s| s.to_string()),
                repository: json.get("homepage").and_then(|v| v.as_str()).map(|s| s.to_string()),
                backend: "npm".into(),
                ..Package::new("", "")
            }));
        }
        Ok(None)
    }

    async fn update(&self, s: bool) -> Result<()> {
        // Upgrades the NPM package manager itself
        self.executor.run("npm", &["install", "-g", "npm@latest"], s).await?;
        Ok(())
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        // REAL LOGIC: Performs a global update of all managed packages
        self.executor.run("npm", &["update", "-g"], s).await?;
        Ok(())
    }
}