use crate::core::{
    CommandExecutor, Package, Result, PackageSpec,
    BackendCore, Installable, Queryable, Searchable, Upgradable, MetadataProvider
};
use crate::parsers::utils::sanitize;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

/// Core backend implementation for Flatpak applications.
pub struct FlatpakBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub available: OnceCell<bool>,
    /// Backend-specific settings like default scope (user vs system).
    pub settings: HashMap<String, String>,
}

impl FlatpakBackendCore {
    pub fn new(executor: CommandExecutor, settings: HashMap<String, String>) -> Self {
        Self { 
            executor, 
            name: "flatpak".to_string(),
            available: OnceCell::new(),
            settings 
        }
    }

    /// Helper to determine if the manager should operate in --user or --system scope.
    pub fn scope_args(&self) -> Vec<&str> {
        if self.settings.get("user").map(|v| v == "true").unwrap_or(false) {
            vec!["--user"]
        } else {
            vec!["--system"]
        }
    }
}

#[async_trait]
impl BackendCore for FlatpakBackendCore {
    fn name(&self) -> &str { &self.name }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.executor.command_exists_sync("flatpak"))
    }

    fn needs_root(&self) -> bool {
        // If the 'user' setting is true, Flatpak does not need root privileges.
        !self.settings.get("user").map(|v| v == "true").unwrap_or(false)
    }
}

#[async_trait]
impl MetadataProvider for FlatpakBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let args = self.scope_args();
        let mut final_args = args.clone();
        final_args.extend(["info", "--show-metadata", name]);

        // Flatpak metadata contains a [Extension] or [Runtime] section.
        // We look for 'runtime=' which is the primary transitive dependency.
        let output = self.executor.run_output("flatpak", &final_args, false).await?;
        let mut deps = Vec::new();

        for line in output.lines() {
            if let Some(runtime) = line.strip_prefix("runtime=") {
                deps.push(runtime.trim().to_string());
            }
        }

        Ok(deps)
    }
}

pub struct FlatpakInstallable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Installable for FlatpakInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() { return Ok(()); }

        let mut args = self.core.scope_args();
        args.extend(["install", "-y", "--noninteractive"]);
        
        let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        args.extend(name_refs);

        info!("Flatpak: Installing {} package(s)...", specs.len());
        self.core.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() { return Ok(()); }

        let mut args = self.core.scope_args();
        args.extend(["uninstall", "-y", "--noninteractive"]);
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        args.extend(name_refs);

        info!("Flatpak: Removing {} package(s)...", names.len());
        self.core.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }
}

pub struct FlatpakQueryable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Queryable for FlatpakQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.core.executor.run_output("flatpak", &["list", "--app", "--columns=application,version"], false).await?;
        let mut packages = Vec::new();

        for line in sanitize(&out).lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                packages.push(Package::with_version(parts[0], parts[1], "flatpak"));
            } else if !line.is_empty() {
                packages.push(Package::new(line.trim(), "flatpak"));
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct FlatpakSearchable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Searchable for FlatpakSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("flatpak", &["search", query], false).await?;
        Ok(parse_flatpak_search(&output))
    }
}

/// Parse `flatpak search <q>` => TAB-separated columns:
/// Name \t Description \t Application ID \t Version \t Branch \t Remotes.
/// The Application ID is the installable identifier, so prefer it as the name.
fn parse_flatpak_search(output: &str) -> Vec<Package> {
    let mut results = Vec::new();
    for line in sanitize(output).lines() {
        if line.trim().is_empty() { continue; }
        let cols: Vec<&str> = line.split('\t').map(|c| c.trim()).collect();
        let display_name = cols.first().copied().unwrap_or("").trim();
        let app_id = cols.get(2).copied().filter(|s| !s.is_empty()).unwrap_or(display_name);
        if app_id.is_empty() { continue; }
        let mut p = Package::new(app_id, "flatpak");
        if let Some(desc) = cols.get(1).filter(|s| !s.is_empty()) {
            p.properties.insert("description".into(), desc.to_string());
        }
        if let Some(ver) = cols.get(3).filter(|s| !s.is_empty()) {
            p.version = Some(ver.to_string());
        }
        results.push(p);
    }
    results
}

pub struct FlatpakUpgradable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Upgradable for FlatpakUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        // Must pass -y --noninteractive (like install/upgrade/clean_orphans), otherwise an
        // automated run blocks on flatpak's interactive confirmation prompt.
        let mut args = self.core.scope_args();
        args.extend(["update", "-y", "--noninteractive"]);
        debug!("Flatpak: Refreshing remotes...");
        self.core.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        let mut args = self.core.scope_args();
        args.extend(["update", "-y", "--noninteractive"]);
        info!("Flatpak: Upgrading all applications...");
        self.core.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        let mut args = self.core.scope_args();
        args.extend(["uninstall", "--unused", "-y", "--noninteractive"]);
        info!("Flatpak: Removing unused runtimes and extensions...");
        self.core.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }
}

/// Build and register the Flatpak backend with all its capabilities.
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let settings = cfg.backend_settings.get("flatpak").cloned().unwrap_or_default();
    let core = Arc::new(FlatpakBackendCore::new(exec.duplicate(), settings));
    reg.register(Arc::new(crate::core::BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(FlatpakInstallable { core: core.clone() }))
        .with_queryable(Arc::new(FlatpakQueryable { core: core.clone() }))
        .with_searchable(Arc::new(FlatpakSearchable { core: core.clone() }))
        .with_upgradable(Arc::new(FlatpakUpgradable { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatpak_search_prefers_app_id() {
        // Name \t Description \t AppID \t Version \t Branch \t Remotes
        let out = "Blender\tFree 3D suite\torg.blender.Blender\t4.0\tstable\tflathub\n\
                   GIMP\tImage editor\torg.gimp.GIMP\t2.10\tstable\tflathub\n";
        let pkgs = parse_flatpak_search(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "org.blender.Blender");
        assert_eq!(pkgs[0].version.as_deref(), Some("4.0"));
        assert_eq!(pkgs[0].properties.get("description").map(String::as_str), Some("Free 3D suite"));
    }
}