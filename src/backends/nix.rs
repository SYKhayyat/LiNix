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
    /// One `nix profile install` for every installable (`Q45`).
    ///
    /// Each invocation evaluates nixpkgs and rebuilds the profile generation; N one at a time
    /// is N evaluations and N generations for a change the user made once.
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }
        let flake_uris: Vec<String> = specs
            .iter()
            .map(|spec| {
                if spec.name.contains('#') {
                    spec.name.clone()
                } else {
                    format!("nixpkgs#{}", spec.name)
                }
            })
            .collect();
        {
            info!(
                "Nix: Installing {} installable(s) to user profile...",
                flake_uris.len()
            );
            let mut args = vec!["profile".to_string(), "install".to_string()];
            crate::core::argv::push_names(&mut args, "nix", flake_uris.iter().map(String::as_str));
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("nix", "nix", &arg_refs, sudo)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool, _reaped: crate::app::sync::guard::Reaped) -> Result<()> {
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

        // Modern `nix profile remove <name> <name>` — one call, and the names are resolved
        // against the profile as it was, so nothing renumbers under them (`Q45`). Verified
        // against real nix 2.x in a container:
        //
        //     removing 'flake:nixpkgs#legacyPackages.x86_64-linux.hello'
        //     removing 'flake:nixpkgs#legacyPackages.x86_64-linux.ripgrep'
        //     removed 2 packages, kept 17 packages
        //
        // **The indexed path above is deliberately left one at a time.** Its highest-first
        // ordering exists because positional indices renumber, and no nix that still reports
        // them was available to prove a batched form safe — a wrong guess there removes a
        // package the user did not name. Modern nix reports no indices at all, so this is the
        // path that actually runs.
        if !by_name.is_empty() {
            info!("Nix: Removing {} package(s) by name...", by_name.len());
            let mut args = vec!["profile".to_string(), "remove".to_string()];
            crate::core::argv::push_names(&mut args, "nix", by_name.iter().copied());
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
        let all = self.installed_listing().await?;
        Ok(all.iter().find(|p| p.name == name).cloned())
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
    let json = crate::parsers::json_document(output).ok_or_else(|| {
        Error::Other(format!(
            "`nix search --json` returned no JSON document, the output opening `{}`",
            output.trim().chars().take(120).collect::<String>()
        ))
    })?;
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
                    p.properties.insert("description".to_string(), d.to_string());
                }
            }
            p.properties.insert("attr_path".to_string(), attr.clone());
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
                p.properties.insert("full_attr".to_string(), a);
            }
            if let Some(sp) = store {
                p.properties.insert("store_path".to_string(), sp);
            }
            if let Some(i) = index {
                p.properties.insert("index".to_string(), i.to_string());
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

    /// Q45. **One `nix profile install` for the batch, and one `remove` for the by-name path.**
    ///
    /// Each invocation evaluates nixpkgs and cuts a new profile generation, so N one at a time
    /// is N evaluations and N generations for one change the user made.
    ///
    /// The by-name removal was verified against real nix 2.x in a container before it was
    /// batched — `nix profile remove hello ripgrep` reported `removed 2 packages, kept 17` and
    /// left the one it was not given. The *indexed* path stays one at a time on purpose: its
    /// highest-index-first ordering exists because positional indices renumber, and no nix that
    /// still reports them was available to prove a batched form safe.
    #[tokio::test]
    async fn a_batch_of_installables_is_one_nix_call() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(NixBackendCore::new(exec, "30d".to_string()));
        let specs = vec![
            crate::core::PackageSpec {
                name: "hello".into(),
                backend: "nix".into(),
                ..Default::default()
            },
            crate::core::PackageSpec {
                name: "ripgrep".into(),
                backend: "nix".into(),
                ..Default::default()
            },
        ];
        NixInstallable { core: core.clone() }
            .install(&specs, false)
            .await
            .unwrap();

        let calls = mock.get_calls().await;
        assert_eq!(calls.len(), 1, "one install for the batch, got {:?}", calls);
        assert!(calls[0].contains("nixpkgs#hello"), "{:?}", calls);
        assert!(calls[0].contains("nixpkgs#ripgrep"), "{:?}", calls);
    }
}
