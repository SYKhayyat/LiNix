use crate::core::{
    Backend, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Upgradable, Error
};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde_json::Value;
use tracing::{debug, info};

/// Specialized manager for Nix (via 'nix profile').
/// Supports modern Nix Flakes and declarative profile management.
/// Uses the LockMap key "nix" to prevent concurrent profile mutations.
pub struct NixManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl NixManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }
}

impl Backend for NixManager {
    fn name(&self) -> &str { "nix" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.executor.command_exists_sync("nix"))
    }

    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
    fn as_upgradable(&self) -> Option<&dyn Upgradable> { Some(self) }
}

#[async_trait]
impl Installable for NixManager {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            // Support Flake URI syntax (e.g. nixpkgs#htop) or simple names
            let flake_uri = if spec.name.contains('#') {
                spec.name.clone()
            } else {
                format!("nixpkgs#{}", spec.name)
            };

            info!("Nix: Installing {} to user profile...", flake_uri);
            // Nix profile operations must be serialized to avoid lock contention
            self.executor.run_exclusive("nix", "nix", &["profile", "install", &flake_uri], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        // Nix profile removal requires the attribute path or the list index.
        // We fetch the current list to map names to indices.
        let installed = self.list_installed().await?;
        
        for name in names {
            if let Some(pkg) = installed.iter().find(|p| p.name == *name) {
                if let Some(index) = pkg.properties.get("index") {
                    info!("Nix: Removing package at profile index {} ({})", index, name);
                    self.executor.run_exclusive("nix", "nix", &["profile", "remove", index], false).await?;
                } else {
                    // Fallback to name-based removal if index lookup fails
                    self.executor.run_exclusive("nix", "nix", &["profile", "remove", name], false).await?;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Queryable for NixManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        // Use --json for production-grade reliability
        let output = self.executor.run_output("nix", &["profile", "list", "--json"], false).await?;
        if output.is_empty() || output == "{}" { return Ok(vec![]); }

        let json: Value = serde_json::from_str(&output).map_err(|e| Error::Other(format!("Nix JSON error: {}", e)))?;
        let mut packages = Vec::new();

        // Nix profile JSON structure: { "elements": [ { "storePaths": [...], "attrPath": "..." } ] }
        if let Some(elements) = json.get("elements").and_then(|e| e.as_array()) {
            for (i, el) in elements.iter().enumerate() {
                let attr_path = el.get("attrPath")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                
                // Clean the name (e.g., "legacyPackages.x86_64-linux.htop" -> "htop")
                let name = attr_path.split('.').last().unwrap_or(attr_path);
                
                let mut p = Package::new(name, "nix");
                // Store the index for precise removals
                p.properties.insert("index".into(), i.to_string());
                p.properties.insert("full_attr".into(), attr_path.to_string());

                if let Some(path) = el.get("storePaths").and_then(|a| a.as_array()).and_then(|a| a.first()) {
                    p.properties.insert("store_path".into(), path.as_str().unwrap_or_default().to_string());
                }

                packages.push(p);
            }
        }

        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // In the context of Nix Profile, all items added are considered 'manual'.
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

#[async_trait]
impl Upgradable for NixManager {
    async fn update(&self, _: bool) -> Result<()> {
        // 'nix flake update' handles registry refreshing, but 'nix profile upgrade' is the primary maintenance task.
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        info!("Nix: Upgrading all packages in user profile...");
        self.executor.run_exclusive("nix", "nix", &["profile", "upgrade", "--all"], false).await?;
        Ok(())
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        info!("Nix: Performing garbage collection (GC)...");
        // Nix GC often needs sudo to clean /nix/store
        self.executor.run("nix-collect-garbage", &["--delete-older-than", "30d"], sudo).await?;
        Ok(())
    }
}