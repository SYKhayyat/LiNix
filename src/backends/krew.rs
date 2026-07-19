// krew, the kubectl plugin manager, exposed as the `krew` backend. Its commands are
// subcommands of `kubectl` (`kubectl krew install ...`), so it is a small dedicated
// backend rather than a generic config entry. Runtime-gated on the `kubectl` binary.
//
//   * install — `kubectl krew install <plugin>`
//   * remove  — `kubectl krew uninstall <plugin>`
//   * list    — `kubectl krew list`
//   * search  — `kubectl krew search <query>`
//   * upgrade — `kubectl krew upgrade`; update refreshes the plugin index

use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct KrewBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl KrewBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "krew".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for KrewBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("kubectl")
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for KrewBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct KrewInstallable {
    pub core: Arc<KrewBackendCore>,
}

#[async_trait]
impl Installable for KrewInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        // krew installs the index's current version; it has no per-install version pin.
        for spec in specs {
            info!("krew: Installing {}...", spec.name);
            self.core
                .executor
                .run_exclusive(
                    "kubectl",
                    "kubectl",
                    &["krew", "install", &spec.name],
                    false,
                )
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("krew: Uninstalling {}...", name);
            self.core
                .executor
                .run_exclusive("kubectl", "kubectl", &["krew", "uninstall", name], false)
                .await?;
        }
        Ok(())
    }
}

pub struct KrewQueryable {
    pub core: Arc<KrewBackendCore>,
}

impl KrewQueryable {
    async fn scan(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("kubectl", &["krew", "list"], false)
            .await?;
        // `kubectl krew list` prints `PLUGIN  VERSION` rows (older versions: bare names).
        Ok(crate::parsers::ecosystem::ws_name_version(&output, "krew"))
    }
}

#[async_trait]
impl Queryable for KrewQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        self.scan().await
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.scan().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        Ok(self.scan().await?.into_iter().find(|p| p.name == name))
    }
}

pub struct KrewSearchable {
    pub core: Arc<KrewBackendCore>,
}

#[async_trait]
impl Searchable for KrewSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("kubectl", &["krew", "search", query], false)
            .await?;
        // Output is `NAME  DESCRIPTION  INSTALLED`; take the plugin name (first column).
        Ok(crate::parsers::ecosystem::names_only(&output, "krew"))
    }
}

pub struct KrewUpgradable {
    pub core: Arc<KrewBackendCore>,
}

#[async_trait]
impl Upgradable for KrewUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        self.core
            .executor
            .run("kubectl", &["krew", "update"], false)
            .await?;
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("krew: Upgrading all installed plugins...");
        self.core
            .executor
            .run_exclusive("kubectl", "kubectl", &["krew", "upgrade"], false)
            .await?;
        Ok(())
    }

}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(KrewBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(KrewInstallable { core: core.clone() }))
            .with_queryable(Arc::new(KrewQueryable { core: core.clone() }))
            .with_searchable(Arc::new(KrewSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(KrewUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
