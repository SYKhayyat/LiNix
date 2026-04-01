use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct GemManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl GemManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for GemManager {
    fn name(&self) -> &str { "gem" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("gem").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        // --no-document skips slow documentation generation during automation
        let mut args = vec!["install", "--no-document"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run_exclusive("gem", &args, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        // -a = all versions, -x = ignore dependencies
        let mut args = vec!["uninstall", "-a", "-x"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run_exclusive("gem", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'gem list --local'
        // Format is: "package-name (1.2.3, 1.1.0)"
        let out = self.executor.run_output("gem", &["list", "--local"], false).await?;
        Ok(out.lines()
            .filter(|l| !l.is_empty() && !l.starts_with("***"))
            .filter_map(|line| {
                let (name, rest) = line.split_once(' ')?;
                let version = rest.trim().trim_matches(|c| c == '(' || c == ')').split(',').next();
                Some(Package {
                    name: name.trim().to_string(),
                    version: version.map(|v| v.trim().to_string()),
                    backend: "gem".into(),
                    ..Package::new("", "")
                })
            }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Query RubyGems API (standard CLI search is incredibly slow)
        let url = format!("https://rubygems.org/api/v1/search.json?query={}", query);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(items) = json.as_array() {
                return Ok(items.iter().filter_map(|i| {
                    let name = i.get("name")?.as_str()?;
                    let mut p = Package::new(name, "gem");
                    p.description = i.get("info").and_then(|s| s.as_str()).map(|s| s.to_string());
                    p.version = i.get("version").and_then(|s| s.as_str()).map(|s| s.to_string());
                    Some(p)
                }).collect());
            }
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Fetch rich metadata from RubyGems API
        let url = format!("https://rubygems.org/api/v1/gems/{}.json", package);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let i: serde_json::Value = res.json().await?;
            return Ok(Some(Package {
                name: package.to_string(),
                version: i.get("version").and_then(|s| s.as_str()).map(|s| s.to_string()),
                description: i.get("info").and_then(|s| s.as_str()).map(|s| s.to_string()),
                repository: i.get("source_code_uri").and_then(|s| s.as_str()).map(|s| s.to_string()),
                backend: "gem".into(),
                ..Package::new("", "")
            }));
        }
        Ok(None)
    }

    /// FIX: Implemented Repository (Source) Management
    async fn add_repo(&self, _: &str, url: &str, s: bool) -> Result<()> {
        self.executor.run("gem", &["sources", "-a", url], s).await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, s: bool) -> Result<()> {
        // 'name' here is the URL for gem sources
        self.executor.run("gem", &["sources", "-r", name], s).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let out = self.executor.run_output("gem", &["sources"], false).await?;
        Ok(out.lines()
            .skip(2) // Skip header
            .filter(|l| !l.is_empty())
            .map(|l| (l.trim().to_string(), "source".to_string()))
            .collect())
    }

    async fn update(&self, _: bool) -> Result<()> {
        // RubyGems update refreshes the source index
        self.executor.run("gem", &["sources", "-u"], false).await?;
        Ok(())
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        // REAL LOGIC: Upgrade all installed gems
        self.executor.run("gem", &["update"], s).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool { true }
    async fn clean_orphans(&self, s: bool) -> Result<()> {
        // gem cleanup removes old versions of installed gems
        self.executor.run("gem", &["cleanup"], s).await?;
        Ok(())
    }
}