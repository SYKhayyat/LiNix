// src/backends/mise.rs

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
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            let version = spec
                .options
                .get("version")
                .map(|v| v.as_str())
                .unwrap_or("latest");
            let tool_spec = format!("{}@{}", spec.name, version);
            info!("Mise: Installing global tool {}...", tool_spec);
            self.core
                .executor
                .run_exclusive("mise", "mise", &["use", "-g", &tool_spec], false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("Mise: Uninstalling tool {}...", name);
            self.core
                .executor
                .run_exclusive("mise", "mise", &["uninstall", name], false)
                .await?;
        }
        Ok(())
    }
}

pub struct MiseQueryable {
    pub core: Arc<MiseBackendCore>,
}

#[async_trait]
impl Queryable for MiseQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
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

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let output = self
            .core
            .executor
            .run_output("mise", &["plugins", "ls", "--all", "--urls"], false)
            .await?;
        for line in output.lines() {
            if let Some((plugin_name, url)) = line.split_once(' ') {
                if plugin_name.trim() == name {
                    let mut p = Package::new(name, "mise");
                    p.properties
                        .insert("repository_url".into(), url.trim().to_string());
                    let install_path = self.core.mise_data_dir()?.join("installs").join(name);
                    p.properties.insert(
                        "install_path".into(),
                        install_path.to_string_lossy().to_string(),
                    );
                    return Ok(Some(p));
                }
            }
        }
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
            .run_output("mise", &["registry"], false)
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

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        info!("Mise: Pruning unused tool versions from cache...");
        self.core
            .executor
            .run("mise", &["prune", "--force"], false)
            .await?;
        Ok(())
    }
}

/// Build and register the mise backend with all its capabilities.
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
    use super::filter_mise_registry;

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
}
