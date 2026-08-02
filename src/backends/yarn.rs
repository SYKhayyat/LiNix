use crate::backends::node_registry::registry_search;
use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;

/// Split a yarn tree label like `lodash@4.17.0` or `@scope/pkg@1.0.0` into
/// `(name, version)`. Splitting on the FIRST `@` breaks scoped packages (whose name
/// itself starts with `@`), so split on the LAST `@` instead. A leading `@` with no
/// later `@` means a scoped package with no version.
fn split_name_version(label: &str) -> (String, String) {
    match label.rsplit_once('@') {
        // `rsplit_once` on "@scope/pkg" would yield ("", "scope/pkg"); guard the
        // leading-scope-only case where the only `@` is at index 0.
        Some((name, ver)) if !name.is_empty() => (name.to_string(), ver.to_string()),
        _ => (label.to_string(), "unknown".to_string()),
    }
}

/// Extract packages from yarn v1's `global list --json` output. That output is NEWLINE-
/// DELIMITED JSON *events* (not a single document):
///   {"type":"progressStart",...}
///   {"type":"info","data":"..."}
///   {"type":"tree","data":{"type":"list","trees":[{"name":"cowsay@1.6.0"},...]}}
/// A single `from_str` over the whole blob errors on the trailing lines — which previously
/// propagated through `info()` into the remove gate and made `remove` exit 1. Parse each
/// line independently and pull names from any event carrying `data.trees[].name`.
fn parse_yarn_json_stream(output: &str) -> Vec<Package> {
    let mut packages = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(trees) = json
            .get("data")
            .and_then(|d| d.get("trees"))
            .and_then(|t| t.as_array())
        {
            for tree in trees {
                if let Some(name) = tree.get("name").and_then(|n| n.as_str()) {
                    let (pkg_name, version) = split_name_version(name);
                    packages.push(Package::with_version(&pkg_name, &version, "yarn"));
                }
            }
        }
    }
    packages
}

#[derive(Clone)]
pub struct YarnBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl YarnBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "yarn".to_string(),
        }
    }

    async fn get_global_prefix(&self) -> Result<String> {
        let output = self
            .executor
            .run_output("yarn", &["global", "dir"], false)
            .await?;
        Ok(output.trim().to_string())
    }

    /// Returns the global binary directory (`yarn global bin`), where executables are linked.
    async fn get_global_bin(&self) -> Result<String> {
        let output = self
            .executor
            .run_output("yarn", &["global", "bin"], false)
            .await?;
        Ok(output.trim().to_string())
    }
}

#[async_trait]
impl BackendCore for YarnBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("yarn")
    }
    fn probes(&self) -> Vec<String> {
        vec!["yarn".into()]
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for YarnBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct YarnInstallable {
    pub core: Arc<YarnBackendCore>,
}

fn global_argv(subcommand: &str, name: &str) -> Vec<String> {
    let mut args = vec!["global".to_string(), subcommand.to_string()];
    crate::core::argv::push_names(&mut args, "yarn", [name]);
    args
}

#[async_trait]
impl Installable for YarnInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            let target = match spec.options.get("version") {
                Some(v) if crate::backends::concrete_version(v) => format!("{}@{}", spec.name, v),
                _ => spec.name.clone(),
            };
            info!("Yarn: Installing {} globally...", target);
            let args = global_argv("add", &target);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("yarn", "yarn", &arg_refs, false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("Yarn: Uninstalling {} globally...", name);
            let args = global_argv("remove", name);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("yarn", "yarn", &arg_refs, false)
                .await?;
        }
        Ok(())
    }
}

pub struct YarnQueryable {
    pub core: Arc<YarnBackendCore>,
}

#[async_trait]
impl Queryable for YarnQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("yarn", &["global", "list", "--json"], false)
            .await?;
        let mut packages = parse_yarn_json_stream(&output);

        if packages.is_empty() {
            // Fallback to the human list, whose rows look like:
            //   info "cowsay@1.6.0" has binaries:
            // Extract the quoted "name@version" label rather than splitting the whole line.
            let plain = self
                .core
                .executor
                .run_output("yarn", &["global", "list"], false)
                .await?;
            for line in plain.lines() {
                let label = match (line.find('"'), line.rfind('"')) {
                    (Some(a), Some(b)) if b > a + 1 => &line[a + 1..b],
                    _ => continue,
                };
                if !label.contains('@') {
                    continue;
                }
                let (name, version) = split_name_version(label);
                packages.push(Package::with_version(name.trim(), version.trim(), "yarn"));
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
            // `yarn global dir` returns the folder CONTAINING node_modules, so the package
            // lives at `<dir>/node_modules/<name>`.
            let prefix = self.core.get_global_prefix().await?;
            pkg.properties.insert(
                "install_path".into(),
                format!("{}/node_modules/{}", prefix, name),
            );
            if let Ok(bin) = self.core.get_global_bin().await {
                pkg.properties.insert("bin_path".into(), bin);
            }
            Ok(Some(pkg))
        } else {
            Ok(None)
        }
    }
}

pub struct YarnSearchable {
    pub core: Arc<YarnBackendCore>,
}

#[async_trait]
impl Searchable for YarnSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // `yarn search` was removed in Yarn 2+ (Berry); resolve from the npm registry.
        registry_search(query, "yarn", 25).await
    }
}

pub struct YarnUpgradable {
    pub core: Arc<YarnBackendCore>,
}

#[async_trait]
impl Upgradable for YarnUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("Yarn: Upgrading all global packages...");
        let installed = self.core.list_installed_internal().await?;
        for pkg in installed {
            let args = global_argv("add", &pkg.name);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let _ = self
                .core
                .executor
                .run_exclusive("yarn", "yarn", &arg_refs, false)
                .await;
        }
        Ok(())
    }
}

impl YarnBackendCore {
    async fn list_installed_internal(&self) -> Result<Vec<Package>> {
        let queryable = YarnQueryable {
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
    let core = Arc::new(YarnBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(YarnInstallable { core: core.clone() }))
            .with_queryable(Arc::new(YarnQueryable { core: core.clone() }))
            .with_searchable(Arc::new(YarnSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(YarnUpgradable { core: core.clone() }))
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
    fn the_global_subcommand_precedes_the_terminator_and_the_name_follows_it() {
        assert_eq!(
            global_argv("add", "cowsay"),
            ["global", "add", "--", "cowsay"]
        );
        assert_eq!(
            global_argv("remove", "@scope/pkg"),
            ["global", "remove", "--", "@scope/pkg"]
        );
    }

    #[tokio::test]
    async fn install_and_remove_end_their_options_before_the_name() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(false, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let core = Arc::new(YarnBackendCore::new(exec));

        let spec = PackageSpec {
            name: "cowsay".into(),
            backend: "yarn".into(),
            ..Default::default()
        };
        YarnInstallable { core: core.clone() }
            .install(&[spec], false)
            .await
            .unwrap();
        YarnInstallable { core: core.clone() }
            .remove(&["cowsay".to_string()], false)
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec!["yarn global add -- cowsay", "yarn global remove -- cowsay"]
        );
    }

    #[test]
    fn ndjson_stream_yields_packages_and_ignores_noise() {
        // Real `yarn global list --json` shape: an NDJSON event stream, tree carries pkgs.
        let stream = concat!(
            "{\"type\":\"progressStart\",\"data\":{\"id\":0,\"total\":33}}\n",
            "{\"type\":\"info\",\"data\":\"\\\"cowsay@1.6.0\\\" has binaries:\"}\n",
            "{\"type\":\"tree\",\"data\":{\"type\":\"list\",\"trees\":[{\"name\":\"cowsay@1.6.0\"},{\"name\":\"@scope/tool@2.3.4\"}]}}\n",
        );
        let pkgs = parse_yarn_json_stream(stream);
        assert_eq!(pkgs.len(), 2, "must parse the tree event, ignore the rest");
        assert_eq!(pkgs[0].name, "cowsay");
        assert_eq!(pkgs[0].version.as_deref(), Some("1.6.0"));
        assert_eq!(pkgs[1].name, "@scope/tool"); // scoped name preserved
        assert_eq!(pkgs[1].version.as_deref(), Some("2.3.4"));
    }

    #[test]
    fn ndjson_empty_or_treeless_is_empty_not_error() {
        assert!(parse_yarn_json_stream("").is_empty());
        assert!(parse_yarn_json_stream("{\"type\":\"progressStart\",\"data\":{}}\n").is_empty());
    }

    #[test]
    fn parses_plain_and_scoped_names() {
        assert_eq!(
            split_name_version("lodash@4.17.0"),
            ("lodash".into(), "4.17.0".into())
        );
        // scoped package: name itself begins with '@'
        assert_eq!(
            split_name_version("@scope/pkg@1.0.0"),
            ("@scope/pkg".into(), "1.0.0".into())
        );
        // scoped, no version
        assert_eq!(
            split_name_version("@scope/pkg"),
            ("@scope/pkg".into(), "unknown".into())
        );
        // plain, no version
        assert_eq!(
            split_name_version("ripgrep"),
            ("ripgrep".into(), "unknown".into())
        );
    }
}
