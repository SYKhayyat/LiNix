use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Upgradable,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct PipxBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl PipxBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor: executor.with_exit_policy(crate::core::exit_policy::for_manager("pipx")),
            name: "pipx".to_string(),
        }
    }

    async fn get_pipx_home(&self) -> Result<String> {
        let output = self
            .executor
            .run_output("pipx", &["environment", "--value", "PIPX_HOME"], false)
            .await
            .or_else(|_| {
                let home = dirs::home_dir()
                    .ok_or_else(|| Error::Other("Could not determine home directory".into()))?;
                Ok::<String, Error>(home.join(".local/pipx").to_string_lossy().to_string())
            })?;
        Ok(output.trim().to_string())
    }
}

#[async_trait]
impl BackendCore for PipxBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("pipx")
    }
    fn probes(&self) -> Vec<String> {
        vec!["pipx".into()]
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for PipxBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct PipxInstallable {
    pub core: Arc<PipxBackendCore>,
}

fn pipx_argv(subcommand: &str, name: &str) -> Vec<String> {
    let mut args = vec![subcommand.to_string()];
    crate::core::argv::push_names(&mut args, "pipx", [name]);
    args
}

#[async_trait]
impl Installable for PipxInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            // pipx accepts a pip requirement spec: pin with `name==version`.
            let target = match spec.options.get("version") {
                Some(v) if crate::backends::concrete_version(v) => format!("{}=={}", spec.name, v),
                _ => spec.name.clone(),
            };
            info!("pipx: Installing {}...", target);
            let args = pipx_argv("install", &target);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("pipx", "pipx", &arg_refs, false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("pipx: Uninstalling {}...", name);
            let args = pipx_argv("uninstall", name);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("pipx", "pipx", &arg_refs, false)
                .await?;
        }
        Ok(())
    }
}

pub struct PipxQueryable {
    pub core: Arc<PipxBackendCore>,
}

#[async_trait]
impl Queryable for PipxQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("pipx", &["list", "--json"], false)
            .await?;
        if output.is_empty() {
            return Ok(vec![]);
        }
        let json: Value = serde_json::from_str(&output)
            .map_err(|e| Error::Other(format!("pipx JSON error: {}", e)))?;
        let mut packages = Vec::new();
        if let Some(venvs) = json.get("venvs").and_then(|v| v.as_object()) {
            for (name, data) in venvs {
                let version = data
                    .get("metadata")
                    .and_then(|m| m.get("main_package"))
                    .and_then(|p| p.get("package_version"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                packages.push(Package::with_version(name, version, "pipx"));
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
            let pipx_home = self.core.get_pipx_home().await?;
            let venv_path = format!("{}/venvs/{}", pipx_home, name);
            pkg.properties.insert("install_path".into(), venv_path);
            Ok(Some(pkg))
        } else {
            Ok(None)
        }
    }
}

pub struct PipxUpgradable {
    pub core: Arc<PipxBackendCore>,
}

#[async_trait]
impl Upgradable for PipxUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("pipx: Upgrading all installed packages...");
        self.core
            .executor
            .run_exclusive("pipx", "pipx", &["upgrade-all"], false)
            .await?;
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(PipxBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(PipxInstallable { core: core.clone() }))
            .with_queryable(Arc::new(PipxQueryable { core: core.clone() }))
            .with_upgradable(Arc::new(PipxUpgradable { core: core.clone() }))
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
    fn the_subcommand_precedes_the_terminator_and_the_name_follows_it() {
        assert_eq!(pipx_argv("install", "black"), ["install", "--", "black"]);
        assert_eq!(
            pipx_argv("uninstall", "black==24.1.0"),
            ["uninstall", "--", "black==24.1.0"]
        );
    }

    #[tokio::test]
    async fn install_and_remove_end_their_options_before_the_name() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(false, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let core = Arc::new(PipxBackendCore::new(exec));

        let spec = PackageSpec {
            name: "black".into(),
            backend: "pipx".into(),
            ..Default::default()
        };
        PipxInstallable { core: core.clone() }
            .install(&[spec], false)
            .await
            .unwrap();
        PipxInstallable { core: core.clone() }
            .remove(&["black".to_string()], false)
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec!["pipx install -- black", "pipx uninstall -- black"]
        );
    }
}
