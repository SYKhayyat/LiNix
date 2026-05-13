use crate::core::{
    Backend, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Upgradable
};
use crate::parsers::utils::sanitize;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

/// Specialized manager for Ubuntu Snap packages.
/// Snaps require exclusive access to the snapd socket, so all mutations
/// use the "snap" lock key in the LockMap.
pub struct SnapManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl SnapManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }
}

impl Backend for SnapManager {
    fn name(&self) -> &str { "snap" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.executor.command_exists_sync("snap"))
    }

    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
    fn as_upgradable(&self) -> Option<&dyn Upgradable> { Some(self) }
}

#[async_trait]
impl Installable for SnapManager {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let mut args = vec!["install".to_string()];
            
            // 1. Handle Classic Confinement (Security Override)
            if spec.options.get("classic") == Some(&"true".to_string()) {
                args.push("--classic".into());
            }
            
            // 2. Handle Release Channels (e.g., beta, edge, candidate)
            if let Some(channel) = spec.options.get("channel") {
                args.push("--channel".into());
                args.push(channel.clone());
            }

            args.push(spec.name.clone());
            
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            info!("Snap: Installing {}...", spec.name);
            
            // snaps must be installed one-by-one or via the store, 
            // and mutations are globally exclusive.
            self.executor.run_exclusive("snap", "snap", &arg_refs, sudo).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            info!("Snap: Removing {}...", name);
            self.executor.run_exclusive("snap", "snap", &["remove", name], sudo).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Queryable for SnapManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor.run_output("snap", &["list"], false).await?;
        let mut packages = Vec::new();
        
        // Snap list format:
        // Name               Version          Rev    Tracking       Publisher   Notes
        // bare               1.0              5      latest/stable  canonical✓  base
        // core22             20230531         766    latest/stable  canonical✓  base
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
        // Filter out base runtime snaps that are system dependencies
        let base_snaps = ["core", "core18", "core20", "core22", "snapd", "bare", "gtk-common-themes"];
        Ok(installed.into_iter()
            .filter(|p| !base_snaps.contains(&p.name.as_str()))
            .collect())
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let output = self.executor.run_output("snap", &["info", name], false).await?;
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
            if let Some(v) = line.strip_prefix("website:") { 
                p.properties.insert("homepage".into(), v.trim().to_string()); 
            }
        }
        Ok(Some(p))
    }
}

#[async_trait]
impl Upgradable for SnapManager {
    async fn update(&self, sudo: bool) -> Result<()> {
        debug!("Snap: Refreshing all snaps...");
        self.executor.run_exclusive("snap", "snap", &["refresh"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        // Snap handles its own upgrade logic via refresh
        self.update(sudo).await
    }
}