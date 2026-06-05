use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Upgradable, Error, MetadataProvider
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for Nix (via 'nix profile').
pub struct NixBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl NixBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "nix".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for NixBackendCore {
    fn name(&self) -> &str { &self.name }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("nix")
    }

    fn needs_root(&self) -> bool {
        // Nix profiles are managed per-user in the nix store; usually doesn't require sudo.
        false
    }
}

/// Phase 1.1: MetadataProvider for Nix.
#[async_trait]
impl MetadataProvider for NixBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        // Nix handles its own dependency tree internally during 'nix profile install'.
        // We return an empty list as we don't need to manually orchestrate nix-native deps.
        Ok(vec![])
    }
}

pub struct NixInstallable {
    pub core: Arc<NixBackendCore>,
}

#[async_trait]
impl Installable for NixInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let flake_uri = if spec.name.contains('#') {
                spec.name.clone()
            } else {
                format!("nixpkgs#{}", spec.name)
            };

            info!("Nix: Installing {} to user profile...", flake_uri);
            self.core.executor.run_exclusive("nix", "nix", &["profile", "install", &flake_uri], sudo).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        let installed = self.core.list_installed_internal().await?;
        
        for name in names {
            if let Some(pkg) = installed.iter().find(|p| p.name == *name) {
                if let Some(index) = pkg.properties.get("index") {
                    info!("Nix: Removing package at profile index {} ({})", index, name);
                    self.core.executor.run_exclusive("nix", "nix", &["profile", "remove", index], sudo).await?;
                } else {
                    self.core.executor.run_exclusive("nix", "nix", &["profile", "remove", name], sudo).await?;
                }
            }
        }
        Ok(())
    }
}

pub struct NixQueryable {
    pub core: Arc<NixBackendCore>,
}

#[async_trait]
impl Queryable for NixQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        self.core.list_installed_internal().await
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct NixUpgradable {
    pub core: Arc<NixBackendCore>,
}

#[async_trait]
impl Upgradable for NixUpgradable {
    async fn update(&self, _: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Nix: Upgrading all packages in user profile...");
        self.core.executor.run_exclusive("nix", "nix", &["profile", "upgrade", "--all"], sudo).await?;
        Ok(())
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        info!("Nix: Performing garbage collection (GC)...");
        self.core.executor.run("nix-collect-garbage", &["--delete-older-than", "30d"], sudo).await?;
        Ok(())
    }
}

impl NixBackendCore {
    /// Internal helper to parse the complex JSON output of 'nix profile list'.
    async fn list_installed_internal(&self) -> Result<Vec<Package>> {
        let output = self.executor.run_output("nix", &["profile", "list", "--json"], false).await?;
        if output.is_empty() || output == "{}" { return Ok(vec![]); }

        let json: Value = serde_json::from_str(&output).map_err(|e| Error::Other(format!("Nix JSON error: {}", e)))?;
        let mut packages = Vec::new();

        if let Some(elements) = json.get("elements").and_then(|e| e.as_array()) {
            for (i, el) in elements.iter().enumerate() {
                let attr_path = el.get("attrPath")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                
                let name = attr_path.split('.').last().unwrap_or(attr_path);
                
                let mut p = Package::new(name, "nix");
                p.properties.insert("index".into(), i.to_string());
                p.properties.insert("full_attr".into(), attr_path.to_string());

                if let Some(store_paths) = el.get("storePaths").and_then(|a| a.as_array()) {
                    if let Some(first_path) = store_paths.first().and_then(|p| p.as_str()) {
                        p.properties.insert("store_path".into(), first_path.to_string());
                    }
                }

                packages.push(p);
            }
        }

        Ok(packages)
    }
}