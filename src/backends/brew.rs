// src/backends/brew.rs

use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec,
    Queryable, Result, Upgradable, MetadataProvider, Error
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;
use serde_json::Value;

/// Core backend implementation for Homebrew.
pub struct BrewBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl BrewBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "brew".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for BrewBackendCore {
    fn name(&self) -> &str { &self.name }
    fn is_available(&self) -> bool { self.executor.command_exists_sync("brew") }
    fn needs_root(&self) -> bool { false }
}

#[async_trait]
impl MetadataProvider for BrewBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let output = self.executor.run_output("brew", &["deps", name], false).await?;
        Ok(output.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
    }
}

pub struct BrewInstallable {
    pub core: Arc<BrewBackendCore>,
}

#[async_trait]
impl Installable for BrewInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            info!("Brew: Installing {}...", spec.name);
            self.core.executor.run_exclusive("brew", "brew", &["install", &spec.name], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("Brew: Uninstalling {}...", name);
            self.core.executor.run_exclusive("brew", "brew", &["uninstall", name], false).await?;
        }
        Ok(())
    }
}

pub struct BrewQueryable {
    pub core: Arc<BrewBackendCore>,
}

#[async_trait]
impl Queryable for BrewQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("brew", &["list", "--versions"], false).await?;
        let mut packages = Vec::new();
        for line in output.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                packages.push(Package::with_version(parts[0], parts[1], "brew"));
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("brew", &["leaves"], false).await?;
        Ok(output.lines().map(|l| Package::new(l.trim(), "brew")).collect())
    }

    /// Enhanced info: uses `brew info --json=v1` to get install path.
    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let output = self.core.executor.run_output("brew", &["info", "--json=v1", name], false).await?;
        if output.is_empty() || output == "[]" {
            return Ok(None);
        }
        let json: Value = serde_json::from_str(&output).map_err(|e| Error::Other(format!("Brew JSON error: {}", e)))?;
        let arr = json.as_array().ok_or_else(|| Error::Other("Expected array".into()))?;
        if arr.is_empty() { return Ok(None); }
        let first = &arr[0];
        let pkg_name = first["name"].as_str().unwrap_or(name).to_string();
        let version = first["versions"]["stable"].as_str().map(|s| s.to_string());
        let mut pkg = Package::with_version(&pkg_name, version.as_deref().unwrap_or("unknown"), "brew");
        if let Some(installed) = first["installed"].as_array().and_then(|a| a.first()) {
            if let Some(path) = installed["installed_as_dependency"].as_bool() {
                pkg.properties.insert("installed_as_dependency".into(), path.to_string());
            }
            // The install path is the prefix of the installed keg
            if let Some(prefix) = installed["prefix"].as_str() {
                pkg.properties.insert("install_path".into(), prefix.to_string());
            }
        }
        // Fallback: use the cellar path
        if !pkg.properties.contains_key("install_path") {
            if let Some(cellar) = first["cellar"].as_str() {
                pkg.properties.insert("install_path".into(), format!("{}/{}", cellar, pkg_name));
            }
        }
        Ok(Some(pkg))
    }
}

pub struct BrewUpgradable {
    pub core: Arc<BrewBackendCore>,
}

#[async_trait]
impl Upgradable for BrewUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        self.core.executor.run("brew", &["update"], false).await?;
        Ok(())
    }
    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        self.core.executor.run_exclusive("brew", "brew", &["upgrade"], false).await?;
        Ok(())
    }
    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        self.core.executor.run("brew", &["autoremove"], false).await?;
        Ok(())
    }
}