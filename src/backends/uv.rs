// src/backends/uv.rs

use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Upgradable,
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for `uv tool` (Astral's fast Python application
/// installer — the modern successor to pipx).
///
/// Scope note: this manages **`uv tool`** installs, which are the system-inventory
/// surface of uv (globally-available CLI applications in isolated environments).
/// Project/venv-scoped `uv pip` packages are deliberately out of scope, exactly as
/// `dotnet` manages global tools and not project NuGet references.
#[derive(Clone)]
pub struct UvBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl UvBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "uv".to_string(),
        }
    }

    /// Returns the base directory where `uv` stores its tool environments.
    async fn get_tools_dir(&self) -> Result<String> {
        let output = self
            .executor
            .run_output("uv", &["tool", "dir"], false)
            .await
            .or_else(|_| {
                let home = dirs::home_dir()
                    .ok_or_else(|| Error::Other("Could not determine home directory".into()))?;
                Ok::<String, Error>(
                    home.join(".local/share/uv/tools")
                        .to_string_lossy()
                        .to_string(),
                )
            })?;
        Ok(output.trim().to_string())
    }
}

#[async_trait]
impl BackendCore for UvBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("uv")
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for UvBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct UvInstallable {
    pub core: Arc<UvBackendCore>,
}

#[async_trait]
impl Installable for UvInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            // uv accepts a PEP 508 requirement spec: pin with `name==version`.
            let target = match spec.options.get("version") {
                Some(v) if crate::backends::concrete_version(v) => format!("{}=={}", spec.name, v),
                _ => spec.name.clone(),
            };
            info!("uv: Installing tool {}...", target);
            self.core
                .executor
                .run_exclusive("uv", "uv", &["tool", "install", target.as_str()], false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("uv: Uninstalling tool {}...", name);
            self.core
                .executor
                .run_exclusive("uv", "uv", &["tool", "uninstall", name.as_str()], false)
                .await?;
        }
        Ok(())
    }
}

pub struct UvQueryable {
    pub core: Arc<UvBackendCore>,
}

impl UvQueryable {
    /// Parse `uv tool list` output. Each installed tool is a top-level line of the
    /// form `ruff v0.2.1`, followed by indented `- <executable>` sub-lines which we
    /// skip. Non-package chatter (e.g. "No tools installed.") lacks a `vX` version
    /// token and is ignored.
    fn parse_list(output: &str) -> Vec<Package> {
        let mut packages = Vec::new();
        for line in output.lines() {
            if line.is_empty() || line.starts_with(char::is_whitespace) || line.starts_with('-') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let Some(name) = parts.next() else { continue };
            match parts.next() {
                // A real version token looks like `v0.2.1`.
                Some(v)
                    if v.starts_with('v')
                        && v[1..].chars().next().is_some_and(|c| c.is_ascii_digit()) =>
                {
                    packages.push(Package::with_version(name, &v[1..], "uv"));
                }
                _ => continue,
            }
        }
        packages
    }
}

#[async_trait]
impl Queryable for UvQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("uv", &["tool", "list"], false)
            .await?;
        Ok(Self::parse_list(&output))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // Every `uv tool` entry was explicitly requested by the user; there are no
        // implicit/dependency tools to distinguish.
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        if let Some(mut pkg) = all.into_iter().find(|p| p.name == name) {
            let tools_dir = self.core.get_tools_dir().await?;
            let venv_path = format!("{}/{}", tools_dir, name);
            pkg.properties.insert("install_path".into(), venv_path);
            Ok(Some(pkg))
        } else {
            Ok(None)
        }
    }
}

pub struct UvUpgradable {
    pub core: Arc<UvBackendCore>,
}

#[async_trait]
impl Upgradable for UvUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        // uv resolves against live indexes at upgrade time; no metadata refresh step.
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("uv: Upgrading all installed tools...");
        self.core
            .executor
            .run_exclusive("uv", "uv", &["tool", "upgrade", "--all"], false)
            .await?;
        Ok(())
    }

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        // uv isolates each tool in its own environment; there are no shared orphans.
        Err(Error::Unsupported("uv".into()))
    }
}

/// Build and register the uv backend with all its capabilities.
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(UvBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(UvInstallable { core: core.clone() }))
            .with_queryable(Arc::new(UvQueryable { core: core.clone() }))
            .with_upgradable(Arc::new(UvUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uv_tool_list() {
        let out = "\
black v24.1.0
- black
- blackd
ruff v0.2.1
- ruff
";
        let pkgs = UvQueryable::parse_list(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "black");
        assert_eq!(pkgs[0].version.as_deref(), Some("24.1.0"));
        assert_eq!(pkgs[0].backend, "uv");
        assert_eq!(pkgs[1].name, "ruff");
        assert_eq!(pkgs[1].version.as_deref(), Some("0.2.1"));
    }

    #[test]
    fn ignores_non_package_chatter() {
        assert!(UvQueryable::parse_list("No tools installed.").is_empty());
        assert!(UvQueryable::parse_list("").is_empty());
    }
}
