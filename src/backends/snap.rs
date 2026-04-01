use crate::core::{CommandExecutor, Package, PackageManager, Result, PackageSpec};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct SnapManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl SnapManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for SnapManager {
    fn name(&self) -> &str { "snap" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("snap").arg("--version").output().is_ok()
        })
    }

    async fn install_with_options(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let mut args = vec!["install"];
            
            // REAL LOGIC: Handle classic confinement if specified in config
            // Many Snaps fail to install without this flag.
            if spec.options.get("classic") == Some(&"true".to_string()) {
                args.push("--classic");
            }
            
            if let Some(channel) = spec.options.get("channel") {
                args.extend(["--channel", channel]);
            }

            args.push(&spec.name);
            self.executor.run_exclusive("snap", &args, sudo).await?;
        }
        Ok(())
    }

    async fn install(&self, p: &[String], sudo: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            self.executor.run_exclusive("snap", &["install", pkg], sudo).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], sudo: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            self.executor.run_exclusive("snap", &["remove", pkg], sudo).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'snap list' table
        let out = self.executor.run_output("snap", &["list"], false).await?;
        Ok(out.lines().skip(1).filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 2 {
                Some(Package {
                    name: parts[0].to_string(),
                    version: Some(parts[1].to_string()),
                    backend: "snap".into(),
                    ..Package::new("", "")
                })
            } else { None }
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // NUCLEAR DELETE FIX: Filter out base system snaps.
        // Snaps like 'core22' and 'snapd' are required for the system to function.
        // If LiNix "drift detection" sees them, it must NOT try to uninstall them.
        let installed = self.list_installed().await?;
        let system_snaps = ["core", "core18", "core20", "core22", "snapd", "bare", "gtk-common-themes", "gnome-3-38-2004"];
        
        Ok(installed.into_iter()
            .filter(|p| !system_snaps.contains(&p.name.as_str()))
            .collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'snap find' output
        let out = self.executor.run_output("snap", &["find", query], false).await?;
        Ok(out.lines().skip(1).filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 3 {
                let mut p = Package::new(parts[0], "snap");
                p.version = Some(parts[1].to_string());
                p.description = Some(parts[parts.len()-1].to_string());
                Some(p)
            } else { None }
        }).collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Parse 'snap info' for detailed metadata
        let out = self.executor.run_output("snap", &["info", package], false).await?;
        if out.is_empty() { return Ok(None); }

        let mut p = Package::new(package, "snap");
        for line in out.lines() {
            if let Some(v) = line.strip_prefix("summary:") { p.description = Some(v.trim().to_string()); }
            if let Some(v) = line.strip_prefix("installed:") { 
                let ver = v.split_whitespace().next().unwrap_or(v);
                p.version = Some(ver.trim().to_string()); 
            }
            if let Some(v) = line.strip_prefix("website:") { p.repository = Some(v.trim().to_string()); }
        }
        Ok(Some(p))
    }

    async fn update(&self, sudo: bool) -> Result<()> {
        // Snap handles its own updates, but 'refresh' forces it now
        self.executor.run_exclusive("snap", &["refresh"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        self.executor.run_exclusive("snap", &["refresh"], sudo).await?;
        Ok(())
    }
}