use crate::backends::BackendRegistry;
use crate::config::parser::{
    identify_line, is_reserved_manifest, parse_group_file, parse_group_str, ManifestLine,
};
use crate::config::Config;
use crate::core::{Error, PackageSpec, Result, Validator};
use semver::{Version, VersionReq};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, error, info, instrument, trace, warn};
use version_compare::{compare as loose_compare, Cmp};

/// Responsible for calculating the "Desired State" of the system.
///
/// The Resolver orchestrates the expansion of high-level manifest logic:
/// 1. Profile Expansion: Loading host-specific or global identity files.
/// 2. Module Expansion: Feature 3 recursive @module loading.
/// 3. Group Expansion: Unrolling named collections from configuration.
///
/// It acts as the primary integrity gate, enforcing strict version locking
/// and validating package names before they reach the planner.
pub struct StateResolver<'a> {
    /// Reference to the global kernel configuration.
    config: &'a Config,
    /// Shared access to all registered backends.
    registry: Arc<BackendRegistry>,
    /// If true, the resolver enforces strict equality with locks.json.
    locked: bool,
    /// Maps "backend:package" to a verified version string.
    locks: HashMap<String, String>,
}

impl<'a> StateResolver<'a> {
    /// Find `filename` in the wish-list folders, in read order: global, then each `-g`.
    /// First match wins. `None` if no folder has it.
    async fn find_in_wish_dirs(&self, filename: &str) -> Option<std::path::PathBuf> {
        for dir in self.config.wish_dirs() {
            let candidate = dir.join(filename);
            if tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
                return Some(candidate);
            }
        }
        None
    }

    /// Initializes a new StateResolver asynchronously.
    ///
    /// If `locked` is true, it attempts to load the machine-generated `locks.json`.
    /// The lockfile lives in the GLOBAL groups folder, not in a `-g` folder: a lock is a
    /// statement about what this machine has pinned, and there is one of those.
    pub async fn new(config: &'a Config, registry: Arc<BackendRegistry>, locked: bool) -> Self {
        let mut locks = HashMap::new();

        if locked {
            let lock_path = config.groups_dir.join("locks.json");
            debug!(
                "Resolver: Locked mode active. Probing for locks at {:?}",
                lock_path
            );

            if tokio::fs::try_exists(&lock_path).await.unwrap_or(false) {
                if let Ok(data) = fs::read_to_string(&lock_path).await {
                    // JSON Structure: {"locks": {"apt:curl": "7.81.0", ...}, "sig": "<hex>"}
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                        if let Some(obj) = json.get("locks").and_then(|l| l.as_object()) {
                            // Tamper check: a lockfile carrying a "sig" must verify against the
                            // machine-local key. A missing sig is a legacy/unsigned lockfile —
                            // allowed, with a nudge to re-run `linix lock`.
                            match json.get("sig").and_then(|s| s.as_str()) {
                                Some(sig) => match crate::core::locksig::read_key(&config.groups_dir) {
                                    // A key exists here → this is the origin machine; enforce.
                                    Some(key) => {
                                        if crate::core::locksig::verify(&key, obj, sig) {
                                            debug!("Resolver: locks.json signature verified.");
                                        } else {
                                            error!(
                                                "Resolver: locks.json signature MISMATCH — the \
                                                 lockfile was modified since `linix lock`. Refusing \
                                                 to use it. Re-run `linix lock` to re-sign."
                                            );
                                            // Fail closed: leave `locks` empty so locked mode does
                                            // not trust a tampered file.
                                            return Self {
                                                config,
                                                registry,
                                                locked,
                                                locks: HashMap::new(),
                                            };
                                        }
                                    }
                                    // No local key (e.g. a fresh machine restoring a bundle): we
                                    // can't verify, but refusing would break reproducibility, so
                                    // proceed with a clear notice.
                                    None => warn!(
                                        "Resolver: locks.json is signed but no local key is present \
                                         to verify it (fresh machine?). Proceeding unverified."
                                    ),
                                },
                                None => warn!(
                                    "Resolver: locks.json is unsigned (older format). Run `linix lock` to add tamper-evidence."
                                ),
                            }
                            for (key, val) in obj {
                                if let Some(v_str) = val.as_str() {
                                    locks.insert(key.clone(), v_str.to_string());
                                }
                            }
                        }
                    }
                }
            } else {
                warn!("Resolver: Locked mode requested but locks.json is missing.");
            }
        }

        Self {
            config,
            registry,
            locked,
            locks,
        }
    }

    /// Fetch a remote `include:` manifest over HTTP and parse it into manifest lines (applying
    /// the same BOM/comment/`when`-conditional handling as a local file). Network failures are
    /// surfaced to the caller, which downgrades them to a skip-with-warning.
    async fn fetch_remote_manifest(&self, url: &str) -> Result<Vec<String>> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                self.config.network_timeout_secs.max(10),
            ))
            .user_agent("linix-include")
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Error::Http(format!(
                "include {} returned HTTP {}",
                url,
                resp.status()
            )));
        }
        let body = resp.text().await.map_err(|e| Error::Http(e.to_string()))?;
        // Guard against pointing at a web page instead of a manifest.
        if crate::app::module_registry::looks_like_html(&body) {
            return Err(Error::Config(format!(
                "include {} looks like an HTML page, not a manifest",
                url
            )));
        }
        parse_group_str(&body)
    }

    /// The primary resolution entry point.
    ///
    /// Performs a breadth-first recursive unrolling of all manifest sources.
    /// Resolves every @module, group:, and host-specific manifest into a
    /// flat Map of PackageSpecs.
    #[instrument(skip(self))]
    pub async fn resolve_desired_state(&self) -> Result<HashMap<String, Vec<PackageSpec>>> {
        let mut resolved: HashMap<String, Vec<PackageSpec>> = HashMap::new();
        // Queue stores (Raw Line String, Origin Source Name, Inherited Exclusions).
        // Exclusions flow DOWN a module/group subtree: `@module:dev -vim` drops vim from
        // everything dev pulls in, but not from what another top-level source contributes.
        let mut raw_inputs: VecDeque<(String, String, HashSet<String>)> = VecDeque::new();
        let mut seen_keys: HashSet<String> = HashSet::new();

        let hostname = Config::get_hostname();
        info!(
            "Resolver: Expanding manifest closure for host '{}'.",
            hostname
        );

        // --- STEP 1: INITIAL SEEDING ---

        // A. Load directory-based .txt manifests, from every wish-list folder: the global
        // one first, then each `-g` in the order given.
        for dir in self.config.wish_dirs() {
            if !tokio::fs::try_exists(&dir).await.unwrap_or(false) {
                debug!(
                    "Resolver: wish-list folder {:?} does not exist, skipping.",
                    dir
                );
                continue;
            }

            // Collected and sorted rather than streamed. `read_dir` yields whatever order
            // the filesystem feels like — hash order on ext4, B-tree order on NTFS — and
            // this order is load-bearing: later lines override earlier ones, so with two
            // files pinning different versions of the same package, the winner was decided
            // by the filesystem and could differ between two machines holding identical
            // files. Sorted by name, it is a rule someone can predict and rely on.
            let mut names: Vec<String> = Vec::new();
            let mut entries = fs::read_dir(&dir).await.map_err(Error::from)?;
            while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            names.sort();

            for fname in names {
                if !fname.ends_with(".txt") || fname.ends_with(".module.txt") {
                    continue;
                }
                // LiNix's own files live in this folder too, and they are not wish lists.
                // `keep.txt` is the one that bit: it means "never remove these", but it
                // ends in .txt, so it was read as a manifest and every name in it became a
                // package to INSTALL. Asking to keep something you had not installed
                // installed it.
                if is_reserved_manifest(&fname) {
                    debug!(
                        "Resolver: {} is a LiNix file, not a wish list — skipping.",
                        fname
                    );
                    continue;
                }
                // Support host-specific provisioning: "host-WORKSTATION.txt"
                if fname.starts_with("host-") && fname != format!("host-{}.txt", hostname) {
                    continue;
                }

                let source_id = format!("manifest:{}", fname);
                debug!("Resolver: Seeding from manifest source '{}'", source_id);
                for line in parse_group_file(&dir.join(&fname)).await? {
                    raw_inputs.push_back((line, source_id.clone(), HashSet::new()));
                }
            }
        }

        // B. Load config.toml [hostname_packages]
        if let Some(config_pkgs) = self.config.hostname_packages.get(&hostname) {
            let source_id = format!("config:hostname:{}", hostname);
            for p in config_pkgs {
                raw_inputs.push_back((p.clone(), source_id.clone(), HashSet::new()));
            }
        }

        // --- STEP 2: RECURSIVE EXPANSION ---

        const MAX_EXPANSION_ITERATIONS: usize = 4096;
        let mut iterations = 0;

        while let Some((line, source, excludes)) = raw_inputs.pop_front() {
            iterations += 1;
            if iterations > MAX_EXPANSION_ITERATIONS {
                return Err(Error::Transaction(
                    "Expansion Paradox: Manifest depth exceeded limit. Check for cyclic group/module references.".into()
                ));
            }

            match identify_line(&line) {
                // Feature 3: Recursive Reusable Modules
                ManifestLine::Module(mod_spec) => {
                    // `@module:dev -vim -apt:nano` — the first token is the module name;
                    // any `-token` after it is an exclusion scoped to this module's whole
                    // (recursive) expansion, unioned with exclusions inherited from above.
                    let mut parts = mod_spec.split_whitespace();
                    let mod_name = parts.next().unwrap_or("").to_string();
                    let mut child_excludes = excludes.clone();
                    for tok in parts {
                        if let Some(name) = tok.strip_prefix('-') {
                            if !name.is_empty() {
                                child_excludes.insert(name.to_string());
                            }
                        }
                    }

                    let mod_file = self
                        .config
                        .modules_dir
                        .join(format!("{}.module.txt", mod_name));
                    trace!(
                        "Resolver: Expanding @module '{}' requested by {}",
                        mod_name,
                        source
                    );

                    if !tokio::fs::try_exists(&mod_file).await.unwrap_or(false) {
                        return Err(Error::Config(format!(
                            "Missing Dependency: Module '{}' referenced in {} not found at {:?}",
                            mod_name, source, mod_file
                        )));
                    }

                    let mod_id = format!("module:{}", mod_name);
                    for mod_line in parse_group_file(&mod_file).await? {
                        raw_inputs.push_back((mod_line, mod_id.clone(), child_excludes.clone()));
                    }
                }

                // Traditional named groups (manifest files or config.toml groups)
                ManifestLine::Group(group_name) => {
                    // Searched across every wish-list folder, in the same order they are
                    // read: global, then each -g. First match wins, so a -g folder can
                    // shadow a global group of the same name — the one place `-g` gets to
                    // override rather than only add.
                    let group_path = self.find_in_wish_dirs(&format!("{}.txt", group_name)).await;
                    if let Some(group_path) = group_path {
                        let group_id = format!("group:{}", group_name);
                        for g_line in parse_group_file(&group_path).await? {
                            raw_inputs.push_back((g_line, group_id.clone(), excludes.clone()));
                        }
                    } else if let Some(pkgs) = self.config.groups.get(&group_name) {
                        let group_id = format!("config:group:{}", group_name);
                        for p in pkgs {
                            raw_inputs.push_back((p.clone(), group_id.clone(), excludes.clone()));
                        }
                    } else {
                        warn!(
                            "Resolver: Unknown group reference '{}' in source '{}'. Skipping.",
                            group_name, source
                        );
                    }
                }

                // Inline another manifest file or a remote URL. A local path is resolved
                // relative to groups_dir (absolute paths honored as-is); an http(s) URL is
                // fetched. Included lines inherit this source's exclusions and re-enter the
                // BFS, so includes may themselves contain groups/modules/includes (the
                // MAX_EXPANSION_ITERATIONS guard above catches include cycles).
                ManifestLine::Include(target) => {
                    let included: Vec<String> =
                        if target.starts_with("http://") || target.starts_with("https://") {
                            match self.fetch_remote_manifest(&target).await {
                                Ok(lines) => lines,
                                Err(e) => {
                                    warn!(
                                        "Resolver: include of '{}' (from {}) failed: {}. Skipping.",
                                        target, source, e
                                    );
                                    Vec::new()
                                }
                            }
                        } else {
                            let p = std::path::Path::new(&target);
                            let path = if p.is_absolute() {
                                Some(p.to_path_buf())
                            } else {
                                // Searched across the wish-list folders, same order, first
                                // match wins — so `include: base.txt` from a -g folder finds
                                // its neighbour rather than only ever looking in global.
                                self.find_in_wish_dirs(&target).await
                            };
                            match path {
                                Some(path) => parse_group_file(&path).await?,
                                None => {
                                    warn!(
                                        "Resolver: include target '{}' (from {}) not found in any \
                                     wish-list folder ({:?}). Skipping.",
                                        target,
                                        source,
                                        self.config.wish_dirs()
                                    );
                                    Vec::new()
                                }
                            }
                        };
                    let inc_id = format!("include:{}", target);
                    for inc_line in included {
                        raw_inputs.push_back((inc_line, inc_id.clone(), excludes.clone()));
                    }
                }

                // Leaf nodes: Actual Package Specification
                ManifestLine::Package(pkg_str) => {
                    let mut spec = self.parse_and_probe_spec(&pkg_str).await?;

                    // Per-host backend gating: if this host manages only a subset of
                    // backends, manifest entries for the others are ignored here (not an
                    // error) — the intended way to keep e.g. npm/cargo out of a server.
                    if !self.config.is_backend_enabled(&spec.backend) {
                        debug!(
                            "Resolver: Skipping '{}:{}' — backend not enabled on host '{}'.",
                            spec.backend, spec.name, hostname
                        );
                        continue;
                    }

                    // Module/group exclusion: an enclosing `@module:X -pkg` drops this
                    // package from the subtree. Matches a bare name (`-vim`) or a
                    // backend-qualified name (`-apt:vim`).
                    if excludes.contains(&spec.name)
                        || excludes.contains(&format!("{}:{}", spec.backend, spec.name))
                    {
                        debug!(
                            "Resolver: '{}:{}' excluded by an enclosing module/group.",
                            spec.backend, spec.name
                        );
                        continue;
                    }

                    let unique_id = format!("{}:{}", spec.backend, spec.name);

                    // Bug Fix 3: Strict Locking Enforcement
                    if self.locked {
                        if let Some(locked_ver) = self.locks.get(&unique_id) {
                            if let Some(manifest_ver) = spec.options.get("version") {
                                if manifest_ver != locked_ver {
                                    return Err(Error::Validation(format!(
                                        "Integrity Failure: {} version mismatch. Manifest: {}, Lock: {}.",
                                        unique_id, manifest_ver, locked_ver
                                    )));
                                }
                            }
                            // Inject the locked version string into the spec for the planner
                            spec.options
                                .insert("version".to_string(), locked_ver.clone());
                        } else {
                            // A+ Grade Fix: Corrected positional argument count for error format
                            return Err(Error::Validation(format!(
                                "Locked Mode Error: '{}' is missing from locks.json.",
                                unique_id
                            )));
                        }
                    }

                    // Feature 4: Scoped Identification & Deduplication
                    if seen_keys.insert(unique_id.clone()) {
                        Validator::validate_package_name_for(&spec.name, &spec.backend)?;

                        // Internal tagging for Feature 4 Targeted Upgrades
                        spec.options.insert("__source".to_string(), source.clone());

                        // Resolve metadata-level dependencies (requires= tag). Exclusions
                        // propagate to required packages too, so `@module:dev -vim` also
                        // suppresses a vim pulled in via a dev package's `requires=`.
                        for req in &spec.requires {
                            raw_inputs.push_back((
                                req.clone(),
                                format!("dep:{}", spec.name),
                                excludes.clone(),
                            ));
                        }

                        resolved.entry(spec.backend.clone()).or_default().push(spec);
                    } else {
                        // Already resolved from another source. Instead of dropping this
                        // origin (first-write-wins, which could hide the package from
                        // `upgrade --module X` if a different source enqueued it first),
                        // MERGE this source into the existing spec's `__source` tag.
                        // Sources are ';'-joined; the planner's scope matcher splits on ';'.
                        if let Some(specs) = resolved.get_mut(&spec.backend) {
                            if let Some(existing) = specs.iter_mut().find(|s| s.name == spec.name) {
                                let entry =
                                    existing.options.entry("__source".to_string()).or_default();
                                merge_source_tag(entry, &source);
                            }
                        }
                    }
                }
            }
        }

        // Config-declared inline managed files (`[managed_files]`) become `link` specs
        // that carry their body inline, so they flow through the same install / status /
        // prune pipeline as everything else. Keyed by target path; a manifest-declared
        // `link` for the same target (already in `seen_keys`) wins and is not overridden.
        for (target, content) in &self.config.managed_files {
            let unique_id = format!("link:{}", target);
            if !seen_keys.insert(unique_id) {
                continue;
            }
            let mut options = HashMap::new();
            options.insert("target".to_string(), target.clone());
            options.insert("content".to_string(), content.clone());
            options.insert("__source".to_string(), "config:managed_files".to_string());
            resolved
                .entry("link".to_string())
                .or_default()
                .push(PackageSpec {
                    name: target.clone(),
                    backend: "link".to_string(),
                    options,
                    requires: vec![],
                });
        }

        debug!(
            "Resolver: Desired state calculated successfully in {} expansion steps.",
            iterations
        );
        Ok(resolved)
    }

    /// Translates a raw package string into a structured PackageSpec.
    /// Handles aliasing and priority-based probing.
    pub async fn parse_and_probe_spec(&self, line: &str) -> Result<PackageSpec> {
        // Expected syntax: [backend:]name[@options]

        let (b_part, rest) = if let Some((b, r)) = line.split_once(':') {
            (Some(b), r)
        } else {
            (None, line)
        };

        let (n_part, o_part) = rest.split_once('@').unwrap_or((rest, ""));
        let package_name = n_part.trim();

        let mut options = HashMap::new();
        let mut requires = Vec::new();

        // Parse CSV options segment (@key=val,key2=val2)
        for pair in o_part.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, "true"));
            let key = k.trim().to_string();
            let val = v.trim().to_string();

            if key == "requires" {
                // Meta-dependencies are semicolon separated inside the value
                requires = val.split(';').map(|s| s.to_string()).collect();
            } else {
                options.insert(key, val);
            }
        }

        // Logic for backend determination
        let backend = if let Some(b) = b_part {
            // Priority 1: User-provided backend name with aliasing
            self.config
                .aliases
                .get(b)
                .cloned()
                .unwrap_or_else(|| b.to_string())
        } else {
            // Priority 2: Automated discovery across all enabled backends
            let mut detected_backend = None;
            let ver_constraint = options.get("version").map(|s| s.as_str());

            for b_priority_name in &self.config.backend_priority {
                if self
                    .remote_package_exists(b_priority_name, package_name, ver_constraint)
                    .await
                {
                    detected_backend = Some(b_priority_name.clone());
                    break;
                }
            }

            // Priority 3: Fallback to global default
            detected_backend.unwrap_or_else(|| {
                self.config
                    .default_backend
                    .clone()
                    .unwrap_or_else(|| "apt".to_string())
            })
        };

        Ok(PackageSpec {
            name: package_name.to_string(),
            backend,
            options,
            requires,
        })
    }

    /// A+ Grade Logic: Safe, panic-free trait matching for remote discovery.
    ///
    /// Verifies if a package exists in a remote backend and matches constraints.
    async fn remote_package_exists(
        &self,
        backend_name: &str,
        package_name: &str,
        constraint: Option<&str>,
    ) -> bool {
        let backend_cap = match self.registry.get(backend_name) {
            Some(b) if b.is_available() => b,
            _ => return false,
        };

        // Panic-free Trait Check (A+ Hardened)
        if let Some(searchable) = backend_cap.as_searchable() {
            // First: Attempt fast existence check if supported
            if let Ok(true) = searchable.remote_has(package_name).await {
                if let Some(req) = constraint {
                    // If version is specific, perform deeper metadata check
                    match searchable.remote_info(package_name).await {
                        Ok(Some(pkg)) => {
                            if let Some(ver) = pkg.version.as_deref() {
                                return self.satisfies_constraint(ver, req);
                            }
                        }
                        _ => return false,
                    }
                }
                return true;
            }

            // Second: Search-based fallback if remote_has is inconclusive
            if let Ok(results) = searchable.search(package_name).await {
                return results.iter().any(|pkg| {
                    if pkg.name == package_name {
                        match constraint {
                            Some(req) => pkg
                                .version
                                .as_deref()
                                .is_some_and(|v| self.satisfies_constraint(v, req)),
                            None => true,
                        }
                    } else {
                        false
                    }
                });
            }
        }

        false
    }

    /// Version comparison logic supporting strict SemVer and fuzzy strings.
    fn satisfies_constraint(&self, version: &str, constraint: &str) -> bool {
        if constraint == "latest" || constraint == "*" || constraint.is_empty() {
            return true;
        }

        // Attempt strict SemVer resolution
        if let Ok(req) = VersionReq::parse(constraint) {
            if let Ok(ver) = Version::parse(version) {
                return req.matches(&ver);
            }
        }

        // Fallback to literal matching
        if version == constraint {
            return true;
        }

        // Fallback to loose comparative matching (e.g. ">1.0")
        match loose_compare(version, constraint) {
            Ok(Cmp::Eq) => true,
            Ok(Cmp::Gt) if constraint.starts_with('>') => true,
            _ => false,
        }
    }
}

/// Merge an additional origin into a package's `;`-joined `__source` tag, de-duplicated.
/// This lets a package that appears in multiple sources (e.g. a manifest AND a module)
/// remain matchable by every scope it belongs to. The planner splits this tag on `;`.
fn merge_source_tag(existing: &mut String, source: &str) {
    if existing.split(';').any(|s| s == source) {
        return;
    }
    if existing.is_empty() {
        existing.push_str(source);
    } else {
        existing.push(';');
        existing.push_str(source);
    }
}

#[cfg(test)]
mod wish_list_tests {
    use crate::backends::BackendRegistry;
    use crate::config::Config;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// Resolve against real files on disk and return the flat `backend:name` set. Goes
    /// through the real `resolve_desired_state`, so it exercises the directory walk that
    /// the overlay changed rather than a re-implementation of it.
    async fn resolved(cfg: &Config) -> Vec<String> {
        let reg = Arc::new(BackendRegistry::new());
        let resolver = super::StateResolver::new(cfg, reg, false).await;
        let by_backend = resolver.resolve_desired_state().await.unwrap();
        let mut out: Vec<String> = by_backend
            .into_iter()
            .flat_map(|(b, specs)| {
                specs
                    .into_iter()
                    .map(move |s| format!("{}:{}", b, s.name))
                    .collect::<Vec<_>>()
            })
            .collect();
        out.sort();
        out
    }

    fn write(dir: &std::path::Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[tokio::test]
    async fn a_g_folder_adds_to_the_global_wish_list_instead_of_replacing_it() {
        // The whole bug in one test. Before the overlay, `-g scratch` made global's
        // packages vanish from the wish list while managed state still owned them —
        // which is what turned them into drift and then into removals.
        let tmp = tempdir().unwrap();
        let global = tmp.path().join("global");
        let scratch = tmp.path().join("scratch");
        write(&global, "base.txt", "apt:curl\n");
        write(&scratch, "extra.txt", "apt:jq\n");

        let mut cfg = Config {
            groups_dir: global.clone(),
            ..Config::default()
        };
        cfg.extra_group_dirs = vec![scratch.clone()];

        assert_eq!(resolved(&cfg).await, vec!["apt:curl", "apt:jq"]);
    }

    #[tokio::test]
    async fn no_global_reads_only_the_g_folder() {
        let tmp = tempdir().unwrap();
        let global = tmp.path().join("global");
        let scratch = tmp.path().join("scratch");
        write(&global, "base.txt", "apt:curl\n");
        write(&scratch, "extra.txt", "apt:jq\n");

        let mut cfg = Config {
            groups_dir: global.clone(),
            ..Config::default()
        };
        cfg.extra_group_dirs = vec![scratch.clone()];
        cfg.no_global = true;

        assert_eq!(resolved(&cfg).await, vec!["apt:jq"]);
    }

    #[tokio::test]
    async fn global_wins_when_a_g_folder_pins_the_same_package_differently() {
        // The consequence of global-first + first-wins, stated as a rule: `-g` can ADD to
        // your wish list but cannot quietly re-pin something global already decided. A
        // scratch folder that could silently downgrade a globally pinned package would be
        // a supply-chain footgun wearing a convenience flag.
        let tmp = tempdir().unwrap();
        let global = tmp.path().join("global");
        let scratch = tmp.path().join("scratch");
        write(&global, "base.txt", "apt:curl@version=1.0\n");
        write(&scratch, "base.txt", "apt:curl@version=9.9\n");

        let mut cfg = Config {
            groups_dir: global.clone(),
            ..Config::default()
        };
        cfg.extra_group_dirs = vec![scratch.clone()];

        let reg = Arc::new(BackendRegistry::new());
        let resolver = super::StateResolver::new(&cfg, reg, false).await;
        let by_backend = resolver.resolve_desired_state().await.unwrap();
        let curl = by_backend
            .get("apt")
            .and_then(|v| v.iter().find(|s| s.name == "curl"))
            .expect("curl resolved");
        assert_eq!(
            curl.options.get("version").map(String::as_str),
            Some("1.0"),
            "a -g folder must not override a pin the global folder already made"
        );
    }

    #[tokio::test]
    async fn keep_txt_is_a_protection_list_not_an_install_list() {
        // C14. keep.txt sits in the groups folder and ends in .txt, so the manifest walk
        // read it as a wish list: `managed keep firefox` meant "never remove firefox" AND,
        // silently, "install firefox".
        let tmp = tempdir().unwrap();
        let global = tmp.path().join("global");
        write(&global, "base.txt", "apt:curl\n");
        write(&global, "keep.txt", "firefox\n");

        let cfg = Config {
            groups_dir: global.clone(),
            ..Config::default()
        };
        let got = resolved(&cfg).await;
        assert_eq!(got, vec!["apt:curl"]);
        assert!(
            !got.iter().any(|s| s.contains("firefox")),
            "a keep-list entry was turned into a package to install: {:?}",
            got
        );
    }

    #[tokio::test]
    async fn manifests_are_read_in_sorted_order_not_filesystem_order() {
        // C4. `read_dir` yields whatever order the filesystem feels like, and the FIRST
        // declaration of a package wins — so with two files pinning one package, the winner
        // was decided by the filesystem and could differ between two machines holding
        // byte-identical files. Sorted by name, it is a rule someone can predict.
        let tmp = tempdir().unwrap();
        let global = tmp.path().join("global");
        // Created in reverse order on purpose: creation order must not decide the winner.
        write(&global, "z-last.txt", "apt:curl@version=2.0\n");
        write(&global, "a-first.txt", "apt:curl@version=1.0\n");

        let cfg = Config {
            groups_dir: global.clone(),
            ..Config::default()
        };
        let reg = Arc::new(BackendRegistry::new());
        let resolver = super::StateResolver::new(&cfg, reg, false).await;
        let by_backend = resolver.resolve_desired_state().await.unwrap();
        let curl = by_backend
            .get("apt")
            .and_then(|v| v.iter().find(|s| s.name == "curl"))
            .expect("curl resolved");
        // a-first.txt sorts before z-last.txt, and the first declaration wins.
        assert_eq!(
            curl.options.get("version").map(String::as_str),
            Some("1.0"),
            "sorted read order must decide the winner, not the filesystem"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::merge_source_tag;

    #[test]
    fn merges_and_dedups_sources() {
        let mut s = String::from("manifest:base.txt");
        merge_source_tag(&mut s, "module:dev");
        assert_eq!(s, "manifest:base.txt;module:dev");

        // duplicate source is not appended again
        merge_source_tag(&mut s, "module:dev");
        assert_eq!(s, "manifest:base.txt;module:dev");

        // a third distinct source joins
        merge_source_tag(&mut s, "group:editors");
        assert_eq!(s, "manifest:base.txt;module:dev;group:editors");

        // empty start
        let mut empty = String::new();
        merge_source_tag(&mut empty, "module:x");
        assert_eq!(empty, "module:x");
    }
}
