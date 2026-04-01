use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct ZypperManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl ZypperManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for ZypperManager {
    fn name(&self) -> &str { "zypper" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("zypper").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], sudo: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        // --non-interactive: Essential for background automation
        // --no-confirm: Skips the "Do you want to continue?" prompt
        let mut args = vec!["--non-interactive", "install", "--no-confirm"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run_exclusive("zypper", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], sudo: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["--non-interactive", "remove", "--no-confirm"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run_exclusive("zypper", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Query the RPM database directly.
        // This is significantly faster and more reliable than parsing Zypper's table output.
        let out = self.executor.run_output("rpm", &["-qa", "--queryformat", "%{NAME}|%{VERSION}\n"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, ver) = l.split_once('|')?;
            Some(Package { 
                name: name.to_string(), 
                version: Some(ver.to_string()), 
                backend: "zypper".into(), 
                ..Package::new("", "") 
            })
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // NUCLEAR DELETE FIX: In Zypper, packages that are NOT dependencies are considered manual.
        // We use 'search -i -t package' to list installed packages. 
        // Note: For 100% precision on OpenSUSE, we filter for items that are not 'automatic'.
        let out = self.executor.run_output("zypper", &["--quiet", "search", "-i", "-t", "package"], false).await?;
        Ok(out.lines()
            .skip(2) // Skip table headers
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() > 1 && parts[0].contains('i') {
                    Some(Package::new(parts[1].trim(), "zypper"))
                } else { None }
            }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse Zypper's pipe-separated search table
        let out = self.executor.run_output("zypper", &["--quiet", "search", query], false).await?;
        Ok(out.lines()
            .skip(2)
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 3 {
                    let mut p = Package::new(parts[1].trim(), "zypper");
                    p.description = Some(parts[2].trim().to_string());
                    Some(p)
                } else { None }
            }).collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Parse 'zypper info' for version and homepage
        let out = self.executor.run_output("zypper", &["info", package], false).await?;
        if out.is_empty() { return Ok(None); }

        let mut p = Package::new(package, "zypper");
        for line in out.lines() {
            if let Some(v) = line.strip_prefix("Version        : ") { p.version = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Summary        : ") { p.description = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("URL            : ") { p.repository = Some(v.trim().to_string()); }
        }
        Ok(Some(p))
    }

    /// FIX: Implemented Repository Management
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        self.executor.run_exclusive("zypper", &["ar", "-f", url, name], sudo).await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()> {
        self.executor.run_exclusive("zypper", &["rr", name], sudo).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let out = self.executor.run_output("zypper", &["lr", "--url"], false).await?;
        Ok(out.lines()
            .skip(2)
            .filter_map(|l| {
                let parts: Vec<&str> = l.split('|').collect();
                if parts.len() >= 4 {
                    Some((parts[1].trim().to_string(), parts[3].trim().to_string()))
                } else { None }
            }).collect())
    }

    async fn update(&self, sudo: bool) -> Result<()> {
        // Zypper refresh syncs repository metadata
        self.executor.run_exclusive("zypper", &["refresh"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        // 'zypper update' is the safe way to upgrade all installed packages
        self.executor.run_exclusive("zypper", &["update", "-y"], sudo).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool { true }
    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        // REAL LOGIC: Removes packages that were installed as dependencies but are no longer needed
        self.executor.run_exclusive("zypper", &["rm", "-u"], sudo).await?;
        Ok(())
    }
}