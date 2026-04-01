use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PacmanManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl PacmanManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for PacmanManager {
    fn name(&self) -> &str { "pacman" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("pacman").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], sudo: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        // --needed: Prevents re-downloading/re-installing if already current
        // --noconfirm: Essential for automation
        let mut args = vec!["-S", "--noconfirm", "--needed"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run_exclusive("pacman", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], sudo: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        // -Rs: Removes the package AND its now-unneeded dependencies (cleaner system)
        let mut args = vec!["-Rs", "--noconfirm"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run_exclusive("pacman", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse pacman -Q (name version)
        let out = self.executor.run_output("pacman", &["-Q"], false).await?;
        Ok(out.lines().filter_map(|line| {
            let (name, ver) = line.split_once(' ')?;
            Some(Package { 
                name: name.to_string(), 
                version: Some(ver.to_string()), 
                backend: "pacman".into(), 
                ..Package::new("", "") 
            })
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // NUCLEAR DELETE FIX: -Qe lists only packages explicitly installed by the user.
        // This ensures LiNix never tries to "garbage collect" your Linux Kernel or drivers.
        let out = self.executor.run_output("pacman", &["-Qe"], false).await?;
        Ok(out.lines().filter_map(|line| {
            let (name, _) = line.split_once(' ')?;
            Some(Package::new(name, "pacman"))
        }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse multi-line output of pacman -Ss
        let out = self.executor.run_output("pacman", &["-Ss", query], false).await?;
        let mut results = Vec::new();
        let mut lines = out.lines().peekable();

        while let Some(line) = lines.next() {
            if line.starts_with(' ') || line.is_empty() { continue; }
            
            // Format: repo/name version (groups) [status]
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(repo_name) = parts.get(0) {
                let name = repo_name.split('/').last().unwrap_or(repo_name);
                let mut p = Package::new(name, "pacman");
                p.version = parts.get(1).map(|v| v.to_string());
                
                // Peek next line for description
                if let Some(desc_line) = lines.peek() {
                    if desc_line.starts_with("    ") {
                        p.description = Some(desc_line.trim().to_string());
                        lines.next(); // Consume the description line
                    }
                }
                results.push(p);
            }
        }
        Ok(results)
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Query the Sync database for remote info
        let out = self.executor.run_output("pacman", &["-Si", package], false).await?;
        if out.is_empty() { return Ok(None); }
        
        let mut p = Package::new(package, "pacman");
        for line in out.lines() {
            if let Some(v) = line.strip_prefix("Version         : ") { p.version = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("Description     : ") { p.description = Some(v.trim().to_string()); }
            else if let Some(v) = line.strip_prefix("URL             : ") { p.repository = Some(v.trim().to_string()); }
        }
        Ok(Some(p))
    }

    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        // REAL LOGIC: Adds a custom repository block to /etc/pacman.conf
        let repo_block = format!("\n[{}]\nSigLevel = Optional TrustAll\nServer = {}\n", name, url);
        let cmd = format!("echo '{}' >> /etc/pacman.conf", repo_block);
        self.executor.run("sh", &["-c", &cmd], sudo).await?;
        self.executor.run("pacman", &["-Sy"], sudo).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        // REAL LOGIC: Parses pacman.conf for active repository headers
        let out = self.executor.run_output("grep", &["-E", "^\\[.*\\]", "/etc/pacman.conf"], false).await?;
        Ok(out.lines()
            .map(|l| l.trim_matches(|c| c == '[' || c == ']'))
            .filter(|&l| l != "options")
            .map(|l| (l.to_string(), "active".to_string()))
            .collect())
    }

    async fn update(&self, sudo: bool) -> Result<()> {
        // -Sy refreshes the package databases
        self.executor.run_exclusive("pacman", &["-Sy"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        // -Syu is the Arch standard for a full system upgrade
        self.executor.run_exclusive("pacman", &["-Syu", "--noconfirm"], sudo).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool { true }
    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        // REAL LOGIC: pacman -Qdtq finds "unneeded dependencies". We then pipe to -Rs to remove.
        let orphans = self.executor.run_output("pacman", &["-Qdtq"], false).await?;
        if !orphans.trim().is_empty() {
            let list: Vec<&str> = orphans.lines().collect();
            let mut args = vec!["-Rs", "--noconfirm"];
            args.extend(list);
            self.executor.run_exclusive("pacman", &args, sudo).await?;
        }
        Ok(())
    }
}