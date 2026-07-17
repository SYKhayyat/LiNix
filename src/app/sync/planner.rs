// src/app/sync/planner.rs

use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Error, GraphAction, PackageSpec, Result, StateRegistry};
use petgraph::algo::is_cyclic_directed;
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableDiGraph;
use semver::{Version, VersionReq};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, instrument};
use version_compare::{compare as loose_compare, Cmp};

#[derive(Debug, Serialize, Clone, Default)]
pub struct SyncReport {
    pub install: Vec<ReportEntry>,
    pub remove: Vec<ReportEntry>,
    pub change_count: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReportEntry {
    pub backend: String,
    pub name: String,
    pub version: Option<String>,
    pub source: Option<String>,
}

/// Narrows a sync to one profile or module. Absence of a scope is `Option::None` rather
/// than a variant: as an enum variant it was an implicit spare-everything switch that
/// `matches!` early-returns skipped past, so adding a variant produced no compiler error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Profile(String),
    Module(String),
}

/// Split a desired-state map into what must exist and what must not.
///
/// `absent:` is a declaration, not drift: it is you reaching outside what LiNix manages,
/// deliberately, by name (V.7). It shares the map with wishes because the map type is the
/// seam, so it must be separated before anything reads the map as a wish list.
fn partition_by_presence(
    desired: &HashMap<String, Vec<PackageSpec>>,
) -> (
    HashMap<String, Vec<PackageSpec>>,
    HashMap<String, Vec<PackageSpec>>,
) {
    let mut wanted: HashMap<String, Vec<PackageSpec>> = HashMap::new();
    let mut unwanted: HashMap<String, Vec<PackageSpec>> = HashMap::new();
    for (backend, specs) in desired {
        for spec in specs {
            let bucket = if spec.present {
                &mut wanted
            } else {
                &mut unwanted
            };
            bucket.entry(backend.clone()).or_default().push(spec.clone());
        }
    }
    (wanted, unwanted)
}

#[derive(Debug, Default, Clone)]
pub struct SyncChanges {
    pub graph: StableDiGraph<GraphAction, ()>,
    pub install_map: HashMap<String, NodeIndex>,
    pub removal_tracker: HashSet<String>,
}

impl SyncChanges {
    pub fn is_empty(&self) -> bool {
        self.graph.node_count() == 0
    }

    pub fn total_install(&self) -> usize {
        self.graph
            .node_weights()
            .filter(|w| matches!(w, GraphAction::Install(_)))
            .count()
    }

    pub fn total_remove(&self) -> usize {
        self.graph
            .node_weights()
            .filter(|w| matches!(w, GraphAction::Remove { .. }))
            .count()
    }

    /// Produce a copy containing only the Remove actions, for the `prune` command (which
    /// removes drift but never installs). Removals have no inter-node ordering.
    pub fn removals_only(&self) -> SyncChanges {
        let mut out = SyncChanges::default();
        for weight in self.graph.node_weights() {
            if let GraphAction::Remove { name, backend } = weight {
                let key = format!("{}:{}", backend, name);
                out.removal_tracker.insert(key);
                out.graph.add_node(GraphAction::Remove {
                    name: name.clone(),
                    backend: backend.clone(),
                });
            }
        }
        out
    }

    pub fn generate_report(&self) -> SyncReport {
        let mut report = SyncReport::default();
        for weight in self.graph.node_weights() {
            match weight {
                GraphAction::Install(spec) => {
                    report.install.push(ReportEntry {
                        backend: spec.backend.clone(),
                        name: spec.name.clone(),
                        version: spec.options.get("version").cloned(),
                        source: spec.options.get("__source").cloned(),
                    });
                }
                GraphAction::Remove { name, backend } => {
                    report.remove.push(ReportEntry {
                        backend: backend.clone(),
                        name: name.clone(),
                        version: None,
                        source: None,
                    });
                }
            }
        }
        report.change_count = report.install.len() + report.remove.len();
        report
    }
}

pub struct ChangePlanner<'a> {
    registry: Arc<BackendRegistry>,
    state: &'a StateRegistry,
    config: &'a Config,
    /// The backends this host manages, from II.6's `priority` file. Empty = every backend —
    /// the default for the imperative paths (which act on exactly the package they were
    /// given) and for tests. A full `sync`/`watch`/`prune` sets it, so drift removal is
    /// scoped: a managed package whose backend is not in `priority` is left alone, never
    /// removed, because "not listed = LiNix does not use it" (II.6).
    enabled: Vec<String>,
}

impl<'a> ChangePlanner<'a> {
    pub fn new(
        registry: Arc<BackendRegistry>,
        state: &'a StateRegistry,
        config: &'a Config,
    ) -> Self {
        Self {
            registry,
            state,
            config,
            enabled: Vec::new(),
        }
    }

    /// Scope drift removal to these backends (the `priority` file). Without it, drift is
    /// planned for every backend — right for an imperative command, wrong for a full sync
    /// that must not reap a backend you have simply stopped listing.
    pub fn with_enabled(mut self, enabled: Vec<String>) -> Self {
        self.enabled = enabled;
        self
    }

    /// Whether `backend` is one this host manages. An empty scope means every backend.
    fn backend_enabled(&self, backend: &str) -> bool {
        self.enabled.is_empty() || self.enabled.iter().any(|b| b == backend)
    }

    #[instrument(skip(self, desired))]
    pub async fn plan(
        &self,
        desired: &HashMap<String, Vec<PackageSpec>>,
        scope: Option<Scope>,
    ) -> Result<SyncChanges> {
        let mut changes = SyncChanges::default();

        // `absent:` says a package must NOT exist (II.2). Split off FIRST, before any
        // other work: everything downstream of here reads `desired` as a wish list, so an
        // absent declaration left in it would be installed — the exact opposite of what it
        // says. Partitioning at the top means no later branch can misread one.
        let (wanted, unwanted) = partition_by_presence(desired);

        let filtered_desired = self.apply_scope_filtering(&wanted, scope.as_ref());
        let expanded_desired = self
            .expand_transitive_dependencies(&filtered_desired)
            .await?;

        // Precompute desired keys for O(1) lookup
        let desired_keys: HashSet<String> = expanded_desired.keys().cloned().collect();

        // Removal planning (drift / bloatware / expired leases) is GLOBAL: it acts on
        // every managed package not present in `desired`. That is only safe for a full,
        // unscoped sync. When the caller narrows to a single profile/module/group
        // (`upgrade --module X`), `desired` has already been reduced to that scope, so
        // running removal here would delete every package OUTSIDE the scope. A targeted
        // upgrade must be non-destructive — skip all removal planning when scoped.
        if scope.is_none() {
            // `absent:` — the one thing LiNix removes that it does not manage, because
            // you named it (V.7). Scheduled whether or not LiNix installed it; the guard
            // decides whether it may actually go (Phase 3).
            for (backend, specs) in &unwanted {
                for spec in specs {
                    let key = format!("{}:{}", backend, spec.name);
                    if changes.removal_tracker.contains(&key) {
                        continue;
                    }
                    changes.removal_tracker.insert(key);
                    changes.graph.add_node(GraphAction::Remove {
                        name: spec.name.clone(),
                        backend: backend.clone(),
                    });
                }
            }

            // Single pass over all managed packages to schedule removals
            for pkg in &self.state.packages {
                let key = format!("{}:{}", pkg.backend, pkg.name);

                // Skip if already scheduled or present in desired state
                if changes.removal_tracker.contains(&key) || desired_keys.contains(&key) {
                    continue;
                }

                // Scope drift by the `priority` file: a managed package whose backend this
                // host no longer lists is left alone, not reaped. Empty scope = every
                // backend (the imperative paths, which act on exactly what they were given).
                if !self.backend_enabled(&pkg.backend) {
                    continue;
                }

                // Protection applies to EVERY removal reason, not only drift. A lease
                // expiring on `apt:dpkg`, or a bloatware file naming it, is a mistake in
                // the input — not a licence to remove it. Checked once here rather than
                // per-branch, which is how the lease and bloatware paths came to skip it.
                if self.config.is_protected(&pkg.name) {
                    debug!(
                        "Planner: '{}' is protected — never scheduling removal.",
                        key
                    );
                    continue;
                }

                // Check for expired lease
                let is_expired = pkg.expires_at.is_some_and(|exp| Self::now() >= exp);

                if is_expired {
                    info!(
                        "Planner: Lease for '{}' expired, not in desired. Scheduling removal.",
                        key
                    );
                    changes.removal_tracker.insert(key.clone());
                    changes.graph.add_node(GraphAction::Remove {
                        name: pkg.name.clone(),
                        backend: pkg.backend.clone(),
                    });
                } else {
                    // Drift: LiNix manages it and nothing declares it any more. Removing
                    // that is what sync IS (V.34) — not a mode, not a second command with
                    // the install half amputated.
                    //
                    // `protect_imperative` used to guard this branch, because an imperative
                    // install had no line and so read as drift the moment it was recorded.
                    // It has a line now (`modules/imperative.txt`), so it is declared like
                    // everything else and the setting protected against a bug that no
                    // longer exists (II.17).
                    debug!("Planner: Scheduling drift removal: {}", key);
                    changes.removal_tracker.insert(key.clone());
                    changes.graph.add_node(GraphAction::Remove {
                        name: pkg.name.clone(),
                        backend: pkg.backend.clone(),
                    });
                }
            }

        } else {
            debug!(
                "Planner: Scoped plan ({:?}) — skipping all removal planning (non-destructive).",
                scope
            );
        }

        // Installations and dependency graph
        let target_specs = self.identify_needed_actions(&expanded_desired).await?;
        self.build_execution_graph(&mut changes, &target_specs)
            .await?;

        if is_cyclic_directed(&changes.graph) {
            return Err(Error::Transaction(
                "Circular dependency detected in graph construction.".into(),
            ));
        }

        Ok(changes)
    }

    fn apply_scope_filtering(
        &self,
        desired: &HashMap<String, Vec<PackageSpec>>,
        scope: Option<&Scope>,
    ) -> HashMap<String, Vec<PackageSpec>> {
        let Some(scope) = scope else {
            return desired.clone();
        };
        let wanted = match scope {
            Scope::Profile(p) => format!("profile:{}", p),
            Scope::Module(m) => format!("module:{}", m.to_lowercase()),
        };
        let mut filtered = HashMap::new();
        for (backend, specs) in desired {
            let matched: Vec<PackageSpec> = specs
                .iter()
                .filter(|s| {
                    s.options
                        .get("__scopes")
                        .is_some_and(|src| Self::in_scope(src, &wanted))
                })
                .cloned()
                .collect();
            if !matched.is_empty() {
                filtered.insert(backend.clone(), matched);
            }
        }
        filtered
    }

    /// Whether a package's `__scopes` tag holds this exact scope.
    ///
    /// The resolver writes every scope a package belongs to, `;`-joined — `module:dev` and
    /// `profile:Work` both, for a package a module holds and a profile reaches. The match
    /// is on the whole segment, never a substring: `module:dev` must not match
    /// `module:dev-tools`.
    fn in_scope(scopes: &str, wanted: &str) -> bool {
        scopes.split(';').any(|s| s.trim() == wanted)
    }

    async fn identify_needed_actions(
        &self,
        expanded: &HashMap<String, PackageSpec>,
    ) -> Result<Vec<PackageSpec>> {
        let mut targets = Vec::new();
        for spec in expanded.values() {
            let b_cap = self
                .registry
                .get(&spec.backend)
                .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;
            let is_missing = if let Some(q) = b_cap.as_queryable() {
                match q.info(&spec.name).await {
                    Ok(Some(p)) => {
                        // A held package that is already installed is frozen: never schedule an
                        // upgrade/version change for it, even if a manifest asks for a newer
                        // version. (Hold does not block a first install of an absent package.)
                        if self.state.is_held(&spec.backend, &spec.name) {
                            false
                        } else if let Some(req_v) = spec.options.get("version") {
                            p.version
                                .as_deref()
                                .is_none_or(|inst_v| !self.satisfies_constraint(inst_v, req_v))
                        } else {
                            if spec.backend == "link"
                                && spec.options.get("template") == Some(&"true".into())
                            {
                                self.template_needs_update(spec).await
                            } else {
                                false
                            }
                        }
                    }
                    _ => true,
                }
            } else {
                true
            };
            if is_missing {
                targets.push(spec.clone());
            }
        }
        Ok(targets)
    }

    async fn build_execution_graph(
        &self,
        changes: &mut SyncChanges,
        targets: &[PackageSpec],
    ) -> Result<()> {
        for spec in targets {
            let key = format!("{}:{}", spec.backend, spec.name);
            let idx = changes.graph.add_node(GraphAction::Install(spec.clone()));
            changes.install_map.insert(key, idx);
        }
        for spec in targets {
            let child_key = format!("{}:{}", spec.backend, spec.name);
            let child_idx = *changes.install_map.get(&child_key).ok_or_else(|| {
                Error::Transaction(format!("Consistency Error: Node {} missing.", child_key))
            })?;
            for req in &spec.requires {
                if let Some(&parent_idx) = changes.install_map.get(req) {
                    changes.graph.add_edge(parent_idx, child_idx, ());
                }
            }
            if let Some(b) = self.registry.get(&spec.backend) {
                if let Some(p) = b.as_metadata_provider() {
                    if let Ok(native_deps) = p.get_dependencies(&spec.name).await {
                        for dep in native_deps {
                            let dep_key = format!("{}:{}", spec.backend, dep);
                            if let Some(&parent_idx) = changes.install_map.get(&dep_key) {
                                changes.graph.add_edge(parent_idx, child_idx, ());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Expand each declared package with its *direct* native dependencies (one level).
    ///
    /// We deliberately do NOT recurse into dependencies-of-dependencies. Every real
    /// package manager resolves and installs the full transitive closure itself at install
    /// time, so LiNix re-deriving it is redundant — and doing it recursively is actively
    /// dangerous: for a backend whose `depends` query answers from a local cache (e.g.
    /// apt), walking the whole tree fans out into hundreds of subprocess calls and hangs
    /// `status`/`sync`. One level is enough to order any co-declared packages in the graph.
    /// (Backends that self-resolve set `depends_args: None` and contribute no extra nodes.)
    async fn expand_transitive_dependencies(
        &self,
        desired: &HashMap<String, Vec<PackageSpec>>,
    ) -> Result<HashMap<String, PackageSpec>> {
        let mut expanded: HashMap<String, PackageSpec> = HashMap::new();

        // Seed with the user-declared specs (roots).
        let mut roots: Vec<PackageSpec> = Vec::new();
        for specs in desired.values() {
            for spec in specs {
                let key = format!("{}:{}", spec.backend, spec.name);
                if expanded.insert(key, spec.clone()).is_none() {
                    roots.push(spec.clone());
                }
            }
        }

        // Add each root's DIRECT native dependencies as install nodes (no recursion).
        for spec in &roots {
            let Some(b) = self.registry.get(&spec.backend) else {
                continue;
            };
            let Some(p) = b.as_metadata_provider() else {
                continue;
            };
            if let Ok(deps) = p.get_dependencies(&spec.name).await {
                for dep in deps {
                    let dep_key = format!("{}:{}", spec.backend, dep);
                    expanded.entry(dep_key).or_insert_with(|| PackageSpec {
                        name: dep,
                        backend: spec.backend.clone(),
                        options: HashMap::new(),
                        requires: Vec::new(),
                        present: true,
                    });
                }
            }
        }
        Ok(expanded)
    }

    fn satisfies_constraint(&self, installed: &str, constraint: &str) -> bool {
        if constraint == "latest" || constraint == "*" || constraint.is_empty() {
            return true;
        }
        if let Ok(req) = VersionReq::parse(constraint) {
            if let Ok(ver) = Version::parse(installed) {
                return req.matches(&ver);
            }
        }
        if installed == constraint {
            return true;
        }
        match loose_compare(installed, constraint) {
            Ok(Cmp::Eq) => true,
            Ok(Cmp::Gt) if constraint.starts_with('>') => true,
            _ => false,
        }
    }

    async fn template_needs_update(&self, spec: &PackageSpec) -> bool {
        let target = match spec.options.get("target") {
            Some(s) => Path::new(s),
            None => return true,
        };
        let source = Path::new(&spec.name);
        if !tokio::fs::try_exists(target).await.unwrap_or(false) {
            return true;
        }
        let s_hash = crate::core::security::generate_checksum(source);
        let t_hash = crate::core::security::generate_checksum(target);
        match (s_hash, t_hash) {
            (Ok(s), Ok(t)) => s != t,
            _ => true,
        }
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::state::ManagedPackage;
    use std::path::PathBuf;

    fn managed(name: &str, backend: &str) -> ManagedPackage {
        ManagedPackage {
            name: name.into(),
            backend: backend.into(),
            version: None,
            installed_at: 0,
            expires_at: None,
            options: HashMap::new(),
            source: None,
            is_transient: false,
            session_id: None,
        }
    }

    // Regression guard for the data-loss-class bug: a scoped upgrade must never
    // schedule removals for packages outside the scope. An unscoped sync still does.
    #[tokio::test]
    async fn scoped_plan_is_non_destructive() {
        let registry = Arc::new(BackendRegistry::new());
        let config = Config::default();
        let mut state = StateRegistry::new(PathBuf::from("test-state.json"));
        // A managed package that is NOT in the (empty) desired state == drift.
        state
            .packages
            .push(managed("drift-pkg-xyz", "generic-test"));

        let desired: HashMap<String, Vec<PackageSpec>> = HashMap::new();

        // Unscoped: drift removal IS planned.
        let unscoped = {
            let planner = ChangePlanner::new(registry.clone(), &state, &config);
            planner.plan(&desired, None).await.unwrap()
        };
        assert_eq!(
            unscoped.total_remove(),
            1,
            "unscoped sync should remove drift"
        );

        // Scoped: NO removals, regardless of drift.
        let scoped = {
            let planner = ChangePlanner::new(registry.clone(), &state, &config);
            planner
                .plan(&desired, Some(Scope::Module("dev".into())))
                .await
                .unwrap()
        };
        assert_eq!(
            scoped.total_remove(),
            0,
            "scoped upgrade must never remove packages"
        );
    }

    fn absent_spec(name: &str, backend: &str) -> PackageSpec {
        PackageSpec {
            name: name.into(),
            backend: backend.into(),
            present: false,
            ..PackageSpec::default()
        }
    }

    /// `absent:` shares the desired-state map with wishes, because the map type is the
    /// seam. Everything downstream of `plan` reads that map as a wish list, so an absent
    /// declaration that survives into it gets INSTALLED — the exact opposite of what the
    /// line says.
    #[tokio::test]
    async fn an_absent_declaration_is_never_installed() {
        let registry = Arc::new(BackendRegistry::new());
        let config = Config::default();
        let state = StateRegistry::new(PathBuf::from("test-state.json"));
        let desired: HashMap<String, Vec<PackageSpec>> = [(
            "generic-test".to_string(),
            vec![absent_spec("libreoffice", "generic-test")],
        )]
        .into_iter()
        .collect();

        let changes = ChangePlanner::new(registry, &state, &config)
            .plan(&desired, None)
            .await
            .unwrap();

        assert_eq!(
            changes.total_install(),
            0,
            "an `absent:` line must never become an install"
        );
    }

    /// V.7: `absent:` is the one exception to "LiNix only removes what it manages" —
    /// because you named it. So it is scheduled even though the registry never owned it.
    #[tokio::test]
    async fn an_absent_declaration_is_scheduled_for_removal_even_if_unmanaged() {
        let registry = Arc::new(BackendRegistry::new());
        let config = Config::default();
        let state = StateRegistry::new(PathBuf::from("test-state.json"));
        let desired: HashMap<String, Vec<PackageSpec>> = [(
            "generic-test".to_string(),
            vec![absent_spec("libreoffice", "generic-test")],
        )]
        .into_iter()
        .collect();

        let changes = ChangePlanner::new(registry, &state, &config)
            .plan(&desired, None)
            .await
            .unwrap();

        assert!(changes
            .removal_tracker
            .contains("generic-test:libreoffice"));
    }

    /// A scoped run is non-destructive, and that must hold for `absent:` too.
    #[tokio::test]
    async fn a_scoped_run_does_not_act_on_absent_declarations() {
        let registry = Arc::new(BackendRegistry::new());
        let config = Config::default();
        let state = StateRegistry::new(PathBuf::from("test-state.json"));
        let desired: HashMap<String, Vec<PackageSpec>> = [(
            "generic-test".to_string(),
            vec![absent_spec("libreoffice", "generic-test")],
        )]
        .into_iter()
        .collect();

        let changes = ChangePlanner::new(registry, &state, &config)
            .plan(&desired, Some(Scope::Module("dev".into())))
            .await
            .unwrap();

        assert_eq!(changes.total_remove(), 0);
        assert_eq!(changes.total_install(), 0);
    }

    #[tokio::test]
    async fn sync_removes_what_it_manages_and_you_stopped_declaring() {
        // V.34: sync removes drift BY DEFINITION. `prune_on_sync` made that a setting, so
        // sync could be configured into something that is not sync — and `linix prune` was
        // sync with the install half amputated.
        let registry = Arc::new(BackendRegistry::new());
        let config = Config::default();
        let mut state = StateRegistry::new(PathBuf::from("test-state.json"));
        state
            .packages
            .push(managed("drift-pkg-xyz", "generic-test"));
        let desired: HashMap<String, Vec<PackageSpec>> = HashMap::new();

        let changes = ChangePlanner::new(registry.clone(), &state, &config)
            .plan(&desired, None)
            .await
            .unwrap();
        assert_eq!(
            changes.total_remove(),
            1,
            "a managed package nothing declares is drift, and removing it is what sync is"
        );
    }

    #[tokio::test]
    async fn an_imperative_install_is_ordinary_drift_once_nothing_declares_it() {
        // `protect_imperative` existed because an imperative install had no line, so it
        // read as drift the moment it was recorded. It has a line now
        // (`modules/imperative.txt`), so it is declared like everything else — and if that
        // line is gone, so is the reason to keep the package (II.17).
        let registry = Arc::new(BackendRegistry::new());
        let config = Config::default();
        let mut state = StateRegistry::new(PathBuf::from("test-state.json"));
        let mut imp = managed("my-imperative-tool", "generic-test");
        imp.source = Some("imperative".into());
        state.packages.push(imp);
        let desired: HashMap<String, Vec<PackageSpec>> = HashMap::new();

        let changes = ChangePlanner::new(registry.clone(), &state, &config)
            .plan(&desired, None)
            .await
            .unwrap();
        assert_eq!(changes.total_remove(), 1);
        assert_eq!(changes.removals_only().total_remove(), 1);
    }

    #[tokio::test]
    async fn sync_never_removes_what_it_does_not_manage() {
        // II.7: what LiNix may remove is what it manages and you stopped declaring, plus
        // `absent:`. Nothing else, ever. `prune_scope = "system"` was a setting that broke
        // that rule — a routine sync deleting software it never installed (V.21). It is
        // `purge-unmanaged` instead: a command you type, not a mode you inherit.
        let registry = Arc::new(BackendRegistry::new());
        let config = Config::default();
        // Nothing managed, nothing desired: an untouched machine full of software.
        let state = StateRegistry::new(PathBuf::from("test-state.json"));
        let desired: HashMap<String, Vec<PackageSpec>> = HashMap::new();

        let changes = ChangePlanner::new(registry.clone(), &state, &config)
            .plan(&desired, None)
            .await
            .unwrap();
        assert_eq!(
            changes.total_remove(),
            0,
            "sync must never reach outside what it manages"
        );
    }

    #[test]
    fn scope_match_is_exact_segment() {
        assert!(ChangePlanner::in_scope("module:dev", "module:dev"));

        // Never a substring: `--module dev` must not sweep up `dev-tools`, and a scoped
        // upgrade acting on a package nobody named is the shape of the bug this repo is
        // named for.
        assert!(!ChangePlanner::in_scope("module:dev-tools", "module:dev"));
        assert!(!ChangePlanner::in_scope("module:dev", "module:dev-tools"));

        // A package belongs to every scope that declared it: the module that holds it and
        // the profile that reaches it.
        assert!(ChangePlanner::in_scope(
            "module:dev;profile:Work",
            "profile:Work"
        ));
        assert!(ChangePlanner::in_scope(
            "module:dev;profile:Work",
            "module:dev"
        ));
        assert!(!ChangePlanner::in_scope(
            "module:dev;profile:Work",
            "profile:Home"
        ));
    }
}
