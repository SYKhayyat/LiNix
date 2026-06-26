use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec,
    Queryable, Result, Searchable, Upgradable, Error, MetadataProvider
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for Nix (via 'nix profile').
pub struct NixBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    /// Retention window for `nix-collect-garbage --delete-older-than` (from config).
    pub gc_age: String,
}

impl NixBackendCore {
    pub fn new(executor: CommandExecutor, gc_age: String) -> Self {
        Self {
            executor,
            name: "nix".to_string(),
            gc_age,
        }
    }
}

#[async_trait]
impl BackendCore for NixBackendCore {
    fn name(&self) -> &str { &self.name }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("nix")
    }

    fn needs_root(&self) -> bool {
        // Nix profiles are managed per-user in the nix store; usually doesn't require sudo.
        false
    }
}

/// Phase 1.1: MetadataProvider for Nix.
#[async_trait]
impl MetadataProvider for NixBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        // Nix handles its own dependency tree internally during 'nix profile install'.
        // We return an empty list as we don't need to manually orchestrate nix-native deps.
        Ok(vec![])
    }
}

pub struct NixInstallable {
    pub core: Arc<NixBackendCore>,
}

#[async_trait]
impl Installable for NixInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let flake_uri = if spec.name.contains('#') {
                spec.name.clone()
            } else {
                format!("nixpkgs#{}", spec.name)
            };

            info!("Nix: Installing {} to user profile...", flake_uri);
            self.core.executor.run_exclusive("nix", "nix", &["profile", "install", &flake_uri], sudo).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        let installed = self.core.list_installed_internal().await?;

        // `nix profile` identifies elements by their array position ("index"). Each
        // `nix profile remove <index>` RENUMBERS every element after it, so removing
        // multiple packages by their originally-resolved indices would target the wrong
        // elements after the first removal. Remove highest-index-first: lower indices are
        // unaffected by the removal of a higher one, so the snapshot stays valid.
        let mut indexed: Vec<(usize, &str)> = Vec::new();
        let mut by_name: Vec<&str> = Vec::new();
        for name in names {
            // not installed => nothing to remove
            if let Some(pkg) = installed.iter().find(|p| p.name == *name) {
                match pkg.properties.get("index").and_then(|i| i.parse::<usize>().ok()) {
                    Some(idx) => indexed.push((idx, name)),
                    None => by_name.push(name),
                }
            }
        }

        indexed.sort_by(|a, b| b.0.cmp(&a.0)); // descending
        for (idx, name) in indexed {
            let idx_str = idx.to_string();
            info!("Nix: Removing package at profile index {} ({})", idx_str, name);
            self.core.executor.run_exclusive("nix", "nix", &["profile", "remove", &idx_str], sudo).await?;
        }

        // Fallback path: remove by attribute name (modern `nix profile remove <name>`).
        for name in by_name {
            info!("Nix: Removing package by name ({})", name);
            self.core.executor.run_exclusive("nix", "nix", &["profile", "remove", name], sudo).await?;
        }
        Ok(())
    }
}

pub struct NixQueryable {
    pub core: Arc<NixBackendCore>,
}

#[async_trait]
impl Queryable for NixQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        self.core.list_installed_internal().await
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct NixSearchable {
    pub core: Arc<NixBackendCore>,
}

#[async_trait]
impl Searchable for NixSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self.core.executor
            .run_output("nix", &["search", "nixpkgs", query, "--json"], false)
            .await?;
        parse_nix_search(&output)
    }
}

/// Parse `nix search nixpkgs <q> --json` => `{ "<attrpath>": { pname, version, description } }`.
fn parse_nix_search(output: &str) -> Result<Vec<Package>> {
    if output.trim().is_empty() || output.trim() == "{}" {
        return Ok(vec![]);
    }
    let json: Value = serde_json::from_str(output)
        .map_err(|e| Error::Other(format!("Nix search JSON error: {}", e)))?;
    let mut results = Vec::new();
    if let Some(map) = json.as_object() {
        for (attr, meta) in map {
            // Prefer `pname`; otherwise derive from the last attribute-path segment.
            let name = meta.get("pname").and_then(|v| v.as_str())
                .unwrap_or_else(|| attr.rsplit('.').next().unwrap_or(attr));
            let mut p = Package::new(name, "nix");
            if let Some(v) = meta.get("version").and_then(|v| v.as_str()) {
                if !v.is_empty() { p.version = Some(v.to_string()); }
            }
            if let Some(d) = meta.get("description").and_then(|v| v.as_str()) {
                if !d.is_empty() { p.properties.insert("description".into(), d.to_string()); }
            }
            p.properties.insert("attr_path".into(), attr.clone());
            results.push(p);
        }
    }
    Ok(results)
}

pub struct NixUpgradable {
    pub core: Arc<NixBackendCore>,
}

#[async_trait]
impl Upgradable for NixUpgradable {
    async fn update(&self, _: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Nix: Upgrading all packages in user profile...");
        self.core.executor.run_exclusive("nix", "nix", &["profile", "upgrade", "--all"], sudo).await?;
        Ok(())
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        info!("Nix: Performing garbage collection (GC, older than {})...", self.core.gc_age);
        self.core.executor.run("nix-collect-garbage", &["--delete-older-than", &self.core.gc_age], sudo).await?;
        Ok(())
    }
}

impl NixBackendCore {
    /// Internal helper to parse the complex JSON output of 'nix profile list'.
    async fn list_installed_internal(&self) -> Result<Vec<Package>> {
        let output = self.executor.run_output("nix", &["profile", "list", "--json"], false).await?;
        if output.is_empty() || output == "{}" { return Ok(vec![]); }

        let json: Value = serde_json::from_str(&output).map_err(|e| Error::Other(format!("Nix JSON error: {}", e)))?;
        let mut packages = Vec::new();

        if let Some(elements) = json.get("elements").and_then(|e| e.as_array()) {
            for (i, el) in elements.iter().enumerate() {
                let attr_path = el.get("attrPath")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                
                let name = attr_path.split('.').next_back().unwrap_or(attr_path);
                
                let mut p = Package::new(name, "nix");
                p.properties.insert("index".into(), i.to_string());
                p.properties.insert("full_attr".into(), attr_path.to_string());

                if let Some(store_paths) = el.get("storePaths").and_then(|a| a.as_array()) {
                    if let Some(first_path) = store_paths.first().and_then(|p| p.as_str()) {
                        p.properties.insert("store_path".into(), first_path.to_string());
                    }
                }

                packages.push(p);
            }
        }

        Ok(packages)
    }
}

/// Build and register the Nix backend with all its capabilities.
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let core = Arc::new(NixBackendCore::new(exec.duplicate(), cfg.nix_gc_age.clone()));
    reg.register(Arc::new(crate::core::BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(NixInstallable { core: core.clone() }))
        .with_queryable(Arc::new(NixQueryable { core: core.clone() }))
        .with_searchable(Arc::new(NixSearchable { core: core.clone() }))
        .with_upgradable(Arc::new(NixUpgradable { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nix_search_parses_json_map() {
        let out = r#"{
            "legacyPackages.x86_64-linux.ripgrep": {"pname":"ripgrep","version":"14.1.0","description":"fast grep"},
            "legacyPackages.x86_64-linux.bat": {"pname":"bat","version":"0.24.0","description":"cat clone"}
        }"#;
        let pkgs = parse_nix_search(out).unwrap();
        assert_eq!(pkgs.len(), 2);
        // HashMap order is nondeterministic; assert by membership.
        let rg = pkgs.iter().find(|p| p.name == "ripgrep").expect("ripgrep present");
        assert_eq!(rg.version.as_deref(), Some("14.1.0"));
        assert!(rg.properties.get("attr_path").unwrap().ends_with("ripgrep"));
        assert!(pkgs.iter().any(|p| p.name == "bat"));
    }

    #[test]
    fn nix_search_empty_is_ok() {
        assert!(parse_nix_search("{}").unwrap().is_empty());
        assert!(parse_nix_search("").unwrap().is_empty());
    }
}