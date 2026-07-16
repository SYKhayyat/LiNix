use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct CargoBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl CargoBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "cargo".to_string(),
        }
    }

    async fn get_cargo_root(&self) -> Result<String> {
        match std::env::var("CARGO_HOME") {
            Ok(home) => Ok(home),
            Err(_) => {
                let user_home = dirs::home_dir()
                    .ok_or_else(|| Error::Other("Could not determine home directory".into()))?;
                Ok(user_home.join(".cargo").to_string_lossy().to_string())
            }
        }
    }
}

#[async_trait]
impl BackendCore for CargoBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("cargo")
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for CargoBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct CargoInstallable {
    pub core: Arc<CargoBackendCore>,
}

#[async_trait]
impl Installable for CargoInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            info!("Cargo: Installing {}...", spec.name);
            let mut args = vec!["install", spec.name.as_str()];
            // Reproducible installs: pin the exact version with `--version` when set.
            if let Some(v) = spec
                .options
                .get("version")
                .filter(|v| crate::backends::concrete_version(v))
            {
                args.push("--version");
                args.push(v.as_str());
            }
            self.core
                .executor
                .run_exclusive("cargo", "cargo", &args, false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            info!("Cargo: Uninstalling {}...", name);
            self.core
                .executor
                .run_exclusive("cargo", "cargo", &["uninstall", name], false)
                .await?;
        }
        Ok(())
    }
}

pub struct CargoQueryable {
    pub core: Arc<CargoBackendCore>,
}

#[async_trait]
impl Queryable for CargoQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("cargo", &["install", "--list"], false)
            .await?;
        Ok(parse_cargo_list(&output))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        if let Some(mut pkg) = all.into_iter().find(|p| p.name == name) {
            let cargo_root = self.core.get_cargo_root().await?;
            // Build the binary path with PathBuf and a platform-correct executable
            // extension (`.exe` on Windows) instead of hardcoding POSIX `/bin/<name>`.
            let bin_name = if cfg!(windows) {
                format!("{}.exe", name)
            } else {
                name.to_string()
            };
            let bin_path = std::path::Path::new(&cargo_root)
                .join("bin")
                .join(&bin_name);
            if bin_path.exists() || self.core.executor.dry_run {
                pkg.properties.insert("install_path".into(), cargo_root);
                pkg.properties
                    .insert("bin_path".into(), bin_path.to_string_lossy().to_string());
            }
            Ok(Some(pkg))
        } else {
            Ok(None)
        }
    }
}

pub struct CargoSearchable {
    pub core: Arc<CargoBackendCore>,
}

#[async_trait]
impl Searchable for CargoSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("cargo", &["search", query], false)
            .await?;
        Ok(parse_cargo_search(&output))
    }
}

/// Parse `cargo install --list`. Crate headers are at column 0 ("ripgrep v13.0.0:");
/// binary names follow on INDENTED lines and must be skipped (else they yield empty
/// package names).
fn parse_cargo_list(output: &str) -> Vec<Package> {
    let mut packages = Vec::new();
    for line in output.lines() {
        if line.starts_with(char::is_whitespace) || line.trim().is_empty() {
            continue; // binary/continuation line
        }
        if let Some((name, rest)) = line.split_once(' ') {
            let version = rest.trim().trim_start_matches('v').trim_end_matches(':');
            packages.push(Package::with_version(name.trim(), version, "cargo"));
        }
    }
    packages
}

/// Parse `cargo search <q>`. Each result is `name = "version"    # description`.
fn parse_cargo_search(output: &str) -> Vec<Package> {
    let mut results = Vec::new();
    for line in output.lines() {
        let Some((name, rest)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        // The trailing "... and N crates more (use --limit N to see more)" line has no '='
        // before it, but guard anyway.
        if name.is_empty() || name.starts_with("...") {
            continue;
        }
        let mut pkg = Package::new(name, "cargo");
        if let Some((ver, _)) = rest.trim().trim_start_matches('"').split_once('"') {
            if !ver.is_empty() {
                pkg.version = Some(ver.to_string());
            }
        }
        if let Some((_, desc)) = rest.split_once('#') {
            let desc = desc.trim();
            if !desc.is_empty() {
                pkg.properties
                    .insert("description".into(), desc.to_string());
            }
        }
        results.push(pkg);
    }
    results
}

pub struct CargoUpgradable {
    pub core: Arc<CargoBackendCore>,
}

#[async_trait]
impl Upgradable for CargoUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("Cargo: Upgrading all installed packages...");
        let installed = self.core.list_installed_internal().await?;
        for pkg in installed {
            let _ = self
                .core
                .executor
                .run_exclusive("cargo", "cargo", &["install", &pkg.name, "--force"], false)
                .await;
        }
        Ok(())
    }

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        // cargo has no global orphan concept; report honestly instead of faking success.
        Err(crate::core::Error::Unsupported("cargo".into()))
    }
}

impl CargoBackendCore {
    async fn list_installed_internal(&self) -> Result<Vec<Package>> {
        let queryable = CargoQueryable {
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
    let core = Arc::new(CargoBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(CargoInstallable { core: core.clone() }))
            .with_queryable(Arc::new(CargoQueryable { core: core.clone() }))
            .with_searchable(Arc::new(CargoSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(CargoUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_list_skips_indented_binaries() {
        let out = "ripgrep v13.0.0:\n    rg\nexa v0.10.1 (/some/path):\n    exa\n";
        let pkgs = parse_cargo_list(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "ripgrep");
        assert_eq!(pkgs[0].version.as_deref(), Some("13.0.0"));
        assert_eq!(pkgs[1].name, "exa");
        // none should be empty-named (the indented binary lines)
        assert!(pkgs.iter().all(|p| !p.name.is_empty()));
    }

    #[test]
    fn cargo_search_parses_name_version_desc() {
        let out = "ripgrep = \"13.0.0\"    # line-oriented search tool\n\
                   bat = \"0.24.0\"       # a cat clone\n\
                   ... and 50 crates more (use --limit N to see more)\n";
        let pkgs = parse_cargo_search(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "ripgrep");
        assert_eq!(pkgs[0].version.as_deref(), Some("13.0.0"));
        assert_eq!(
            pkgs[0].properties.get("description").map(String::as_str),
            Some("line-oriented search tool")
        );
        assert_eq!(pkgs[1].name, "bat");
    }
}
