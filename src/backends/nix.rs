use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;

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
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("nix")
    }
    fn probes(&self) -> Vec<String> {
        vec!["nix".into()]
    }

    fn needs_root(&self) -> bool {
        // Nix profiles are managed per-user in the nix store; usually doesn't require sudo.
        false
    }
}

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
            let mut args = vec!["profile".to_string(), "install".to_string()];
            crate::core::argv::push_names(&mut args, "nix", [&flake_uri]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("nix", "nix", &arg_refs, sudo)
                .await?;
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
            if let Some(pkg) = installed.iter().find(|p| p.name == *name) {
                match pkg
                    .properties
                    .get("index")
                    .and_then(|i| i.parse::<usize>().ok())
                {
                    Some(idx) => indexed.push((idx, name)),
                    None => by_name.push(name),
                }
            }
        }

        indexed.sort_by_key(|x| std::cmp::Reverse(x.0)); // highest index first
        for (idx, name) in indexed {
            let idx_str = idx.to_string();
            info!(
                "Nix: Removing package at profile index {} ({})",
                idx_str, name
            );
            let mut args = vec!["profile".to_string(), "remove".to_string()];
            crate::core::argv::push_names(&mut args, "nix", [&idx_str]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("nix", "nix", &arg_refs, sudo)
                .await?;
        }

        // Fallback path: remove by attribute name (modern `nix profile remove <name>`).
        for name in by_name {
            info!("Nix: Removing package by name ({})", name);
            let mut args = vec!["profile".to_string(), "remove".to_string()];
            crate::core::argv::push_names(&mut args, "nix", [name]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("nix", "nix", &arg_refs, sudo)
                .await?;
        }
        Ok(())
    }
}

pub struct NixQueryable {
    pub core: Arc<NixBackendCore>,
}

#[async_trait]
impl Queryable for NixQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
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
        // `--json` moves ahead of the terminator: behind it nix would read it as a flake ref.
        let mut args = vec!["search".to_string(), "--json".to_string()];
        crate::core::argv::push_names(&mut args, "nix", ["nixpkgs", query]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self
            .core
            .executor
            .search_output("nix", &arg_refs, false)
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
            let name = meta
                .get("pname")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| attr.rsplit('.').next().unwrap_or(attr));
            let mut p = Package::new(name, "nix");
            if let Some(v) = meta.get("version").and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    p.version = Some(v.to_string());
                }
            }
            if let Some(d) = meta.get("description").and_then(|v| v.as_str()) {
                if !d.is_empty() {
                    p.properties.insert("description".into(), d.to_string());
                }
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
        self.core
            .executor
            .run_exclusive("nix", "nix", &["profile", "upgrade", "--all"], sudo)
            .await?;
        Ok(())
    }

    async fn clean_cache(&self, sudo: bool) -> Result<()> {
        info!(
            "Nix: Performing garbage collection (GC, older than {})...",
            self.core.gc_age
        );
        self.core
            .executor
            .run(
                "nix-collect-garbage",
                &["--delete-older-than", &self.core.gc_age],
                sudo,
            )
            .await?;
        Ok(())
    }
}

impl NixBackendCore {
    async fn list_installed_internal(&self) -> Result<Vec<Package>> {
        let output = self
            .executor
            .run_output("nix", &["profile", "list", "--json"], false)
            .await?;
        if output.is_empty() || output == "{}" {
            return Ok(vec![]);
        }

        let json: Value = serde_json::from_str(&output)
            .map_err(|e| Error::Other(format!("Nix JSON error: {}", e)))?;
        let packages = parse_profile_list(&json);

        Ok(packages)
    }
}

/// `nix profile list --json` -> the packages LiNix owns.
///
/// **Two shapes, one manager.** `elements` was an ARRAY (position = identity) until Nix 2.20 and
/// is an OBJECT KEYED BY NAME from schema v3 onward. LiNix read only the array, so on a modern
/// nix `list` returned nothing it had just installed — E6's class, a blind `list` producing
/// permanent phantom drift, on the one backend no image had ever installed. Measured against
/// Determinate Nix 3.21.9 in the tools image; the capture is
/// `tests/fixtures/nix/profile-list-json.txt`.
///
/// Both are read here because they are one tool's output across versions, not two mechanisms of
/// LiNix's own — the NO-LEGACY rule is about this codebase's formats, not upstream's.
fn parse_profile_list(json: &Value) -> Vec<Package> {
    let fields = |el: &Value, key: Option<&str>| {
        let attr_path = el.get("attrPath").and_then(|v| v.as_str());
        let name = key
            .or_else(|| attr_path.and_then(|a| a.split('.').next_back()))
            .unwrap_or("unknown")
            .to_string();
        let store_path = el
            .get("storePaths")
            .and_then(|a| a.as_array())
            .and_then(|a| a.first())
            .and_then(|p| p.as_str())
            .map(str::to_string);
        (name, attr_path.map(str::to_string), store_path)
    };
    let build =
        |name: String, attr: Option<String>, store: Option<String>, index: Option<usize>| {
            let mut p = Package::new(&name, "nix");
            if let Some(a) = attr {
                p.properties.insert("full_attr".into(), a);
            }
            if let Some(sp) = store {
                p.properties.insert("store_path".into(), sp);
            }
            if let Some(i) = index {
                p.properties.insert("index".into(), i.to_string());
            }
            p
        };

    match json.get("elements") {
        // v3: the key IS the name, and it is what `nix profile remove` takes.
        Some(Value::Object(map)) => map
            .iter()
            .map(|(key, el)| {
                let (name, attr, store) = fields(el, Some(key));
                build(name, attr, store, None)
            })
            .collect(),
        // The array form, where position is the identity and removal must renumber.
        Some(Value::Array(elements)) => elements
            .iter()
            .enumerate()
            .map(|(i, el)| {
                let (name, attr, store) = fields(el, None);
                build(name, attr, store, Some(i))
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let core = Arc::new(NixBackendCore::new(
        exec.duplicate(),
        cfg.nix_gc_age.clone(),
    ));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(NixInstallable { core: core.clone() }))
            .with_queryable(Arc::new(NixQueryable { core: core.clone() }))
            .with_searchable(Arc::new(NixSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(NixUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `nix profile list --json`, captured from Determinate Nix 3.21.9 in the tools image the
    /// hour this was written. The schema is v3: `elements` is an OBJECT KEYED BY NAME, and
    /// LiNix asked it for an array — so `nix` listed nothing it had just installed, which is
    /// E6's class (a blind `list` is permanent phantom drift) on a backend that until tonight
    /// had never been installed in any image to notice.
    #[test]
    fn nix_profile_list_reads_the_v3_map_keyed_by_name() {
        const REAL: &str = include_str!("../../tests/fixtures/nix/profile-list-json.txt");
        let json: Value = serde_json::from_str(REAL).unwrap();
        let pkgs = parse_profile_list(&json);
        let mut names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["determinate-nix", "hello", "nss-cacert"]);

        let hello = pkgs.iter().find(|p| p.name == "hello").expect("hello");
        assert_eq!(
            hello.properties.get("full_attr").map(String::as_str),
            Some("legacyPackages.x86_64-linux.hello")
        );
        assert!(hello
            .properties
            .get("store_path")
            .is_some_and(|s| s.contains("hello-2.12.3")));
        // No `index` on the map form: position is not identity there, and the removal path
        // branches on exactly this.
        assert!(!hello.properties.contains_key("index"));

        // And the array form still reads, because an older nix is still a nix.
        let old: Value = serde_json::from_str(
            r#"{"elements":[{"attrPath":"legacyPackages.x86_64-linux.jq","storePaths":["/nix/store/x-jq"]}]}"#,
        )
        .unwrap();
        let old_pkgs = parse_profile_list(&old);
        assert_eq!(old_pkgs.len(), 1);
        assert_eq!(old_pkgs[0].name, "jq");
        assert_eq!(
            old_pkgs[0].properties.get("index").map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn nix_search_parses_json_map() {
        let out = r#"{
            "legacyPackages.x86_64-linux.ripgrep": {"pname":"ripgrep","version":"14.1.0","description":"fast grep"},
            "legacyPackages.x86_64-linux.bat": {"pname":"bat","version":"0.24.0","description":"cat clone"}
        }"#;
        let pkgs = parse_nix_search(out).unwrap();
        assert_eq!(pkgs.len(), 2);
        // HashMap order is nondeterministic; assert by membership.
        let rg = pkgs
            .iter()
            .find(|p| p.name == "ripgrep")
            .expect("ripgrep present");
        assert_eq!(rg.version.as_deref(), Some("14.1.0"));
        assert!(rg.properties.get("attr_path").unwrap().ends_with("ripgrep"));
        assert!(pkgs.iter().any(|p| p.name == "bat"));
    }

    #[tokio::test]
    async fn nix_flake_refs_and_queries_come_after_the_terminator() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(NixBackendCore::new(exec, "30d".into()));

        NixInstallable { core: core.clone() }
            .install(
                &[PackageSpec {
                    name: "ripgrep".into(),
                    backend: "nix".into(),
                    ..Default::default()
                }],
                false,
            )
            .await
            .unwrap();
        NixSearchable { core: core.clone() }
            .search("ripgrep")
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec![
                "nix profile install -- nixpkgs#ripgrep",
                "nix search --json -- nixpkgs ripgrep",
            ]
        );
    }

    #[test]
    fn nix_search_empty_is_ok() {
        assert!(parse_nix_search("{}").unwrap().is_empty());
        assert!(parse_nix_search("").unwrap().is_empty());
    }
}
