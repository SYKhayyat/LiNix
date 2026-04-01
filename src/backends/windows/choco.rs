use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct ChocoManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl ChocoManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for ChocoManager {
    fn name(&self) -> &str { "choco" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            // Checks for 'choco' in the Windows PATH
            std::process::Command::new("where").arg("choco").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["install", "-y"];
        args.extend(p.iter().map(|s| s.as_str()));
        // Choco handles its own admin elevation if needed
        self.executor.run("choco", &args, false).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["uninstall", "-y"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("choco", &args, false).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'choco list -lo -r' (local only, limited output)
        // Format: "name|version"
        let out = self.executor.run_output("choco", &["list", "-lo", "-r"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let parts: Vec<&str> = l.split('|').collect();
            if parts.len() >= 2 {
                Some(Package {
                    name: parts[0].to_string(),
                    version: Some(parts[1].to_string()),
                    backend: "choco".into(),
                    ..Package::new("", "")
                })
            } else { None }
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // Choco doesn't have a perfect "user-installed only" flag,
        // but 'list -lo' (local only) is the standard for management.
        self.list_installed().await
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'choco search [query] -r'
        let out = self.executor.run_output("choco", &["search", query, "-r"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let parts: Vec<&str> = l.split('|').collect();
            if parts.len() >= 2 {
                Some(Package::new(parts[0], "choco"))
            } else { None }
        }).collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Parse 'choco info' for detailed metadata
        let out = self.executor.run_output("choco", &["info", package, "-r"], false).await?;
        if out.is_empty() { return Ok(None); }

        let mut p = Package::new(package, "choco");
        // Choco info -r returns pipe-separated or key-value depending on version.
        // We parse the most verbose common keys.
        for line in out.lines() {
            if let Some(v) = line.strip_prefix("Title: ") { p.description = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Version: ") { p.version = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Project URL: ") { p.repository = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Description: ") { p.description = Some(v.trim().to_string()); }
        }
        Ok(Some(p))
    }

    /// FIX: Implemented real 'source' logic for repositories
    async fn add_repo(&self, name: &str, url: &str, _: bool) -> Result<()> {
        self.executor.run("choco", &["source", "add", "-n", name, "-s", url], false).await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, _: bool) -> Result<()> {
        self.executor.run("choco", &["source", "remove", "-n", name], false).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let out = self.executor.run_output("choco", &["source", "list", "-r"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, url) = l.split_once('|')?;
            Some((name.trim().to_string(), url.trim().to_string()))
        }).collect())
    }

    async fn update(&self, _: bool) -> Result<()> {
        // Chocolatey refreshes metadata automatically during commands.
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        // REAL LOGIC: Upgrades all installed packages
        self.executor.run("choco", &["upgrade", "all", "-y"], false).await?;
        Ok(())
    }
}