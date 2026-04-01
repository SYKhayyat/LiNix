use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct WingetManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl WingetManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for WingetManager {
    fn name(&self) -> &str { "winget" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            // Checks for 'winget' in the Windows PATH
            std::process::Command::new("winget").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // HANGING ROBOT FIX: Added --accept-package-agreements and --accept-source-agreements
            // Added --id to ensure we install the exact package requested
            self.executor.run("winget", &[
                "install", "--id", pkg, "--silent", 
                "--accept-package-agreements", "--accept-source-agreements"
            ], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            self.executor.run("winget", &["uninstall", "--id", pkg, "--silent"], false).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'winget list'
        let out = self.executor.run_output("winget", &["list"], false).await?;
        Ok(out.lines()
            .skip(2) // Skip headers and the dash line (---)
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    // Winget columns: Name (0), Id (1), Version (2)
                    Some(Package {
                        name: parts[1].to_string(), // Use ID as name for reliability
                        version: Some(parts[2].to_string()),
                        backend: "winget".into(),
                        ..Package::new("", "")
                    })
                } else { None }
            }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // Winget doesn't perfectly track "manual" vs "dependency".
        // However, we filter for items that have a "Source" column entry (like 'winget' or 'msstore'),
        // which usually indicates an app managed by the system rather than a built-in library.
        let out = self.executor.run_output("winget", &["list"], false).await?;
        Ok(out.lines()
            .skip(2)
            .filter(|l| l.contains("winget") || l.contains("msstore"))
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some(Package::new(parts[1], "winget"))
                } else { None }
            }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'winget search' table
        let out = self.executor.run_output("winget", &["search", query], false).await?;
        Ok(out.lines()
            .skip(2)
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let mut p = Package::new(parts[1], "winget");
                    p.description = Some(parts[0].to_string()); // Store human name in description
                    Some(p)
                } else { None }
            }).collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Parse 'winget show' for specific metadata
        let out = self.executor.run_output("winget", &["show", "--id", package], false).await?;
        if out.is_empty() { return Ok(None); }

        let mut p = Package::new(package, "winget");
        for line in out.lines() {
            if let Some(v) = line.strip_prefix("Description: ") { p.description = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Version: ") { p.version = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Homepage: ") { p.repository = Some(v.trim().to_string()); }
        }
        Ok(Some(p))
    }

    /// FIX: Implemented real 'source' logic for repositories
    async fn add_repo(&self, name: &str, url: &str, _: bool) -> Result<()> {
        self.executor.run("winget", &["source", "add", "--name", name, "--arg", url], false).await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, _: bool) -> Result<()> {
        self.executor.run("winget", &["source", "remove", "--name", name], false).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let out = self.executor.run_output("winget", &["source", "list"], false).await?;
        Ok(out.lines()
            .skip(2)
            .filter_map(|l| {
                let parts: Vec<&str> = l.split_whitespace().collect();
                if parts.len() >= 2 {
                    Some((parts[0].to_string(), parts[1].to_string()))
                } else { None }
            }).collect())
    }

    async fn update(&self, _: bool) -> Result<()> {
        // winget source update refreshes the local package catalogs
        self.executor.run("winget", &["source", "update"], false).await?;
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        // REAL LOGIC: Upgrades all upgradable packages
        self.executor.run("winget", &["upgrade", "--all", "--silent", "--accept-package-agreements"], false).await?;
        Ok(())
    }
}