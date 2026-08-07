use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct MiseBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl MiseBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "mise".to_string(),
        }
    }

    /// Resolve mise's data directory in a cross-platform way. `mise path` is not a real
    /// subcommand, so we honor `MISE_DATA_DIR`, then fall back to the platform default:
    /// `%LOCALAPPDATA%\mise` on Windows, `~/.local/share/mise` on Unix/macOS.
    fn mise_data_dir(&self) -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("MISE_DATA_DIR") {
            if !dir.is_empty() {
                return Ok(PathBuf::from(dir));
            }
        }
        let base = if cfg!(windows) {
            dirs::data_local_dir()
        } else {
            dirs::home_dir().map(|h| h.join(".local").join("share"))
        };
        base.map(|p| p.join("mise"))
            .ok_or_else(|| Error::Other("Could not determine mise data directory".into()))
    }
}

#[async_trait]
impl BackendCore for MiseBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("mise")
    }
    fn probes(&self) -> Vec<String> {
        vec!["mise".into()]
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for MiseBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct MiseInstallable {
    pub core: Arc<MiseBackendCore>,
}

#[async_trait]
impl Installable for MiseInstallable {
    /// One `mise use -g` for every tool (`Q45`). `mise use -g node@22 go@1.23` is one
    /// resolution and one write to the global config; one call each rewrites that file N times.
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }
        let tool_specs: Vec<String> = specs
            .iter()
            .map(|spec| {
                let version = spec
                    .options
                    .get("version")
                    .map(|v| v.as_str())
                    .unwrap_or("latest");
                format!("{}@{}", spec.name, version)
            })
            .collect();
        info!("Mise: Installing {} global tool(s)...", tool_specs.len());
        let mut args = vec!["use".to_string(), "-g".to_string()];
        crate::core::argv::push_names(&mut args, "mise", tool_specs.iter().map(String::as_str));
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.core
            .executor
            .run_exclusive("mise", "mise", &arg_refs, false)
            .await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool, _reaped: crate::app::sync::guard::Reaped) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        info!("Mise: Uninstalling {} tool(s)...", names.len());
        let mut args = vec!["uninstall".to_string()];
        crate::core::argv::push_names(&mut args, "mise", names.iter().map(String::as_str));
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.core
            .executor
            .run_exclusive("mise", "mise", &arg_refs, false)
            .await?;
        Ok(())
    }
}

pub struct MiseQueryable {
    pub core: Arc<MiseBackendCore>,
}

#[async_trait]
impl Queryable for MiseQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("mise", &["list", "--json"], false)
            .await?;
        if output.is_empty() || output == "{}" {
            return Ok(vec![]);
        }
        let json: Value = serde_json::from_str(&output)
            .map_err(|e| Error::Other(format!("Mise JSON error: {}", e)))?;
        let mut packages = Vec::new();
        if let Some(tools) = json.as_object() {
            for (name, versions) in tools {
                if let Some(v_list) = versions.as_array() {
                    for v_obj in v_list {
                        // mise keeps reporting a tool after `mise uninstall` — the entry stays,
                        // with `installed: false`, because the *declaration* in its config is
                        // still there. Reporting those as installed told LiNix a package was
                        // present when it was not, which is drift detection reading backwards.
                        if v_obj.get("installed").and_then(|i| i.as_bool()) == Some(false) {
                            continue;
                        }
                        let version = v_obj
                            .get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let mut p = Package::with_version(name, version, "mise");
                        if let Some(source) = v_obj
                            .get("source")
                            .and_then(|s| s.get("type"))
                            .and_then(|t| t.as_str())
                        {
                            p.properties
                                .insert("source_type".into(), source.to_string());
                        }
                        packages.push(p);
                    }
                }
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let all = self.list_installed().await?;
        Ok(all
            .into_iter()
            .filter(|p| {
                p.properties
                    .get("source_type")
                    .map(|s| s == "global")
                    .unwrap_or(false)
            })
            .collect())
    }

    /// Is this tool installed **here**?
    ///
    /// It used to ask `mise plugins ls --all`, which lists every plugin mise has ever heard
    /// of — so `info` answered `Some` for anything in the catalogue, the planner read that as
    /// "already installed", and `linix install mise:jq` reported *already up to date* while
    /// installing nothing. Found by the `tools` container image, which is the only place a
    /// real mise runs (2026-07-24).
    ///
    /// The catalogue answers a different question — *could this be installed?* — and nothing
    /// asks that here. `info` consults the installed set and nothing else.
    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        if let Some(mut p) = all.into_iter().find(|p| p.name == name) {
            let version = p.version.as_deref().unwrap_or("unknown").to_string();
            let install_path = self
                .core
                .mise_data_dir()?
                .join("installs")
                .join(&p.name)
                .join(&version);
            p.properties.insert(
                "install_path".into(),
                install_path.to_string_lossy().to_string(),
            );
            Ok(Some(p))
        } else {
            Ok(None)
        }
    }
}

pub struct MiseSearchable {
    pub core: Arc<MiseBackendCore>,
}

#[async_trait]
impl Searchable for MiseSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // `mise registry` lists every known tool ("<name>  <backend:slug> ..."). There is
        // no server-side search, so filter the registry by the query substring.
        let output = self
            .core
            .executor
            .search_output("mise", &["registry"], false)
            .await?;
        Ok(filter_mise_registry(&output, query))
    }
}

/// Filter `mise registry` output (`"<name>  <backend:slug> ..."`) by query substring.
fn filter_mise_registry(output: &str, query: &str) -> Vec<Package> {
    let q = query.to_lowercase();
    let mut results = Vec::new();
    for line in output.lines() {
        let name = line.split_whitespace().next().unwrap_or("").trim();
        if name.is_empty() || name.eq_ignore_ascii_case("tool") {
            continue;
        } // skip header
        if name.to_lowercase().contains(&q) {
            results.push(Package::new(name, "mise"));
        }
    }
    results
}

pub struct MiseUpgradable {
    pub core: Arc<MiseBackendCore>,
}

#[async_trait]
impl Upgradable for MiseUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        info!("Mise: Updating plugin repository metadata...");
        self.core
            .executor
            .run("mise", &["plugins", "update"], false)
            .await?;
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("Mise: Upgrading all globally installed tools...");
        self.core
            .executor
            .run_exclusive("mise", "mise", &["upgrade"], false)
            .await?;
        Ok(())
    }

    async fn clean_cache(&self, _sudo: bool) -> Result<()> {
        info!("Mise: Pruning unused tool versions from cache...");
        self.core
            .executor
            .run("mise", &["prune", "--force"], false)
            .await?;
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(MiseBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(MiseInstallable { core: core.clone() }))
            .with_queryable(Arc::new(MiseQueryable { core: core.clone() }))
            .with_searchable(Arc::new(MiseSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(MiseUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mise_tool_names_come_after_the_terminator() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(MiseBackendCore::new(exec));

        MiseInstallable { core: core.clone() }
            .install(
                &[PackageSpec {
                    name: "node".into(),
                    backend: "mise".into(),
                    ..Default::default()
                }],
                false,
            )
            .await
            .unwrap();
        MiseInstallable { core: core.clone() }
            .remove(&["node".to_string()], false, crate::app::sync::guard::Reaped::for_reason(crate::app::sync::guard::GuardScope::Remove, "a unit test of the effector itself"))
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec!["mise use -g -- node@latest", "mise uninstall -- node"]
        );
    }

    /// Real `mise list --json` output, captured from the `tools` container on 2026-07-24.
    /// Both states in one document: `jq` uninstalled (the entry mise keeps after
    /// `mise uninstall`) and `node` installed.
    const REAL_LIST_JSON: &str = r#"{
      "jq": [
        {
          "version": "1.8.2",
          "requested_version": "latest",
          "install_path": "/root/.local/share/mise/installs/jq/1.8.2",
          "source": { "type": "mise.toml", "path": "/root/.config/mise/config.toml" },
          "installed": false,
          "active": false
        }
      ],
      "node": [
        {
          "version": "22.1.0",
          "requested_version": "latest",
          "install_path": "/root/.local/share/mise/installs/node/22.1.0",
          "source": { "type": "mise.toml", "path": "/root/.config/mise/config.toml" },
          "installed": true,
          "active": true
        }
      ]
    }"#;

    fn mise_with(
        list_json: &str,
    ) -> (
        Arc<MiseBackendCore>,
        Arc<crate::core::executor::MockExecutor>,
    ) {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        mock.set_response(
            "mise list --json",
            Ok(crate::core::executor::DryRunOutput {
                stdout: list_json.as_bytes().to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        (Arc::new(MiseBackendCore::new(exec)), mock)
    }

    /// mise keeps a tool in `list --json` after `mise uninstall`, flagged `installed: false`,
    /// because the declaration in its config survives. Reporting those as installed told LiNix
    /// a package was present when it was not.
    #[tokio::test]
    async fn a_tool_mise_says_is_not_installed_is_not_listed() {
        let (core, _) = mise_with(REAL_LIST_JSON);
        let listed = MiseQueryable { core }.list_installed().await.unwrap();
        let names: Vec<&str> = listed.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["node"],
            "an uninstalled tool was reported as installed"
        );
    }

    /// The fail-silent bug the `tools` image caught: `info` asked mise's plugin CATALOGUE, so
    /// it answered "yes" for anything mise had heard of. The planner read that as "already
    /// installed" and `linix install mise:jq` reported *already up to date* while installing
    /// nothing at all.
    #[tokio::test]
    async fn info_answers_installed_here_not_known_to_mise() {
        let (core, mock) = mise_with(REAL_LIST_JSON);
        let q = MiseQueryable { core };

        // `jq` is a tool mise knows and could install, but it is NOT installed.
        assert!(
            q.info("jq").await.unwrap().is_none(),
            "info claimed an uninstalled tool was present — the planner would skip its install"
        );
        // A tool mise has never heard of is likewise absent.
        assert!(q.info("nosuchtool").await.unwrap().is_none());
        // And one that really is installed is found, with its version.
        let found = q.info("node").await.unwrap().expect("node is installed");
        assert_eq!(found.version.as_deref(), Some("22.1.0"));

        // The catalogue is never consulted: it answers "could this be installed?", which is a
        // different question and the one that caused the bug.
        let calls = mock.get_calls().await;
        assert!(
            !calls.iter().any(|c| c.contains("plugins")),
            "info consulted the plugin catalogue: {:?}",
            calls
        );
    }

    /// An empty mise reports `{}`, which must be no packages rather than a parse error.
    #[tokio::test]
    async fn an_empty_mise_lists_nothing() {
        let (core, _) = mise_with("{}");
        assert!(MiseQueryable { core }
            .list_installed()
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn mise_registry_filters_by_query() {
        let out = "Tool  Backends\nnode  core:node\nnodejs  asdf:nodejs\npython  core:python\n";
        let pkgs = filter_mise_registry(out, "node");
        // matches "node" and "nodejs", skips header + python
        assert_eq!(pkgs.len(), 2);
        assert!(pkgs.iter().any(|p| p.name == "node"));
        assert!(pkgs.iter().any(|p| p.name == "nodejs"));
        assert!(pkgs.iter().all(|p| p.backend == "mise"));
    }

    /// Q45: **one command for N packages, not N commands.**
    ///
    /// The generic backend batches; this one is hand-written and did not. `mise` takes a
    /// list, so N one at a time is N of whatever that command costs — and where it runs under
    /// `run_exclusive`, N serialised lock acquisitions on top.
    #[tokio::test]
    async fn a_batch_of_tools_is_one_mise_call() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(MiseBackendCore::new(exec));
        let specs = vec![
            crate::core::PackageSpec {
                name: "node".into(),
                backend: "mise".into(),
                ..Default::default()
            },
            crate::core::PackageSpec {
                name: "go".into(),
                backend: "mise".into(),
                ..Default::default()
            },
        ];
        MiseInstallable { core: core.clone() }
            .install(&specs, false)
            .await
            .unwrap();
        MiseInstallable { core: core.clone() }
            .remove(&["node".to_string(), "go".to_string()], false, crate::app::sync::guard::Reaped::for_reason(crate::app::sync::guard::GuardScope::Remove, "a unit test of the effector itself"))
            .await
            .unwrap();

        let calls = mock.get_calls().await;
        assert_eq!(
            calls.len(),
            2,
            "expected 2 command(s) for the whole batch, got {}: {:?}",
            calls.len(),
            calls
        );
        assert!(
            calls[0].contains("node@latest") && calls[0].contains("go@latest"),
            "{:?}",
            calls
        );
        assert!(
            calls[1].contains("node") && calls[1].contains("go"),
            "{:?}",
            calls
        );
    }
}
