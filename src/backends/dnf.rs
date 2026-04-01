use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct DnfManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl DnfManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for DnfManager {
    fn name(&self) -> &str { "dnf" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("dnf").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        // --best and --allowerasing are mandatory for clean automated installs on Fedora/RHEL
        let mut args = vec!["install", "-y", "--best", "--allowerasing"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run_exclusive("dnf", &args, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["remove", "-y"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run_exclusive("dnf", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Query RPM database directly (much faster than DNF for listing)
        let out = self.executor.run_output("rpm", &["-qa", "--queryformat", "%{NAME}|%{VERSION}\n"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, ver) = l.split_once('|')?;
            Some(Package { 
                name: name.to_string(), 
                version: Some(ver.to_string()), 
                backend: "dnf".into(), 
                ..Package::new("", "") 
            })
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // NUCLEAR DELETE FIX: repoquery --userinstalled is the only way to find packages 
        // the human actually typed. Everything else is a system dependency.
        let out = self.executor.run_output("dnf", &["repoquery", "--userinstalled", "--queryformat", "%{name}"], false).await?;
        Ok(out.lines()
            .filter(|line| !line.is_empty())
            .map(|line| Package::new(line.trim(), "dnf"))
            .collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let out = self.executor.run_output("dnf", &["search", "-q", query], false).await?;
        Ok(out.lines()
            .filter_map(|line| {
                let (name, _) = line.split_once('.')?;
                if name.contains(':') || name.starts_with('=') { return None; }
                Some(Package::new(name.trim(), "dnf"))
            }).collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let out = self.executor.run_output("dnf", &["info", "-q", package], false).await?;
        if out.is_empty() { return Ok(None); }
        
        let mut p = Package::new(package, "dnf");
        for line in out.lines() {
            if let Some(v) = line.strip_prefix("Version      : ") { p.version = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Description  : ") { p.description = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("URL          : ") { p.repository = Some(v.trim().to_string()); }
        }
        Ok(Some(p))
    }

    async fn add_repo(&self, _: &str, url: &str, sudo: bool) -> Result<()> {
        // REAL REPO LOGIC: Use dnf-config-manager to safely add .repo files
        self.executor.run_exclusive("dnf", &["config-manager", "--add-repo", url], sudo).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let out = self.executor.run_output("dnf", &["repolist", "-q"], false).await?;
        Ok(out.lines().skip(1).filter_map(|l| {
            let (id, name) = l.split_once(' ')?;
            Some((id.trim().to_string(), name.trim().to_string()))
        }).collect())
    }

    async fn update(&self, sudo: bool) -> Result<()> {
        self.executor.run_exclusive("dnf", &["makecache"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        self.executor.run_exclusive("dnf", &["upgrade", "-y"], sudo).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool { true }
    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        // REAL LOGIC: DNF autoremove cleans up leaf nodes no longer required
        self.executor.run_exclusive("dnf", &["autoremove", "-y"], sudo).await?;
        Ok(())
    }
}