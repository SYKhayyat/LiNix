// src/backends/conda.rs
//
// Conda is env-scoped: every operation targets a named environment. Which env is a
// user choice, so it can't be expressed by the generic (arg-list) backend — this
// specialized backend injects `-n <env>` into each command. The env is read from
// `backend_settings.conda.env` (default "base"), letting users point LiNix at the
// environment they actually work in.

use crate::config::Config;
use crate::core::{
    BackendCapabilities, BackendCore, CommandExecutor, Installable, MetadataProvider, Package,
    PackageSpec, Queryable, Result, Searchable, Upgradable,
};
use crate::parsers::conda::{parse_conda_list, parse_conda_search};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Default Conda environment when the user hasn't chosen one.
const DEFAULT_ENV: &str = "base";

#[derive(Clone)]
pub struct CondaBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    /// The Conda environment all operations target (e.g. "base", "ml").
    pub env: String,
}

impl CondaBackendCore {
    pub fn new(executor: CommandExecutor, env: String) -> Self {
        Self {
            executor,
            name: "conda".to_string(),
            env,
        }
    }
}

#[async_trait]
impl BackendCore for CondaBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("conda")
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for CondaBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        // Conda resolves dependencies internally at install time; it exposes no stable,
        // cheap per-package dependency query, so report none rather than guess.
        Ok(vec![])
    }
}

pub struct CondaInstallable {
    pub core: Arc<CondaBackendCore>,
}

#[async_trait]
impl Installable for CondaInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }
        let mut args: Vec<String> = vec![
            "install".into(),
            "-n".into(),
            self.core.env.clone(),
            "-y".into(),
        ];
        for spec in specs {
            // Reproducible installs: Conda pins with `name=version`.
            match spec
                .options
                .get("version")
                .filter(|v| crate::backends::concrete_version(v))
            {
                Some(ver) => args.push(format!("{}={}", spec.name, ver)),
                None => args.push(spec.name.clone()),
            }
        }
        info!("Conda: Installing into env '{}'...", self.core.env);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.core
            .executor
            .run_exclusive("conda", "conda", &arg_refs, false)
            .await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        let mut args: Vec<String> = vec![
            "remove".into(),
            "-n".into(),
            self.core.env.clone(),
            "-y".into(),
        ];
        args.extend(names.iter().cloned());
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.core
            .executor
            .run_exclusive("conda", "conda", &arg_refs, false)
            .await?;
        Ok(())
    }
}

pub struct CondaQueryable {
    pub core: Arc<CondaBackendCore>,
}

#[async_trait]
impl Queryable for CondaQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("conda", &["list", "-n", &self.core.env, "--json"], false)
            .await?;
        Ok(parse_conda_list(&output))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct CondaSearchable {
    pub core: Arc<CondaBackendCore>,
}

#[async_trait]
impl Searchable for CondaSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // Search spans all configured channels, not a single env, so it takes no `-n`.
        let output = self
            .core
            .executor
            .run_output("conda", &["search", query, "--json"], false)
            .await?;
        Ok(parse_conda_search(&output))
    }
}

pub struct CondaUpgradable {
    pub core: Arc<CondaBackendCore>,
}

#[async_trait]
impl Upgradable for CondaUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        // Conda has no separate index-refresh step; upgrades resolve against channels live.
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!(
            "Conda: Upgrading all packages in env '{}'...",
            self.core.env
        );
        self.core
            .executor
            .run_exclusive(
                "conda",
                "conda",
                &["update", "-n", &self.core.env, "-y", "--all"],
                false,
            )
            .await?;
        Ok(())
    }

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        // Conda has no orphan concept; `conda clean` only clears caches. Report honestly.
        Err(crate::core::Error::Unsupported("conda".into()))
    }
}

/// Reads the target Conda environment from `backend_settings.conda.env`, defaulting to
/// `base`.
fn resolve_env(config: &Config) -> String {
    config
        .backend_settings
        .get("conda")
        .and_then(|m| m.get("env"))
        .filter(|e| !e.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| DEFAULT_ENV.to_string())
}

/// Build and register the Conda backend with all its capabilities.
pub fn register(reg: &mut crate::backends::BackendRegistry, exec: &CommandExecutor, cfg: &Config) {
    let env = resolve_env(cfg);
    let core = Arc::new(CondaBackendCore::new(exec.duplicate(), env));
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(CondaInstallable { core: core.clone() }))
            .with_queryable(Arc::new(CondaQueryable { core: core.clone() }))
            .with_searchable(Arc::new(CondaSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(CondaUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn resolve_env_defaults_to_base() {
        let cfg = Config::default();
        assert_eq!(resolve_env(&cfg), "base");
    }

    #[test]
    fn resolve_env_reads_backend_setting() {
        let mut cfg = Config::default();
        let mut conda = HashMap::new();
        conda.insert("env".to_string(), "ml".to_string());
        cfg.backend_settings.insert("conda".to_string(), conda);
        assert_eq!(resolve_env(&cfg), "ml");
    }

    #[test]
    fn resolve_env_ignores_blank() {
        let mut cfg = Config::default();
        let mut conda = HashMap::new();
        conda.insert("env".to_string(), "   ".to_string());
        cfg.backend_settings.insert("conda".to_string(), conda);
        assert_eq!(resolve_env(&cfg), "base");
    }
}
