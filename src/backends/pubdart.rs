// Dart/Flutter global packages (pub.dev), exposed as the `pub` backend. The user-facing
// command is a subcommand of the `dart` binary (`dart pub global ...`), so this is a small
// dedicated backend rather than a generic config entry (whose invoked binary must equal
// the backend id). Runtime-gated on the `dart` binary being present.
//
//   * install — `dart pub global activate <pkg> [version]`
//   * remove  — `dart pub global deactivate <pkg>`
//   * list    — `dart pub global list`  (rows: `name version`)
//   * upgrade — reactivate each package (pulls the newest allowed)
//   * search  — unsupported (pub.dev has no CLI search)

use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Upgradable,
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct PubBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl PubBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "pub".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for PubBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("dart")
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for PubBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct PubInstallable {
    pub core: Arc<PubBackendCore>,
}

#[async_trait]
impl Installable for PubInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            let mut args = vec!["pub", "global", "activate", spec.name.as_str()];
            // pub pins with a trailing positional version: `activate <pkg> <version>`.
            if let Some(v) = spec
                .options
                .get("version")
                .filter(|v| crate::backends::concrete_version(v))
            {
                args.push(v.as_str());
            }
            info!("pub: Activating {} globally...", spec.name);
            self.core
                .executor
                .run_exclusive("dart", "dart", &args, false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("pub: Deactivating {}...", name);
            self.core
                .executor
                .run_exclusive(
                    "dart",
                    "dart",
                    &["pub", "global", "deactivate", name],
                    false,
                )
                .await?;
        }
        Ok(())
    }
}

pub struct PubQueryable {
    pub core: Arc<PubBackendCore>,
}

impl PubQueryable {
    async fn scan(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("dart", &["pub", "global", "list"], false)
            .await?;
        Ok(crate::parsers::ecosystem::ws_name_version(&output, "pub"))
    }
}

#[async_trait]
impl Queryable for PubQueryable {
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

pub struct PubUpgradable {
    pub core: Arc<PubBackendCore>,
}

#[async_trait]
impl Upgradable for PubUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("pub: Reactivating all global packages...");
        let q = PubQueryable {
            core: self.core.clone(),
        };
        for pkg in q.scan().await? {
            let _ = self
                .core
                .executor
                .run_exclusive(
                    "dart",
                    "dart",
                    &["pub", "global", "activate", &pkg.name],
                    false,
                )
                .await;
        }
        Ok(())
    }

}

/// Search is omitted: pub.dev has no CLI search.
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(PubBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(PubInstallable { core: core.clone() }))
            .with_queryable(Arc::new(PubQueryable { core: core.clone() }))
            .with_upgradable(Arc::new(PubUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
