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

    /// Returns the global `node_modules` directory (`pnpm root -g`), which is where each
    /// global package's own folder lives.
    async fn get_global_prefix(&self) -> Result<String> {
        let output = self
            .executor
            .run_output("pnpm", &["root", "-g"], false)
            .await?;
        Ok(output.trim().to_string())
    }

    /// Returns the global bin directory (`pnpm bin -g`), where executables are linked.
    async fn get_global_bin(&self) -> Result<String> {
        let output = self
            .executor
            .run_output("pnpm", &["bin", "-g"], false)
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
    fn probes(&self) -> Vec<String> {
        vec!["pnpm".into()]
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

fn global_argv(subcommand: &str, name: &str) -> Vec<String> {
    let mut args = vec![subcommand.to_string(), "-g".to_string()];
    crate::core::argv::push_names(&mut args, "pnpm", [name]);
    args
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
            let args = global_argv("add", &target);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("pnpm", "pnpm", &arg_refs, false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("pnpm: Uninstalling {} globally...", name);
            let args = global_argv("remove", name);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("pnpm", "pnpm", &arg_refs, false)
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
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
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
            // `pnpm root -g` already returns the global node_modules dir, so the package's
            // folder is `<root>/<name>`; appending another `node_modules` yields a path
            // that does not exist.
            let prefix = self.core.get_global_prefix().await?;
            pkg.properties
                .insert("install_path".into(), format!("{}/{}", prefix, name));
            if let Ok(bin) = self.core.get_global_bin().await {
                pkg.properties.insert("bin_path".into(), bin);
            }
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
            let args = global_argv("add", &pkg.name);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let _ = self
                .core
                .executor
                .run_exclusive("pnpm", "pnpm", &arg_refs, false)
                .await;
        }
        Ok(())
    }

    async fn clean_cache(&self, _sudo: bool) -> Result<()> {
        info!("pnpm: Pruning the global store...");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::executor::MockExecutor;
    use dashmap::DashMap;

    #[test]
    fn the_global_flag_precedes_the_terminator_and_the_name_follows_it() {
        assert_eq!(global_argv("add", "cowsay"), ["add", "-g", "--", "cowsay"]);
        assert_eq!(
            global_argv("remove", "@scope/pkg"),
            ["remove", "-g", "--", "@scope/pkg"]
        );
    }

    #[tokio::test]
    async fn install_and_remove_end_their_options_before_the_name() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(false, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let core = Arc::new(PnpmBackendCore::new(exec));

        let spec = PackageSpec {
            name: "cowsay".into(),
            backend: "pnpm".into(),
            ..Default::default()
        };
        PnpmInstallable { core: core.clone() }
            .install(&[spec], false)
            .await
            .unwrap();
        PnpmInstallable { core: core.clone() }
            .remove(&["cowsay".to_string()], false)
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec!["pnpm add -g -- cowsay", "pnpm remove -g -- cowsay"]
        );
    }
}
