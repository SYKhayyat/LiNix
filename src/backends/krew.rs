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
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    Result, Searchable, Upgradable,
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
    /// Both binaries, not just `kubectl`. krew is a *plugin*: `kubectl krew …` only works
    /// because krew installs `kubectl-krew` on PATH. A host with kubectl and no krew
    /// reported this backend READY and then failed every command with `unknown command
    /// "krew"` — including `linix update`, which refreshes every backend at once.
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("kubectl")
            && self.executor.command_exists_sync("kubectl-krew")
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
            let mut args = vec!["krew".to_string(), "install".to_string()];
            crate::core::argv::push_names(&mut args, "kubectl", [&spec.name]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("kubectl", "kubectl", &arg_refs, false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("krew: Uninstalling {}...", name);
            let mut args = vec!["krew".to_string(), "uninstall".to_string()];
            crate::core::argv::push_names(&mut args, "kubectl", [name]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("kubectl", "kubectl", &arg_refs, false)
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
        let mut args = vec!["krew".to_string(), "search".to_string()];
        crate::core::argv::push_names(&mut args, "kubectl", [query]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self
            .core
            .executor
            .search_output("kubectl", &arg_refs, false)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn krew_plugin_names_come_after_the_terminator() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(KrewBackendCore::new(exec));

        KrewInstallable { core: core.clone() }
            .install(
                &[PackageSpec {
                    name: "ctx".into(),
                    backend: "krew".into(),
                    ..Default::default()
                }],
                false,
            )
            .await
            .unwrap();
        KrewInstallable { core: core.clone() }
            .remove(&["ctx".to_string()], false)
            .await
            .unwrap();
        KrewSearchable { core: core.clone() }
            .search("ctx")
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec![
                "kubectl krew install -- ctx",
                "kubectl krew uninstall -- ctx",
                "kubectl krew search -- ctx",
            ]
        );
    }
}
