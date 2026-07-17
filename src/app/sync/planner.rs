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
    /// Whether to schedule drift removals (packages in state but not in desired).
    /// Defaults to true to preserve existing reconcile behavior; `sync` overrides this
    /// from `config.prune_on_sync` so pruning is opt-in there.
    prune: bool,
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
            prune: true,
        }
    }

    /// Control whether drift packages are scheduled for removal.
    pub fn with_prune(mut self, prune: bool) -> Self {
        self.prune = prune;
        self
    }

    #[instrument(skip(self, desired))]
    pub async fn plan(
        &self,
        desired: &HashMap<String, Vec<PackageSpec>>,
        scope: Option<Scope>,
    ) -> Result<SyncChanges> {
        let mut changes = SyncChanges::default();
        let filtered_desired = self.apply_scope_filtering(desired, scope.as_ref());
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
            // Single pass over all managed packages to schedule removals
            for pkg in &self.state.packages {
                let key = format!("{}:{}", pkg.backend, pkg.name);

                // Skip if already scheduled or present in desired state
                if changes.removal_tracker.contains(&key) || desired_keys.contains(&key) {
                    continue;
                }

                // Respect per-host and `-b` backend gating. Without this, `linix -b cargo
                // prune` still removed apt drift: `-b` narrows which backend a command
                // acts on everywhere else, so a removal loop that ignores it is scoped by
                // nothing. The system-scope loop below already checked this; this one did
                // not.
                if !self.config.is_backend_enabled(&pkg.backend) {
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
                } else if self.prune
                    && !(self.config.protect_imperative
                        && pkg.source.as_deref() == Some("imperative"))
                {
                    // Drift removal (only when pruning is enabled and — when
                    // protect_imperative is on — not an imperatively-installed package;
                    // protection was already applied to every branch above).
                    debug!("Planner: Scheduling drift removal: {}", key);
                    changes.removal_tracker.insert(key.clone());
                    changes.graph.add_node(GraphAction::Remove {
                        name: pkg.name.clone(),
                        backend: pkg.backend.clone(),
                    });
                }
            }

            // System-wide prune (opt-in): also remove packages that are INSTALLED but not
            // under LiNix management and not in the desired state — a true "make the system
            // exactly match my manifests" mode. Protected packages and LiNix itself are
            // always spared. Only runs when `prune_scope = "system"` and pruning is enabled.
            if self.prune && self.config.prune_scope == crate::config::PruneScope::System {
                for backend in self.registry.available() {
                    // Respect per-host backend gating: never remove packages from a
                    // backend this host is told not to manage.
                    if !self.config.is_backend_enabled(backend.name()) {
                        continue;
                    }
                    let Some(q) = backend.as_queryable() else {
                        continue;
                    };
                    let installed = match q.list_installed().await {
                        Ok(v) => v,
                        Err(_) => continue, // a backend that can't be queried is skipped, not fatal
                    };
                    let bname = backend.name().to_string();
                    for pkg in installed {
                        let key = format!("{}:{}", bname, pkg.name);
                        if changes.removal_tracker.contains(&key) || desired_keys.contains(&key) {
                            continue;
                        }
                        if self.config.is_protected(&pkg.name) || pkg.name == "linix" {
                            continue;
                        }
                        debug!(
                            "Planner: Scheduling system-scope removal (unmanaged drift): {}",
                            key
                        );
                        changes.removal_tracker.insert(key.clone());
                        changes.graph.add_node(GraphAction::Remove {
                            name: pkg.name.clone(),
                            backend: bname.clone(),
                        });
                    }
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
        let prefix = match scope {
            Scope::Profile(p) => format!("manifest:{}", p),
            Scope::Module(m) => format!("module:{}", m),
        };
        let mut filtered = HashMap::new();
        for (backend, specs) in desired {
            let matched: Vec<PackageSpec> = specs
                .iter()
                .filter(|s| {
                    s.options
                        .get("__source")
                        .is_some_and(|src| Self::source_matches_scope(src, &prefix))
                })
                .cloned()
                .collect();
            if !matched.is_empty() {
                filtered.insert(backend.clone(), matched);
            }
        }
        filtered
    }

    /// Exact-segment match between a package's `__source` tag and a scope prefix.
    ///
    /// Sources look like `module:dev`, `manifest:base.txt`, `group:editors`, or the
    /// composite `config:group:editors`. A scope prefix is e.g. `module:dev` or
    /// `group:editors`. We must NOT use a naive substring match (`module:dev` would
    /// then wrongly match `module:dev-tools`). Match if the source equals the prefix
    /// or ends with `:{prefix}` (so `config:group:editors` still matches `group:editors`).
    fn source_matches_scope(source: &str, prefix: &str) -> bool {
        // A source may carry multiple origins joined by ';' (see resolver tagging).
        source.split(';').any(|src| {
            let src = src.trim();
            src == prefix || src.ends_with(&format!(":{}", prefix))
        })
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

    // `with_prune(false)` (what `sync` uses when prune_on_sync is off) must NOT schedule
    // drift removals; `with_prune(true)` does. Removals are now opt-in for sync.
    #[tokio::test]
    async fn prune_flag_gates_drift_removal() {
        let registry = Arc::new(BackendRegistry::new());
        let config = Config::default();
        let mut state = StateRegistry::new(PathBuf::from("test-state.json"));
        state
            .packages
            .push(managed("drift-pkg-xyz", "generic-test"));
        let desired: HashMap<String, Vec<PackageSpec>> = HashMap::new();

        let no_prune = ChangePlanner::new(registry.clone(), &state, &config)
            .with_prune(false)
            .plan(&desired, None)
            .await
            .unwrap();
        assert_eq!(
            no_prune.total_remove(),
            0,
            "with_prune(false) must not remove drift"
        );

        let pruned = ChangePlanner::new(registry.clone(), &state, &config)
            .with_prune(true)
            .plan(&desired, None)
            .await
            .unwrap();
        assert_eq!(
            pruned.total_remove(),
            1,
            "with_prune(true) should remove drift"
        );
        // removals_only() preserves the removal
        assert_eq!(pruned.removals_only().total_remove(), 1);
    }

    // protect_imperative shields imperatively-installed packages from drift removal.
    #[tokio::test]
    async fn protect_imperative_shields_imperative_installs_from_drift() {
        let registry = Arc::new(BackendRegistry::new());
        let mut config = Config::default();
        let mut state = StateRegistry::new(PathBuf::from("test-state.json"));
        let mut imp = managed("my-imperative-tool", "generic-test");
        imp.source = Some("imperative".into());
        state.packages.push(imp);
        let desired: HashMap<String, Vec<PackageSpec>> = HashMap::new();

        // Default (protect_imperative = true): imperative drift is NOT scheduled for removal.
        config.protect_imperative = true;
        let protected = ChangePlanner::new(registry.clone(), &state, &config)
            .with_prune(true)
            .plan(&desired, None)
            .await
            .unwrap();
        assert_eq!(
            protected.total_remove(),
            0,
            "imperative install must be shielded when protect_imperative=true"
        );

        // protect_imperative = false: it becomes ordinary drift and IS scheduled.
        config.protect_imperative = false;
        let unprotected = ChangePlanner::new(registry.clone(), &state, &config)
            .with_prune(true)
            .plan(&desired, None)
            .await
            .unwrap();
        assert_eq!(
            unprotected.total_remove(),
            1,
            "imperative install is drift when protect_imperative=false"
        );
    }

    #[test]
    fn scope_match_is_exact_segment() {
        // exact
        assert!(ChangePlanner::source_matches_scope(
            "module:dev",
            "module:dev"
        ));
        // composite source still matches a bare group scope
        assert!(ChangePlanner::source_matches_scope(
            "config:group:editors",
            "group:editors"
        ));
        // must NOT substring-match a longer module name
        assert!(!ChangePlanner::source_matches_scope(
            "module:dev-tools",
            "module:dev"
        ));
        assert!(!ChangePlanner::source_matches_scope(
            "module:dev",
            "module:dev-tools"
        ));
        // multi-origin source (';'-joined) matches if any segment matches
        assert!(ChangePlanner::source_matches_scope(
            "manifest:base.txt;module:dev",
            "module:dev"
        ));
    }
}
