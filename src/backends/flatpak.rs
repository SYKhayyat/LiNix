use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct FlatpakManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    settings: Option<HashMap<String, String>>,
}

impl FlatpakManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }

    /// Determines if we should use --user or --system (default)
    fn is_user(&self) -> bool {
        self.settings.as_ref()
            .and_then(|s| s.get("user_installation"))
            .map(|v| v == "true")
            .unwrap_or(false)
    }

    fn scope_args(&self) -> Vec<&str> {
        if self.is_user() { vec!["--user"] } else { vec!["--system"] }
    }
}

#[async_trait]
impl PackageManager for FlatpakManager {
    fn name(&self) -> &str { "flatpak" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("flatpak").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = self.scope_args();
        args.extend(["install", "-y", "--noninteractive"]);
        args.extend(p.iter().map(|x| x.as_str()));
        // Use sudo only if system-wide and s is true
        self.executor.run("flatpak", &args, !self.is_user() && s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = self.scope_args();
        args.extend(["uninstall", "-y", "--noninteractive"]);
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("flatpak", &args, !self.is_user() && s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // Returns EVERYTHING (apps + runtimes/dependencies)
        let out = self.executor.run_output("flatpak", &["list", "--columns=application,version"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, ver) = l.split_once('\t')?;
            Some(Package { 
                name: name.trim().to_string(), 
                version: Some(ver.trim().to_string()), 
                backend: "flatpak".into(), 
                ..Package::new("", "") 
            })
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // NUCLEAR DELETE FIX: Only returns actual applications. 
        // Runtimes (drivers/libraries) are filtered out via --app.
        let out = self.executor.run_output("flatpak", &["list", "--app", "--columns=application"], false).await?;
        Ok(out.lines()
            .filter(|l| !l.is_empty())
            .map(|l| Package::new(l.trim(), "flatpak"))
            .collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let out = self.executor.run_output("flatpak", &["search", "--columns=application,description", query], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, desc) = l.split_once('\t')?;
            let mut p = Package::new(name.trim(), "flatpak");
            p.description = Some(desc.trim().to_string());
            Some(p)
        }).collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let out = self.executor.run_output("flatpak", &["info", package], false).await?;
        if out.is_empty() { return Ok(None); }
        
        let mut p = Package::new(package, "flatpak");
        for line in out.lines() {
            if let Some(v) = line.strip_prefix("Version: ") { p.version = Some(v.trim().to_string()); }
            if let Some(v) = line.strip_prefix("Description: ") { p.description = Some(v.trim().to_string()); }
        }
        Ok(Some(p))
    }

    /// FIX: Implemented real 'remote' logic for repositories
    async fn add_repo(&self, name: &str, url: &str, s: bool) -> Result<()> {
        let mut args = self.scope_args();
        args.extend(["remote-add", "--if-not-exists", name, url]);
        self.executor.run("flatpak", &args, !self.is_user() && s).await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, s: bool) -> Result<()> {
        let mut args = self.scope_args();
        args.extend(["remote-delete", name]);
        self.executor.run("flatpak", &args, !self.is_user() && s).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        // Lists all remotes and their URLs
        let out = self.executor.run_output("flatpak", &["remotes", "--columns=name,url"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, url) = l.split_once('\t')?;
            Some((name.trim().to_string(), url.trim().to_string()))
        }).collect())
    }

    async fn update(&self, _: bool) -> Result<()> {
        // Flatpak update refreshes remotes and checks for metadata updates
        let mut args = self.scope_args();
        args.push("update");
        args.push("--appstream"); 
        self.executor.run("flatpak", &args, false).await?;
        Ok(())
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        let mut args = self.scope_args();
        args.extend(["update", "-y", "--noninteractive"]);
        self.executor.run("flatpak", &args, !self.is_user() && s).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool { true }
    async fn clean_orphans(&self, _: bool) -> Result<()> {
        // Removes runtimes and extensions that are no longer used by any installed app
        self.executor.run("flatpak", &["uninstall", "--unused", "-y"], false).await?;
        Ok(())
    }
}