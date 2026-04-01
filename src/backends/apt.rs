use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct AptManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl AptManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
    
    // AUTOMATION FIX: Stops APT from waiting for user input during background syncs
    fn env(&self) -> HashMap<String, String> {
        let mut e = HashMap::new();
        e.insert("DEBIAN_FRONTEND".into(), "noninteractive".into());
        e
    }
}

#[async_trait]
impl PackageManager for AptManager {
    fn name(&self) -> &str { "apt" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("apt-get").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        // force-confold: Keeps current configs when a package update asks to overwrite them
        let mut args = vec!["install", "-y", "-o", "Dpkg::Options::=--force-confold"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run_with_env("apt-get", &args, s, &self.env()).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["purge", "-y"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run_with_env("apt-get", &args, s, &self.env()).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Uses dpkg-query for fast, reliable parsing
        let out = self.executor.run_output("dpkg-query", &["-W", "-f=${Package} ${Version}\n"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (n, v) = l.split_once(' ')?;
            Some(Package { name: n.into(), version: Some(v.into()), backend: "apt".into(), ..Package::new("", "") })
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // NUCLEAR DELETE FIX: Only returns packages user specifically requested
        let out = self.executor.run_output("apt-mark", &["showmanual"], false).await?;
        Ok(out.lines().filter(|l| !l.is_empty()).map(|l| Package::new(l.trim(), "apt")).collect())
    }

    async fn add_repo(&self, _: &str, url: &str, s: bool) -> Result<()> {
        // REAL REPO LOGIC: Supports both PPAs and raw deb lines
        if url.starts_with("ppa:") {
            self.executor.run_with_env("add-apt-repository", &["-y", url], s, &self.env()).await?;
        } else {
            let line = if url.starts_with("deb") { url.to_string() } else { format!("deb {}", url) };
            let cmd = format!("echo '{}' > /etc/apt/sources.list.d/linix_managed.list", line);
            self.executor.run("sh", &["-c", &cmd], s).await?;
        }
        self.executor.run_with_env("apt-get", &["update"], s, &self.env()).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        // REAL REPO LOGIC: Parses system files for active repositories
        let out = self.executor.run_combined_output("grep", &["-r", "^deb", "/etc/apt/sources.list", "/etc/apt/sources.list.d/"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 3 {
                Some((parts[0].trim_end_matches(':').into(), parts[2..].join(" ")))
            } else { None }
        }).collect())
    }

    async fn update(&self, sudo: bool) -> Result<()> {
        self.executor.run_exclusive("apt-get", &["update"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        self.executor.run_with_env("apt-get", &["dist-upgrade", "-y"], sudo, &self.env()).await?;
        Ok(())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let out = self.executor.run_output("apt-cache", &["search", query], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, desc) = l.split_once(" - ")?;
            let mut p = Package::new(name.trim(), "apt");
            p.description = Some(desc.trim().to_string());
            Some(p)
        }).collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let out = self.executor.run_output("apt-cache", &["show", package], false).await?;
        if out.is_empty() { return Ok(None); }
        
        let mut p = Package::new(package, "apt");
        for line in out.lines() {
            if let Some(v) = line.strip_prefix("Version: ") { p.version = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Description: ") { p.description = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Homepage: ") { p.repository = Some(v.trim().to_string()); }
        }
        Ok(Some(p))
    }

    fn supports_orphan_cleanup(&self) -> bool { true }
    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        self.executor.run_with_env("apt-get", &["autoremove", "-y"], sudo, &self.env()).await?;
        Ok(())
    }
}