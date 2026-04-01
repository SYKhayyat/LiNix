use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PnpmManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl PnpmManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for PnpmManager {
    fn name(&self) -> &str { "pnpm" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("pnpm").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["add", "-g"];
        args.extend(p.iter().map(|x| x.as_str()));
        // Use run_exclusive to prevent pnpm store lock contention
        self.executor.run_exclusive("pnpm", &args, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["remove", "-g"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run_exclusive("pnpm", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse pnpm's global JSON output
        let out = self.executor.run_output("pnpm", &["list", "-g", "--depth=0", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut pkgs = vec![];

        // pnpm returns an array where the first element contains dependencies
        if let Some(entries) = json.as_array() {
            if let Some(first) = entries.first() {
                if let Some(deps) = first.get("dependencies").and_then(|d| d.as_object()) {
                    for (name, data) in deps {
                        let ver = data.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                        pkgs.push(Package { 
                            name: name.clone(), 
                            version: ver, 
                            backend: "pnpm".into(), 
                            ..Package::new("", "") 
                        });
                    }
                }
            }
        }
        Ok(pkgs)
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Query the NPM Registry API directly (pnpm uses the same registry)
        let url = format!("https://registry.npmjs.org/-/v1/search?text={}&size=20", query);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(objects) = json.get("objects").and_then(|o| o.as_array()) {
                return Ok(objects.iter().filter_map(|obj| {
                    let pkg = obj.get("package")?;
                    let name = pkg.get("name")?.as_str()?;
                    Some(Package::new(name, "pnpm"))
                }).collect());
            }
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Fetch high-quality metadata from the registry
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
                backend: "pnpm".into(),
                ..Package::new("", "")
            }));
        }
        Ok(None)
    }

    async fn update(&self, s: bool) -> Result<()> {
        // Upgrades the pnpm CLI tool itself
        self.executor.run("pnpm", &["add", "-g", "pnpm"], s).await?;
        Ok(())
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        // REAL LOGIC: Upgrades all globally installed packages
        self.executor.run("pnpm", &["update", "-g"], s).await?;
        Ok(())
    }
}