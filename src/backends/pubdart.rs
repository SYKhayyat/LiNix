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
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    Result, Upgradable,
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

/// pub pins with a trailing positional version (`activate <pkg> <version>`), so the version
/// belongs behind the terminator with the name, not in front of it as a flag.
fn activate_argv(name: &str, version: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "pub".to_string(),
        "global".to_string(),
        "activate".to_string(),
    ];
    let names: Vec<&str> = std::iter::once(name).chain(version).collect();
    crate::core::argv::push_names(&mut args, "dart", names);
    args
}

fn deactivate_argv(name: &str) -> Vec<String> {
    let mut args = vec![
        "pub".to_string(),
        "global".to_string(),
        "deactivate".to_string(),
    ];
    crate::core::argv::push_names(&mut args, "dart", [name]);
    args
}

#[async_trait]
impl Installable for PubInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            let version = spec
                .options
                .get("version")
                .filter(|v| crate::backends::concrete_version(v))
                .map(String::as_str);
            let args = activate_argv(&spec.name, version);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            info!("pub: Activating {} globally...", spec.name);
            self.core
                .executor
                .run_exclusive("dart", "dart", &arg_refs, false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("pub: Deactivating {}...", name);
            let args = deactivate_argv(name);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("dart", "dart", &arg_refs, false)
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
            let args = activate_argv(&pkg.name, None);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let _ = self
                .core
                .executor
                .run_exclusive("dart", "dart", &arg_refs, false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::executor::MockExecutor;
    use dashmap::DashMap;

    #[test]
    fn the_name_and_its_pinned_version_both_sit_behind_the_terminator() {
        assert_eq!(
            activate_argv("webdev", None),
            ["pub", "global", "activate", "--", "webdev"]
        );
        assert_eq!(
            activate_argv("webdev", Some("2.7.0")),
            ["pub", "global", "activate", "--", "webdev", "2.7.0"]
        );
        assert_eq!(
            deactivate_argv("webdev"),
            ["pub", "global", "deactivate", "--", "webdev"]
        );
    }

    #[tokio::test]
    async fn activate_and_deactivate_end_their_options_before_the_name() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(false, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let core = Arc::new(PubBackendCore::new(exec));

        let spec = PackageSpec {
            name: "webdev".into(),
            backend: "pub".into(),
            ..Default::default()
        };
        PubInstallable { core: core.clone() }
            .install(&[spec], false)
            .await
            .unwrap();
        PubInstallable { core: core.clone() }
            .remove(&["webdev".to_string()], false)
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec![
                "dart pub global activate -- webdev",
                "dart pub global deactivate -- webdev",
            ]
        );
    }
}
