// src/backends/cargo.rs

use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec,
    Queryable, Result, Upgradable, MetadataProvider, Error
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for Cargo (Rust package manager).
#[derive(Clone)]
pub struct CargoBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl CargoBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "cargo".to_string(),
        }
    }

    /// Returns the path to Cargo's global installation directory.
    async fn get_cargo_root(&self) -> Result<String> {
        match std::env::var("CARGO_HOME") {
            Ok(home) => Ok(home),
            Err(_) => {
                let user_home = dirs::home_dir()
                    .ok_or_else(|| Error::Other("Could not determine home directory".into()))?;
                Ok(user_home.join(".cargo").to_string_lossy().to_string())
            }
        }
    }
}

#[async_trait]
impl BackendCore for CargoBackendCore {
    fn name(&self) -> &str { &self.name }
    fn is_available(&self) -> bool { self.executor.command_exists_sync("cargo") }
    fn needs_root(&self) -> bool { false }
}

#[async_trait]
impl MetadataProvider for CargoBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct CargoInstallable {
    pub core: Arc<CargoBackendCore>,
}

#[async_trait]
impl Installable for CargoInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            info!("Cargo: Installing {}...", spec.name);
            self.core.executor.run_exclusive("cargo", "cargo", &["install", &spec.name], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("Cargo: Uninstalling {}...", name);
            self.core.executor.run_exclusive("cargo", "cargo", &["uninstall", name], false).await?;
        }
        Ok(())
    }
}

pub struct CargoQueryable {
    pub core: Arc<CargoBackendCore>,
}

#[async_trait]
impl Queryable for CargoQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("cargo", &["install", "--list"], false).await?;
        let mut packages = Vec::new();
        for line in output.lines() {
            if let Some((name, rest)) = line.split_once(' ') {
                let version = rest.trim_start_matches('v').trim_end_matches(':');
                packages.push(Package::with_version(name, version, "cargo"));
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
            let cargo_root = self.core.get_cargo_root().await?;
            let bin_path = format!("{}/bin/{}", cargo_root, name);
            if std::path::Path::new(&bin_path).exists() || self.core.executor.dry_run {
                pkg.properties.insert("install_path".into(), cargo_root);
                pkg.properties.insert("bin_path".into(), bin_path);
            }
            Ok(Some(pkg))
        } else {
            Ok(None)
        }
    }
}

pub struct CargoUpgradable {
    pub core: Arc<CargoBackendCore>,
}

#[async_trait]
impl Upgradable for CargoUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("Cargo: Upgrading all installed packages...");
        let installed = self.core.list_installed_internal().await?;
        for pkg in installed {
            let _ = self.core.executor.run_exclusive("cargo", "cargo", &["install", &pkg.name, "--force"], false).await;
        }
        Ok(())
    }

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }
}

impl CargoBackendCore {
    async fn list_installed_internal(&self) -> Result<Vec<Package>> {
        let queryable = CargoQueryable { core: Arc::new(self.clone()) };
        queryable.list_installed().await
    }
}