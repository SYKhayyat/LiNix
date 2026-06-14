// src/backends/yarn.rs

use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec,
    Queryable, Result, Upgradable, MetadataProvider, Error
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use serde_json::Value;

/// Core backend implementation for Yarn (Node.js package manager alternative).
#[derive(Clone)]
pub struct YarnBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl YarnBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "yarn".to_string(),
        }
    }

    /// Returns the global installation prefix (where global packages are stored).
    async fn get_global_prefix(&self) -> Result<String> {
        let output = self.executor.run_output("yarn", &["global", "dir"], false).await?;
        Ok(output.trim().to_string())
    }

    /// Returns the global binary directory.
	    #[allow(dead_code)]
    async fn get_global_bin(&self) -> Result<String> {
        let output = self.executor.run_output("yarn", &["global", "bin"], false).await?;
        Ok(output.trim().to_string())
    }
}

#[async_trait]
impl BackendCore for YarnBackendCore {
    fn name(&self) -> &str { &self.name }
    fn is_available(&self) -> bool { self.executor.command_exists_sync("yarn") }
    fn needs_root(&self) -> bool { false }
}

#[async_trait]
impl MetadataProvider for YarnBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct YarnInstallable {
    pub core: Arc<YarnBackendCore>,
}

#[async_trait]
impl Installable for YarnInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            info!("Yarn: Installing {} globally...", spec.name);
            self.core.executor.run_exclusive("yarn", "yarn", &["global", "add", &spec.name], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("Yarn: Uninstalling {} globally...", name);
            self.core.executor.run_exclusive("yarn", "yarn", &["global", "remove", name], false).await?;
        }
        Ok(())
    }
}

pub struct YarnQueryable {
    pub core: Arc<YarnBackendCore>,
}

#[async_trait]
impl Queryable for YarnQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("yarn", &["global", "list", "--json"], false).await?;
        if output.is_empty() {
            return Ok(vec![]);
        }
        let json: Value = serde_json::from_str(&output).map_err(|e| Error::Other(format!("Yarn JSON error: {}", e)))?;
        let mut packages = Vec::new();

        if let Some(data) = json.get("data").and_then(|d| d.as_object()) {
            if let Some(trees) = data.get("trees").and_then(|t| t.as_array()) {
                for tree in trees {
                    if let Some(name) = tree.get("name").and_then(|n| n.as_str()) {
                        let parts: Vec<&str> = name.split('@').collect();
                        let pkg_name = parts[0];
                        let version = parts.get(1).unwrap_or(&"unknown");
                        packages.push(Package::with_version(pkg_name, version, "yarn"));
                    }
                }
            }
        }

        if packages.is_empty() {
            let plain = self.core.executor.run_output("yarn", &["global", "list"], false).await?;
            for line in plain.lines() {
                if let Some((name, version)) = line.split_once('@') {
                    packages.push(Package::with_version(name.trim(), version.trim(), "yarn"));
                }
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
            let install_path = format!("{}/node_modules/{}", prefix, name);
            pkg.properties.insert("install_path".into(), install_path);
            Ok(Some(pkg))
        } else {
            Ok(None)
        }
    }
}

pub struct YarnUpgradable {
    pub core: Arc<YarnBackendCore>,
}

#[async_trait]
impl Upgradable for YarnUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("Yarn: Upgrading all global packages...");
        let installed = self.core.list_installed_internal().await?;
        for pkg in installed {
            let _ = self.core.executor.run_exclusive("yarn", "yarn", &["global", "add", &pkg.name], false).await;
        }
        Ok(())
    }

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }
}

impl YarnBackendCore {
    async fn list_installed_internal(&self) -> Result<Vec<Package>> {
        let queryable = YarnQueryable { core: Arc::new(self.clone()) };
        queryable.list_installed().await
    }
}