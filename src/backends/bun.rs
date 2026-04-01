use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct BunManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl BunManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for BunManager {
    fn name(&self) -> &str { "bun" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("bun").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["add", "-g"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("bun", &args, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["remove", "-g"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("bun", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'bun pm ls -g' which outputs the global dependency tree
        let out = self.executor.run_output("bun", &["pm", "ls", "-g"], false).await?;
        Ok(out.lines()
            .filter(|l| l.contains('@') && !l.contains('/') && !l.contains(':'))
            .filter_map(|l| {
                // Clean tree characters: ├─, └─, etc.
                let cleaned = l.trim()
                    .trim_start_matches(|c: char| !c.is_alphanumeric() && c != '@')
                    .trim();
                let (name, ver) = cleaned.rsplit_once('@')?;
                Some(Package {
                    name: name.to_string(),
                    version: Some(ver.to_string()),
                    backend: "bun".into(),
                    ..Package::new("", "")
                })
            }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Bun doesn't have a search CLI, so we query the NPM Registry API directly
        let url = format!("https://registry.npmjs.org/-/v1/search?text={}&size=20", query);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(objects) = json.get("objects").and_then(|o| o.as_array()) {
                return Ok(objects.iter().filter_map(|obj| {
                    let pkg = obj.get("package")?;
                    let name = pkg.get("name")?.as_str()?;
                    Some(Package::new(name, "bun"))
                }).collect());
            }
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Fetch full package metadata from the registry
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
                backend: "bun".into(),
                ..Package::new("", "")
            }));
        }
        Ok(None)
    }

    async fn update(&self, s: bool) -> Result<()> {
        // Upgrade Bun itself
        self.executor.run("bun", &["upgrade"], s).await?;
        Ok(())
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        // Upgrade all global packages by re-installing them
        let pkgs = self.list_installed().await?;
        let names: Vec<String> = pkgs.into_iter().map(|p| p.name).collect();
        self.install(&names, s).await
    }
}