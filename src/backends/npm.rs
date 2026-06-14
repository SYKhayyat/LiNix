// src/backends/npm.rs

use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec,
    Queryable, Result, Upgradable, MetadataProvider, Error
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use serde_json::Value;

/// Core backend implementation for npm (Node.js package manager).
#[derive(Clone)]
pub struct NpmBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl NpmBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "npm".to_string(),
        }
    }

    /// Returns the global npm prefix (installation root).
    async fn get_global_prefix(&self) -> Result<String> {
        let output = self.executor.run_output("npm", &["prefix", "-g"], false).await?;
        Ok(output.trim().to_string())
    }
}

#[async_trait]
impl BackendCore for NpmBackendCore {
    fn name(&self) -> &str { &self.name }
    fn is_available(&self) -> bool { self.executor.command_exists_sync("npm") }
    fn needs_root(&self) -> bool { false }
}

#[async_trait]
impl MetadataProvider for NpmBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct NpmInstallable {
    pub core: Arc<NpmBackendCore>,
}

#[async_trait]
impl Installable for NpmInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            info!("npm: Installing {} globally...", spec.name);
            self.core.executor.run_exclusive("npm", "npm", &["install", "-g", &spec.name], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("npm: Uninstalling {} globally...", name);
            self.core.executor.run_exclusive("npm", "npm", &["uninstall", "-g", name], false).await?;
        }
        Ok(())
    }
}

pub struct NpmQueryable {
    pub core: Arc<NpmBackendCore>,
}

#[async_trait]
impl Queryable for NpmQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("npm", &["list", "-g", "--depth=0", "--json"], false).await?;
        if output.is_empty() {
            return Ok(vec![]);
        }
        let json: Value = serde_json::from_str(&output).map_err(|e| Error::Other(format!("npm JSON error: {}", e)))?;
        let mut packages = Vec::new();
        if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
            for (name, info) in deps {
                let version = info.get("version").and_then(|v| v.as_str()).unwrap_or("unknown");
                packages.push(Package::with_version(name, version, "npm"));
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        if let Some(mut pkg) = all.into_iter().find(|p| p.name == name) {
            let prefix = self.core.get_global_prefix().await?;
            let install_path = format!("{}/lib/node_modules/{}", prefix, name);
            pkg.properties.insert("install_path".into(), install_path);
            Ok(Some(pkg))
        } else {
            Ok(None)
        }
    }
}

pub struct NpmUpgradable {
    pub core: Arc<NpmBackendCore>,
}

#[async_trait]
impl Upgradable for NpmUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("npm: Upgrading all global packages...");
        let installed = self.core.list_installed_internal().await?;
        for pkg in installed {
            let _ = self.core.executor.run_exclusive("npm", "npm", &["install", "-g", &pkg.name], false).await;
        }
        Ok(())
    }

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }
}

impl NpmBackendCore {
    async fn list_installed_internal(&self) -> Result<Vec<Package>> {
        let queryable = NpmQueryable { core: Arc::new(self.clone()) };
        queryable.list_installed().await
    }
}