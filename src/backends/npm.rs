use crate::backends::node_registry::registry_search;
use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;

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

    async fn get_global_prefix(&self) -> Result<String> {
        let output = self
            .executor
            .run_output("npm", &["prefix", "-g"], false)
            .await?;
        Ok(output.trim().to_string())
    }
}

#[async_trait]
impl BackendCore for NpmBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("npm")
    }
    fn needs_root(&self) -> bool {
        false
    }
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
            let target = match spec.options.get("version") {
                Some(v) if crate::backends::concrete_version(v) => format!("{}@{}", spec.name, v),
                _ => spec.name.clone(),
            };
            info!("npm: Installing {} globally...", target);
            self.core
                .executor
                .run_exclusive("npm", "npm", &["install", "-g", &target], false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("npm: Uninstalling {} globally...", name);
            self.core
                .executor
                .run_exclusive("npm", "npm", &["uninstall", "-g", name], false)
                .await?;
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
        let output = self
            .core
            .executor
            .run_output("npm", &["list", "-g", "--depth=0", "--json"], false)
            .await?;
        if output.is_empty() {
            return Ok(vec![]);
        }
        let json: Value = serde_json::from_str(&output)
            .map_err(|e| Error::Other(format!("npm JSON error: {}", e)))?;
        let mut packages = Vec::new();
        if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
            for (name, info) in deps {
                let version = info
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
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
            // Global module layout differs by OS: POSIX puts them under
            // `<prefix>/lib/node_modules`, Windows directly under `<prefix>/node_modules`.
            let base = std::path::Path::new(&prefix);
            let install_path = if cfg!(windows) {
                base.join("node_modules").join(name)
            } else {
                base.join("lib").join("node_modules").join(name)
            };
            pkg.properties.insert(
                "install_path".into(),
                install_path.to_string_lossy().to_string(),
            );
            Ok(Some(pkg))
        } else {
            Ok(None)
        }
    }
}

pub struct NpmSearchable {
    pub core: Arc<NpmBackendCore>,
}

#[async_trait]
impl Searchable for NpmSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        registry_search(query, "npm", 25).await
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
            let _ = self
                .core
                .executor
                .run_exclusive("npm", "npm", &["install", "-g", &pkg.name], false)
                .await;
        }
        Ok(())
    }

}

impl NpmBackendCore {
    async fn list_installed_internal(&self) -> Result<Vec<Package>> {
        let queryable = NpmQueryable {
            core: Arc::new(self.clone()),
        };
        queryable.list_installed().await
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(NpmBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(NpmInstallable { core: core.clone() }))
            .with_queryable(Arc::new(NpmQueryable { core: core.clone() }))
            .with_searchable(Arc::new(NpmSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(NpmUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
