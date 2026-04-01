use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct YarnManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl YarnManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for YarnManager {
    fn name(&self) -> &str { "yarn" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            // Checks for the 'yarn' binary in the system PATH
            std::process::Command::new("yarn").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        // --non-interactive: Essential for background automation
        let mut args = vec!["global", "add", "--non-interactive"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("yarn", &args, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["global", "remove", "--non-interactive"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("yarn", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse the output of 'yarn global list --depth=0'
        // Format: "├── package-name@1.2.3"
        let out = self.executor.run_output("yarn", &["global", "list", "--depth=0"], false).await?;
        Ok(out.lines()
            .filter(|line| line.contains('@') && !line.contains("info"))
            .filter_map(|line| {
                let cleaned = line.trim()
                    .trim_start_matches("├── ")
                    .trim_start_matches("└── ")
                    .trim();
                
                // Use rsplit_once to handle scoped packages like @scope/name@1.2.3
                let (name, ver) = cleaned.rsplit_once('@')?;
                Some(Package {
                    name: name.to_string(),
                    version: Some(ver.to_string()),
                    backend: "yarn".into(),
                    ..Package::new("", "")
                })
            }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Query the NPM Registry API (Yarn uses the same ecosystem)
        let url = format!("https://registry.npmjs.org/-/v1/search?text={}&size=20", query);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(objects) = json.get("objects").and_then(|o| o.as_array()) {
                return Ok(objects.iter().filter_map(|obj| {
                    let pkg = obj.get("package")?;
                    let name = pkg.get("name")?.as_str()?;
                    Some(Package::new(name, "yarn"))
                }).collect());
            }
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Fetch rich metadata (description, homepage) from the registry
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
                backend: "yarn".into(),
                ..Package::new("", "")
            }));
        }
        Ok(None)
    }

    async fn update(&self, _: bool) -> Result<()> {
        // Yarn doesn't have a local index to refresh
        Ok(())
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        // REAL LOGIC: Upgrades all globally installed yarn packages
        self.executor.run("yarn", &["global", "upgrade", "--non-interactive"], s).await?;
        Ok(())
    }
}