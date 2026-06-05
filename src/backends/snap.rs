use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Upgradable
};
use crate::parsers::utils::sanitize;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, info};

/// Core backend implementation for Ubuntu Snap packages.
pub struct SnapBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl SnapBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "snap".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for SnapBackendCore {
    fn name(&self) -> &str { "snap" }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("snap")
    }
}

pub struct SnapInstallable {
    pub core: Arc<SnapBackendCore>,
}

#[async_trait]
impl Installable for SnapInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let mut args = vec!["install".to_string()];
            
            if spec.options.get("classic") == Some(&"true".to_string()) {
                args.push("--classic".into());
            }
            
            if let Some(channel) = spec.options.get("channel") {
                args.push("--channel".into());
                args.push(channel.clone());
            }

            args.push(spec.name.clone());
            
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            info!("Snap: Installing {}...", spec.name);
            
            self.core.executor.run_exclusive("snap", "snap", &arg_refs, sudo).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            info!("Snap: Removing {}...", name);
            self.core.executor.run_exclusive("snap", "snap", &["remove", name], sudo).await?;
        }
        Ok(())
    }
}

pub struct SnapQueryable {
    pub core: Arc<SnapBackendCore>,
}

#[async_trait]
impl Queryable for SnapQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("snap", &["list"], false).await?;
        let mut packages = Vec::new();
        
        for line in sanitize(&output).lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let (Some(name), Some(version)) = (parts.get(0), parts.get(1)) {
                packages.push(Package::with_version(*name, *version, "snap"));
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let installed = self.list_installed().await?;
        let base_snaps = ["core", "core18", "core20", "core22", "snapd", "bare", "gtk-common-themes"];
        Ok(installed.into_iter()
            .filter(|p| !base_snaps.contains(&p.name.as_str()))
            .collect())
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let output = self.core.executor.run_output("snap", &["info", name], false).await?;
        if output.is_empty() { return Ok(None); }

        let mut p = Package::new(name, "snap");
        for line in output.lines() {
            if let Some(v) = line.strip_prefix("summary:") { 
                p.properties.insert("description".into(), v.trim().to_string()); 
            }
            if let Some(v) = line.strip_prefix("installed:") { 
                let ver = v.split_whitespace().next().unwrap_or(v);
                p.version = Some(ver.trim().to_string()); 
            }
        }
        Ok(Some(p))
    }
}

pub struct SnapUpgradable {
    pub core: Arc<SnapBackendCore>,
}

#[async_trait]
impl Upgradable for SnapUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        debug!("Snap: Refreshing all snaps...");
        self.core.executor.run_exclusive("snap", "snap", &["refresh"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        self.update(sudo).await
    }

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        // Snap automatically manages its own revisions and core dependencies.
        Ok(())
    }
}