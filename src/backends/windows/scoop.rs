use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct ScoopManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl ScoopManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for ScoopManager {
    fn name(&self) -> &str { "scoop" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            // Checks for 'scoop' in the Windows PATH
            std::process::Command::new("scoop").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // Scoop is inherently non-interactive for installs
            self.executor.run("scoop", &["install", pkg], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            self.executor.run("scoop", &["uninstall", pkg], false).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'scoop list'
        // Skip headers and parse columns: Name, Version, Source, Updated
        let out = self.executor.run_output("scoop", &["list"], false).await?;
        Ok(out.lines()
            .filter(|l| !l.is_empty() && !l.contains("---") && !l.contains("Installed apps"))
            .filter_map(|l| {
                let parts: Vec<&str> = l.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some(Package { 
                        name: parts[0].to_string(), 
                        version: Some(parts[1].to_string()), 
                        backend: "scoop".into(), 
                        ..Package::new("", "") 
                    })
                } else { None }
            }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // Scoop doesn't separate dependencies from user-requested apps in its list.
        // Since Scoop is almost exclusively used for user-land tools, we treat the whole list as manual.
        self.list_installed().await
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'scoop search' table
        let out = self.executor.run_output("scoop", &["search", query], false).await?;
        Ok(out.lines()
            .filter(|l| !l.is_empty() && !l.contains("---") && !l.contains("Results from"))
            .filter_map(|l| {
                let parts: Vec<&str> = l.split_whitespace().collect();
                if !parts.is_empty() && !parts[0].is_empty() {
                    Some(Package::new(parts[0], "scoop"))
                } else { None }
            }).collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Parse 'scoop info' for detailed metadata
        let out = self.executor.run_output("scoop", &["info", package], false).await?;
        if out.is_empty() { return Ok(None); }

        let mut p = Package::new(package, "scoop");
        for line in out.lines() {
            if let Some(v) = line.strip_prefix("Version: ") { p.version = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Description: ") { p.description = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Website: ") { p.repository = Some(v.trim().to_string()); }
        }
        Ok(Some(p))
    }

    /// FIX: Implemented real 'bucket' logic for repositories
    async fn add_repo(&self, name: &str, url: &str, _: bool) -> Result<()> {
        // Scoop calls repositories 'buckets'
        self.executor.run("scoop", &["bucket", "add", name, url], false).await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, _: bool) -> Result<()> {
        self.executor.run("scoop", &["bucket", "rm", name], false).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let out = self.executor.run_output("scoop", &["bucket", "list"], false).await?;
        Ok(out.lines()
            .skip(2) // Skip headers
            .filter_map(|l| {
                let parts: Vec<&str> = l.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some((parts[0].to_string(), parts[1].to_string()))
                } else { None }
            }).collect())
    }

    async fn update(&self, _: bool) -> Result<()> {
        // 'scoop update' refreshes the local manifests and bucket data
        self.executor.run("scoop", &["update"], false).await?;
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        // REAL LOGIC: Upgrades all installed apps
        self.executor.run("scoop", &["update", "*"], false).await?;
        Ok(())
    }
}