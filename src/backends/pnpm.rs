// src/backends/pnpm.rs

use crate::backends::node_registry::registry_search;
use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for pnpm (fast, disk-efficient Node.js package manager).
#[derive(Clone)]
pub struct PnpmBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl PnpmBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "pnpm".to_string(),
        }
    }

    /// Returns the global pnpm store path.
    #[allow(dead_code)]
    async fn get_global_store(&self) -> Result<String> {
        let output = self
            .executor
            .run_output("pnpm", &["store", "path"], false)
            .await
            .or_else(|_| {
                let home = dirs::home_dir()
                    .ok_or_else(|| Error::Other("Could not determine home directory".into()))?;
                Ok::<String, Error>(
                    home.join(".local/share/pnpm/store")
                        .to_string_lossy()
                        .to_string(),
                )
            })?;
        Ok(output.trim().to_string())
    }

    /// Returns the global installation prefix (where global binaries are linked).
    async fn get_global_prefix(&self) -> Result<String> {
        let output = self
            .executor
            .run_output("pnpm", &["root", "-g"], false)
            .await?;
        Ok(output.trim().to_string())
    }
}

#[async_trait]
impl BackendCore for PnpmBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("pnpm")
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for PnpmBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct PnpmInstallable {
    pub core: Arc<PnpmBackendCore>,
}

#[async_trait]
impl Installable for PnpmInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            let target = match spec.options.get("version") {
                Some(v) if crate::backends::concrete_version(v) => format!("{}@{}", spec.name, v),
                _ => spec.name.clone(),
            };
            info!("pnpm: Installing {} globally...", target);
            self.core
                .executor
                .run_exclusive("pnpm", "pnpm", &["add", "-g", &target], false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("pnpm: Uninstalling {} globally...", name);
            self.core
                .executor
                .run_exclusive("pnpm", "pnpm", &["remove", "-g", name], false)
                .await?;
        }
        Ok(())
    }
}

pub struct PnpmQueryable {
    pub core: Arc<PnpmBackendCore>,
}

#[async_trait]
impl Queryable for PnpmQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("pnpm", &["list", "-g", "--depth=0", "--json"], false)
            .await?;
        if output.is_empty() {
            return Ok(vec![]);
        }
        let json: Value = serde_json::from_str(&output)
            .map_err(|e| Error::Other(format!("pnpm JSON error: {}", e)))?;
        let mut packages = Vec::new();
        // `pnpm list -g --json` returns an ARRAY of project objects
        // (`[{"dependencies":{...}}]`), not a bare object — so iterate entries and collect
        // each one's dependency map, else every global package parses as empty.
        let entries: Vec<&Value> = match &json {
            Value::Array(items) => items.iter().collect(),
            other => vec![other],
        };
        for entry in entries {
            if let Some(deps) = entry.get("dependencies").and_then(|d| d.as_object()) {
                for (name, info) in deps {
                    let version = info
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    packages.push(Package::with_version(name, version, "pnpm"));
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

pub struct PnpmSearchable {
    pub core: Arc<PnpmBackendCore>,
}

#[async_trait]
impl Searchable for PnpmSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // pnpm has no `search` subcommand; it resolves from the npm registry.
        registry_search(query, "pnpm", 25).await
    }
}

pub struct PnpmUpgradable {
    pub core: Arc<PnpmBackendCore>,
}

#[async_trait]
impl Upgradable for PnpmUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("pnpm: Upgrading all global packages...");
        let installed = self.core.list_installed_internal().await?;
        for pkg in installed {
            let _ = self
                .core
                .executor
                .run_exclusive("pnpm", "pnpm", &["add", "-g", &pkg.name], false)
                .await;
        }
        Ok(())
    }

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        info!("pnpm: Cleaning global store orphans...");
        self.core
            .executor
            .run("pnpm", &["store", "prune"], false)
            .await?;
        Ok(())
    }
}

impl PnpmBackendCore {
    async fn list_installed_internal(&self) -> Result<Vec<Package>> {
        let queryable = PnpmQueryable {
            core: Arc::new(self.clone()),
        };
        queryable.list_installed().await
    }
}

/// Build and register the pnpm backend with all its capabilities.
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(PnpmBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(PnpmInstallable { core: core.clone() }))
            .with_queryable(Arc::new(PnpmQueryable { core: core.clone() }))
            .with_searchable(Arc::new(PnpmSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(PnpmUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
