use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;

pub struct BrewBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl BrewBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor: executor.with_exit_policy(crate::core::exit_policy::for_manager("brew")),
            name: "brew".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for BrewBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("brew")
    }
    fn probes(&self) -> Vec<String> {
        vec!["brew".into()]
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for BrewBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let mut args = vec!["deps".to_string()];
        crate::core::argv::push_names(&mut args, "brew", [name]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self.executor.run_output("brew", &arg_refs, false).await?;
        Ok(output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }
}

pub struct BrewInstallable {
    pub core: Arc<BrewBackendCore>,
}

#[async_trait]
impl Installable for BrewInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            // Best-effort version pin via brew's versioned formulae (e.g. `python@3.11`).
            // Only some formulae publish versioned variants; otherwise brew installs latest.
            let target = match spec.options.get("version") {
                Some(v) if crate::backends::concrete_version(v) => format!("{}@{}", spec.name, v),
                _ => spec.name.clone(),
            };
            info!("Brew: Installing {}...", target);
            let mut args = vec!["install".to_string()];
            crate::core::argv::push_names(&mut args, "brew", [&target]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("brew", "brew", &arg_refs, false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("Brew: Uninstalling {}...", name);
            let mut args = vec!["uninstall".to_string()];
            crate::core::argv::push_names(&mut args, "brew", [name]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("brew", "brew", &arg_refs, false)
                .await?;
        }
        Ok(())
    }
}

pub struct BrewQueryable {
    pub core: Arc<BrewBackendCore>,
}

#[async_trait]
impl Queryable for BrewQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("brew", &["list", "--versions"], false)
            .await?;
        let mut packages = Vec::new();
        for line in output.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                packages.push(Package::with_version(parts[0], parts[1], "brew"));
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("brew", &["leaves"], false)
            .await?;
        Ok(output
            .lines()
            .map(|l| Package::new(l.trim(), "brew"))
            .collect())
    }

    /// Uses `brew info --json=v1`, the only form that reports the install path.
    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let mut args = vec!["info".to_string(), "--json=v1".to_string()];
        crate::core::argv::push_names(&mut args, "brew", [name]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self
            .core
            .executor
            .run_output("brew", &arg_refs, false)
            .await?;
        if output.is_empty() || output == "[]" {
            return Ok(None);
        }
        let json: Value = serde_json::from_str(&output)
            .map_err(|e| Error::Other(format!("Brew JSON error: {}", e)))?;
        let arr = json
            .as_array()
            .ok_or_else(|| Error::Other("Expected array".into()))?;
        if arr.is_empty() {
            return Ok(None);
        }
        let first = &arr[0];
        let pkg_name = first["name"].as_str().unwrap_or(name).to_string();
        let version = first["versions"]["stable"].as_str().map(|s| s.to_string());
        let mut pkg =
            Package::with_version(&pkg_name, version.as_deref().unwrap_or("unknown"), "brew");
        if let Some(installed) = first["installed"].as_array().and_then(|a| a.first()) {
            if let Some(path) = installed["installed_as_dependency"].as_bool() {
                pkg.properties
                    .insert("installed_as_dependency".into(), path.to_string());
            }
            // The install path is the prefix of the installed keg
            if let Some(prefix) = installed["prefix"].as_str() {
                pkg.properties
                    .insert("install_path".into(), prefix.to_string());
            }
        }
        // Fallback: use the cellar path
        if !pkg.properties.contains_key("install_path") {
            if let Some(cellar) = first["cellar"].as_str() {
                pkg.properties
                    .insert("install_path".into(), format!("{}/{}", cellar, pkg_name));
            }
        }
        Ok(Some(pkg))
    }
}

pub struct BrewSearchable {
    pub core: Arc<BrewBackendCore>,
}

#[async_trait]
impl Searchable for BrewSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let mut args = vec!["search".to_string()];
        crate::core::argv::push_names(&mut args, "brew", [query]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self
            .core
            .executor
            .search_output("brew", &arg_refs, false)
            .await?;
        Ok(parse_brew_search(&output))
    }
}

/// Parse `brew search <q>` — one formula/cask name per line, with "==> Formulae" /
/// "==> Casks" section headers to skip.
fn parse_brew_search(output: &str) -> Vec<Package> {
    let mut results = Vec::new();
    for line in output.lines() {
        let name = line.trim();
        if name.is_empty() || name.starts_with("==>") {
            continue;
        }
        results.push(Package::new(name, "brew"));
    }
    results
}

pub struct BrewUpgradable {
    pub core: Arc<BrewBackendCore>,
}

#[async_trait]
impl Upgradable for BrewUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        self.core.executor.run("brew", &["update"], false).await?;
        Ok(())
    }
    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        self.core
            .executor
            .run_exclusive("brew", "brew", &["upgrade"], false)
            .await?;
        Ok(())
    }
    async fn list_orphans(&self) -> Result<Vec<String>> {
        let out = self
            .core
            .executor
            .run_output("brew", &["autoremove", "--dry-run"], false)
            .await?;
        // `--dry-run` prints "Would remove: a b c" plus prose; the formula names are the
        // lines that are a bare token, which is what every other brew listing looks like.
        Ok(out
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.contains(' ') && !l.starts_with("==>"))
            .map(|l| l.to_string())
            .collect())
    }

    async fn clean_cache(&self, _sudo: bool) -> Result<()> {
        self.core.executor.run("brew", &["cleanup"], false).await?;
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(BrewBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(BrewInstallable { core: core.clone() }))
            .with_queryable(Arc::new(BrewQueryable { core: core.clone() }))
            .with_searchable(Arc::new(BrewSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(BrewUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::executor::MockExecutor;
    use dashmap::DashMap;

    fn mocked() -> (Arc<MockExecutor>, Arc<BrewBackendCore>) {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(false, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        (mock, Arc::new(BrewBackendCore::new(exec)))
    }

    #[tokio::test]
    async fn every_brew_command_ends_its_options_before_the_name() {
        let (mock, core) = mocked();
        let spec = PackageSpec {
            name: "ripgrep".into(),
            backend: "brew".into(),
            ..Default::default()
        };
        BrewInstallable { core: core.clone() }
            .install(&[spec], false)
            .await
            .unwrap();
        BrewInstallable { core: core.clone() }
            .remove(&["ripgrep".to_string()], false)
            .await
            .unwrap();
        BrewQueryable { core: core.clone() }
            .info("ripgrep")
            .await
            .ok();
        BrewSearchable { core: core.clone() }
            .search("ripgrep")
            .await
            .unwrap();
        core.get_dependencies("ripgrep").await.unwrap();

        let calls = mock.get_calls().await;
        assert_eq!(
            calls,
            vec![
                "brew install -- ripgrep",
                "brew uninstall -- ripgrep",
                "brew info --json=v1 -- ripgrep",
                "brew search -- ripgrep",
                "brew deps -- ripgrep",
            ]
        );
    }

    #[test]
    fn brew_search_skips_section_headers() {
        let out = "==> Formulae\nripgrep\nripgrep-all\n==> Casks\nripgrep-cask\n";
        let pkgs = parse_brew_search(out);
        assert_eq!(pkgs.len(), 3);
        assert!(pkgs.iter().all(|p| !p.name.starts_with("==>")));
        assert_eq!(pkgs[0].name, "ripgrep");
    }
}
