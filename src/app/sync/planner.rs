// src/app/sync/planner.rs

use crate::backends::BackendRegistry;
use crate::config::grammar::Origin;
use crate::config::Config;
use crate::core::{Error, GraphAction, PackageSpec, Result, StateRegistry};
use crate::model::cycle::{self, Hop};
use petgraph::algo::{is_cyclic_directed, tarjan_scc};
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
            bucket
                .entry(backend.clone())
                .or_default()
                .push(spec.clone());
        }
    }
    (wanted, unwanted)
}

/// `backend:name` for a graph node.
fn node_key(action: &GraphAction) -> String {
    match action {
        GraphAction::Install(spec) => format!("{}:{}", spec.backend, spec.name),
        GraphAction::Remove { name, backend } => format!("{}:{}", backend, name),
    }
}

/// The line a node was declared on, so the loop can name it (II.7 wants the file and line of
/// every edge). `__source` is the resolver's answer to that question; a node with none came
/// from a command line and has no file to name.
fn node_origin(action: &GraphAction) -> Origin {
    match action {
        GraphAction::Install(spec) => spec
            .options
            .get("__source")
            .and_then(|s| s.parse::<Origin>().ok())
            .unwrap_or_else(Origin::argument),
        GraphAction::Remove { .. } => Origin::argument(),
    }
}

/// The `@requires` loop, in II.7's shape: every file and line, in the order the edges point.
///
/// **The same error a `use` loop gets**, through the same renderer — II.7 calls them one
/// error, and two spellings of it is how the second one goes stale. The walk differs
/// (Tarjan over the plan graph, rather than the path the resolver was already tracking)
/// because the graph is packages, not files, and it is built before anything looks for a
/// loop.
fn describe_cycle(graph: &StableDiGraph<GraphAction, ()>) -> String {
    let loop_nodes: Vec<_> = tarjan_scc(graph)
        .into_iter()
        // tarjan_scc yields reverse-topological order; reverse it so the chain reads the way
        // the `requires` edges point.
        .map(|scc| scc.into_iter().rev().collect::<Vec<_>>())
        .find(|scc| scc.len() > 1)
        // A self-loop is its own SCC of one, so it is found separately: II.7's one-element
        // case, not a special case.
        .or_else(|| {
            graph
                .node_indices()
                .find(|&idx| graph.find_edge(idx, idx).is_some())
                .map(|idx| vec![idx])
        })
        .unwrap_or_default();

    if loop_nodes.is_empty() {
        return "a set of packages that each require the next".to_string();
    }

    let keys: Vec<String> = loop_nodes.iter().map(|&i| node_key(&graph[i])).collect();
    let hops: Vec<Hop> = loop_nodes
        .iter()
        .enumerate()
        .map(|(i, &idx)| {
            let next = &keys[(i + 1) % keys.len()];
            Hop::new(
                node_origin(&graph[idx]),
                format!("{} requires {}", keys[i], next),
            )
        })
        .collect();

    cycle::describe(
        "packages require each other in a loop",
        &hops,
        keys.first().map(String::as_str).unwrap_or(""),
    )
}

/// Whether a declared size or quota disagrees with what the backend reports (Q19).
///
/// Three answers, because the backends report three states. A **byte count** is compared by
/// value, so `@quota=10240M` against a reported `10737418240` is not a change. **`none`** is the
/// backend saying it looked and there is no limit, which against a line that declares one is
/// drift. **Nothing at all** is the backend saying it could not look, and that is left alone —
/// D13's rule, and the reason it exists: a value read as "no limit" whenever the read fails
/// schedules the same change on every sync for ever.
fn limit_drifted(want: &str, reported: Option<&String>) -> bool {
    match reported.map(String::as_str) {
        None => false,
        Some(crate::backends::storage::NO_LIMIT) => true,
        Some(bytes) => bytes
            .parse::<u64>()
            .is_ok_and(|b| !crate::core::same_size(want, b)),
    }
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
        // Sort for a stable, readable plan: the graph's node order follows dependency edges
        // and a HashMap crawl, so without this the same change set prints in a different order
        // each run. This is display only — execution still follows the graph's topology.
        let key = |e: &ReportEntry| (e.backend.clone(), e.name.clone());
        report.install.sort_by_key(key);
        report.remove.sort_by_key(key);
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

    /// What each named backend reports as installed, asked **once per backend**.
    ///
    /// Removal planning needs to know whether a package is actually there, for as many
    /// packages as the manifest and the registry hold between them. Asking per package would
    /// be one subprocess each; asking per backend is one, and the answer is a set.
    ///
    /// A backend that cannot be queried, or that fails, is absent from the map — and
    /// [`is_installed`](Self::is_installed) treats that as "assume it is there", preserving
    /// exactly the behaviour that existed before this check: schedule the removal and let it
    /// report its own failure. Not knowing must never turn into "so skip it", or a backend
    /// having a bad day silently stops LiNix removing anything through it.
    async fn installed_sets(
        &self,
        backends: &std::collections::BTreeSet<String>,
    ) -> HashMap<String, HashSet<String>> {
        use futures::stream::{self, StreamExt};

        stream::iter(backends.iter().cloned())
            .map(|backend| {
                let registry = self.registry.clone();
                async move {
                    let b_cap = registry.get(&backend)?;
                    let installed = b_cap.as_queryable()?.list_installed().await.ok()?;
                    Some((
                        backend,
                        installed
                            .into_iter()
                            .map(|p| p.name)
                            .collect::<HashSet<_>>(),
                    ))
                }
            })
            .buffer_unordered(8)
            .filter_map(|r| async move { r })
            .collect()
            .await
    }

    /// Whether this package is actually on the machine, per the sets gathered above.
    ///
    /// **Unknown means yes.** A backend that could not answer must not have its removals
    /// silently dropped — see [`installed_sets`](Self::installed_sets).
    fn is_installed(sets: &HashMap<String, HashSet<String>>, backend: &str, name: &str) -> bool {
        sets.get(backend).is_none_or(|set| set.contains(name))
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
            // Removing something that is not there is not a change — it is a command that
            // fails every time it runs. `absent:jq` on a machine that has never had jq made
            // every sync fail, permanently, with an error from the package manager about a
            // package it does not have.
            let consulted: std::collections::BTreeSet<String> = unwanted
                .iter()
                .filter(|(_, specs)| !specs.is_empty())
                .map(|(backend, _)| backend.clone())
                .collect();
            let installed = self.installed_sets(&consulted).await;

            // `absent:` — the one thing LiNix removes that it does not manage, because
            // you named it (V.7). Scheduled whether or not LiNix *installed* it, which is
            // the point of the rule; not scheduled when it is not there, which is not a
            // removal at all. The guard still decides whether it may actually go (Phase 3).
            for (backend, specs) in &unwanted {
                for spec in specs {
                    let key = format!("{}:{}", backend, spec.name);
                    if changes.removal_tracker.contains(&key) {
                        continue;
                    }
                    if !Self::is_installed(&installed, backend, &spec.name) {
                        debug!("'{}' is declared absent and is already absent.", key);
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
                    debug!("'{}' is protected — never scheduling removal.", key);
                    continue;
                }

                // NOT gated on "is it still installed", deliberately — unlike the `absent:`
                // loop above. A managed package that has vanished from the machine still has a
                // registry entry, and the removal is what *drops* that entry: skipping it here
                // would leave LiNix permanently claiming to manage something that is gone,
                // which is a quieter wrong state than the failed removal it would avoid.
                // Reconciling a stale entry is `heal`'s job, not the planner's.

                // Check for expired lease
                let is_expired = pkg.expires_at.is_some_and(|exp| Self::now() >= exp);

                if is_expired {
                    info!(
                        "Lease for '{}' expired, not in desired. Scheduling removal.",
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
                    debug!("Scheduling drift removal: {}", key);
                    changes.removal_tracker.insert(key.clone());
                    changes.graph.add_node(GraphAction::Remove {
                        name: pkg.name.clone(),
                        backend: pkg.backend.clone(),
                    });
                }
            }
        } else {
            debug!(
                "Scoped plan ({:?}) — skipping all removal planning (non-destructive).",
                scope
            );
        }

        // Installations and dependency graph
        let target_specs = self.identify_needed_actions(&expanded_desired).await?;
        self.build_execution_graph(&mut changes, &target_specs)
            .await?;

        if is_cyclic_directed(&changes.graph) {
            return Err(Error::Transaction(format!(
                "`requires` forms a cycle — these packages each wait for the next, so none \
                 can go first: {}. Break the loop by removing one `requires` edge.",
                describe_cycle(&changes.graph)
            )));
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
        use futures::stream::{self, StreamExt, TryStreamExt};

        // Each spec's "is it already installed?" check is a separate query — usually a process
        // spawn (`apt list <pkg>`, `brew info <pkg>`). Done one after another this is the
        // dominant cost of `sync`/`status`/`plan` on a large config. Overlap the waits, capped
        // at `max_parallel`; the futures borrow `&self` so this stays on one task (no spawn),
        // which is all that is needed since the time is spent waiting on child processes.
        let cap = self.config.max_parallel.max(1);
        let needed: Vec<PackageSpec> = stream::iter(expanded.values())
            .map(|spec| async move {
                Ok::<_, Error>(self.spec_is_missing(spec).await?.then(|| spec.clone()))
            })
            .buffer_unordered(cap)
            .try_filter_map(|opt| async move { Ok(opt) })
            .try_collect()
            .await?;
        Ok(needed)
    }

    /// Whether one desired spec needs an install/change action: absent, or present but not
    /// satisfying a `@version=`, or a template whose rendered content has drifted. Held-and-
    /// present packages are frozen. Extracted so the fan-out in `identify_needed_actions` and
    /// the decision are one thing described once.
    async fn spec_is_missing(&self, spec: &PackageSpec) -> Result<bool> {
        let b_cap = self
            .registry
            .get(&spec.backend)
            .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;
        let Some(q) = b_cap.as_queryable() else {
            return Ok(true);
        };
        let installed = match q.info(&spec.name).await {
            Ok(Some(p)) => p,
            Ok(None) => return Ok(true),
            // "I could not ask" is not "it is not installed". Read as absence it schedules an
            // Install node for every managed package — each one a trivial success that lands in
            // the transaction's history, so a single later failure rolls back across the whole
            // set. `search_output` already draws this distinction for the same reason (V.7c).
            Err(e) => {
                return Err(Error::Other(format!(
                    "`{}` could not say whether {} is installed, so LiNix cannot tell what \
                     needs doing: {}",
                    spec.backend, spec.name, e
                )))
            }
        };
        // A held package that is already installed is frozen: never schedule an upgrade or
        // version change for it, even if a manifest asks for a newer version. (Hold does not
        // block a first install of an absent package.)
        if self.state.is_held(&spec.backend, &spec.name) {
            return Ok(false);
        }
        if let Some(req_v) = spec.options.get("version") {
            return Ok(installed
                .version
                .as_deref()
                .is_none_or(|inst_v| !self.satisfies_constraint(inst_v, req_v)));
        }
        // D13: a `@channel` that differs from what the package is following needs a refresh —
        // otherwise a channel change is invisible and does nothing. Only acts when the current
        // channel is *readable*: a channel we cannot read is left alone rather than refreshed
        // on every sync, which would be worse than the drift it is meant to catch.
        let mut drifted = false;
        if let Some(want) = spec.options.get("channel") {
            use crate::backends::capability::channel_risk;
            if let Some(current) = installed.properties.get("channel") {
                drifted |= channel_risk(current) != channel_risk(want);
            }
        }
        // Q20: `@classic` is confinement, and it was applied at install and never again — a snap
        // that gained the option after it was installed stayed strictly confined for ever, with
        // `sync` reporting nothing to do. The same shape as `@quota`, on a different backend.
        //
        // Absent means unmanaged, exactly as it does for a declared quota: a line that says
        // nothing about confinement is not asking for strict, so it never schedules the
        // remove-and-reinstall that narrowing would take. Only an explicit `@classic=false`
        // does, and the backend refuses it by name rather than removing a declared package.
        if let Some(want) = spec.options.get("classic") {
            if let Some(current) = installed.properties.get("classic") {
                drifted |= current != want;
            }
        }
        // Q18: a declared storage object that is not mounted where the line says is drift, the
        // same shape as `@channel` above. Without this a `@mount=` that failed — or one the
        // machine lost — is invisible for ever: the subvolume exists, so the name is present, so
        // `sync` says "already up to date" over a declaration it never finished applying.
        // Measured, on a real filesystem: an install whose mount half failed reported nothing
        // wrong on every subsequent run.
        //
        // **Mounted nowhere is a state, not an unknown.** D13 leaves an unreadable value alone,
        // and the first draft of this rule copied that — which put the motivating case straight
        // back: the failed mount reports no mountpoint at all, so "no property" had to mean "not
        // where the line says" or the declaration would never converge. Re-applying is
        // idempotent (`mount`, `zfs set mountpoint=`), so the cost of being wrong here is a
        // repeated no-op, while the cost of the other reading is a mount that never happens.
        //
        // Q19: and every other facet of that geometry is checked beside it, with the answers
        // OR-ed rather than returned. `@mount` used to `return` from here, so a line carrying
        // both a mount and a quota had only the mount looked at — the second option was dead the
        // moment somebody wrote the two together. `@channel` above had the identical fault and
        // is folded into the same accumulator (Q20).
        if let Some(want) = spec.options.get("mount") {
            let current = installed.properties.get("mount").map(String::as_str);
            drifted |= current.map(|c| c.trim_end_matches('/')) != Some(want.trim_end_matches('/'));
        }
        // The option field of the fstab entry `@mount` wrote. Editing it and finding nothing
        // happens is the same defect as an editable `@quota` that never re-applies: the entry
        // on disk keeps yesterday's options and the next boot honours them.
        if let Some(want) = spec.options.get("mount_options") {
            if let Some(current) = installed.properties.get("mount_options") {
                drifted |= current != want;
            }
        }
        for key in ["quota", "size"] {
            if let Some(want) = spec.options.get(key) {
                drifted |= limit_drifted(want, installed.properties.get(key));
            }
        }
        if drifted {
            return Ok(true);
        }
        if spec.backend == "link" && spec.options.get("template") == Some(&"true".into()) {
            return Ok(self.template_needs_update(spec).await);
        }
        Ok(false)
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

    #[test]
    fn a_requires_cycle_names_the_packages_and_where_they_came_from() {
        // V.45: the message must name what closed the loop, not just say one exists.
        let mut graph: StableDiGraph<GraphAction, ()> = StableDiGraph::new();
        let mk = |name: &str, src: &str| {
            let mut options = HashMap::new();
            options.insert("__source".to_string(), src.to_string());
            GraphAction::Install(PackageSpec {
                name: name.into(),
                backend: "apt".into(),
                options,
                requires: vec![],
                present: true,
            })
        };
        let a = graph.add_node(mk("foo", "modules/dev.txt:3"));
        let b = graph.add_node(mk("bar", "modules/dev.txt:4"));
        graph.add_edge(a, b, ());
        graph.add_edge(b, a, ());

        // II.7: a `requires` loop owes the same error a `use` loop does — every file and
        // line, in the order the edges point, and the arrow back to where it started.
        let msg = describe_cycle(&graph);
        assert!(
            msg.contains("modules/dev.txt:3  apt:foo requires apt:bar"),
            "{}",
            msg
        );
        assert!(
            msg.contains("modules/dev.txt:4  apt:bar requires apt:foo"),
            "{}",
            msg
        );
        assert!(msg.trim_end().ends_with("^ back to apt:foo"), "{}", msg);
    }

    #[test]
    fn a_package_requiring_itself_is_named() {
        let mut graph: StableDiGraph<GraphAction, ()> = StableDiGraph::new();
        let n = graph.add_node(GraphAction::Remove {
            name: "loop".into(),
            backend: "apt".into(),
        });
        graph.add_edge(n, n, ());
        // The one-element case, in the same shape as every other loop.
        let msg = describe_cycle(&graph);
        assert!(msg.contains("apt:loop requires apt:loop"), "{}", msg);
        assert!(msg.trim_end().ends_with("^ back to apt:loop"), "{}", msg);
    }

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

        assert!(changes.removal_tracker.contains("generic-test:libreoffice"));
    }

    /// A backend that reports exactly what it was told is installed. Enough to answer the one
    /// question removal planning asks — *is it actually on the machine?* — which an empty
    /// registry cannot, and which is why this bug survived the tests above it.
    struct FakeInstalled {
        name: String,
        installed: Vec<String>,
    }

    #[async_trait::async_trait]
    impl crate::core::manager::BackendCore for FakeInstalled {
        fn name(&self) -> &str {
            &self.name
        }
        fn is_available(&self) -> bool {
            true
        }
        fn probes(&self) -> Vec<String> {
            Vec::new()
        }
        fn needs_root(&self) -> bool {
            false
        }
    }

    #[async_trait::async_trait]
    impl crate::core::manager::Queryable for FakeInstalled {
        async fn list_installed(&self) -> Result<Vec<crate::core::Package>> {
            Ok(self
                .installed
                .iter()
                .map(|n| crate::core::Package {
                    name: n.clone(),
                    backend: self.name.clone(),
                    version: None,
                    properties: HashMap::new(),
                })
                .collect())
        }
        async fn list_manual(&self) -> Result<Vec<crate::core::Package>> {
            self.list_installed().await
        }
        async fn info(&self, name: &str) -> Result<Option<crate::core::Package>> {
            Ok(self
                .list_installed()
                .await?
                .into_iter()
                .find(|p| p.name == name))
        }
    }

    fn registry_reporting(backend: &str, installed: &[&str]) -> Arc<BackendRegistry> {
        let fake = Arc::new(FakeInstalled {
            name: backend.to_string(),
            installed: installed.iter().map(|s| s.to_string()).collect(),
        });
        let mut registry = BackendRegistry::new();
        registry.register(Arc::new(
            crate::core::manager::BackendCapabilities::builder(fake.clone())
                .with_queryable(fake)
                .build(),
        ));
        Arc::new(registry)
    }

    async fn absent_removals(registry: Arc<BackendRegistry>, name: &str) -> usize {
        let config = Config::default();
        let state = StateRegistry::new(PathBuf::from("test-state.json"));
        let desired: HashMap<String, Vec<PackageSpec>> = [(
            "generic-test".to_string(),
            vec![absent_spec(name, "generic-test")],
        )]
        .into_iter()
        .collect();
        ChangePlanner::new(registry, &state, &config)
            .plan(&desired, None)
            .await
            .unwrap()
            .total_remove()
    }

    /// **The bug:** `absent:` scheduled a removal whether or not the package was there, so a
    /// machine that had never had it failed every single sync — the package manager refusing
    /// to remove something it does not have, forever, with no way to converge.
    #[tokio::test]
    async fn an_absent_declaration_for_something_not_installed_is_not_a_removal() {
        let registry = registry_reporting("generic-test", &["something-else"]);
        assert_eq!(
            absent_removals(registry, "libreoffice").await,
            0,
            "removing what is not there is not a change, it is a command that always fails"
        );
    }

    /// The other half of the same rule: when it IS there, `absent:` still removes it. A fix
    /// that made `absent:` a no-op would pass the test above and destroy the feature.
    #[tokio::test]
    async fn an_absent_declaration_for_something_installed_is_still_a_removal() {
        let registry = registry_reporting("generic-test", &["libreoffice"]);
        assert_eq!(absent_removals(registry, "libreoffice").await, 1);
    }

    /// A backend that cannot answer must not have its removals silently dropped. Unknown
    /// means "assume it is there" — the behaviour that existed before the check — so a
    /// backend having a bad day cannot quietly disable `absent:`.
    #[tokio::test]
    async fn a_backend_that_cannot_be_queried_still_plans_the_removal() {
        // An empty registry: no backend, so no installed set, so no answer.
        let changes = absent_removals(Arc::new(BackendRegistry::new()), "libreoffice").await;
        assert_eq!(changes, 1, "not knowing must never mean not removing");
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
    /// The plan is displayed in a stable, sorted order regardless of how the graph was built
    /// — the node order follows dependency edges and a HashMap crawl, so without the sort in
    /// `generate_report` the same change set printed differently each run.
    #[test]
    fn the_report_is_sorted_for_a_stable_plan() {
        use petgraph::stable_graph::StableDiGraph;
        let ins = |name: &str, backend: &str| {
            GraphAction::Install(PackageSpec {
                name: name.into(),
                backend: backend.into(),
                options: HashMap::new(),
                requires: vec![],
                present: true,
            })
        };
        let mut graph: StableDiGraph<GraphAction, ()> = StableDiGraph::new();
        // Add out of order, across backends.
        graph.add_node(ins("zsh", "apt"));
        graph.add_node(ins("bat", "cargo"));
        graph.add_node(ins("acl", "apt"));
        graph.add_node(GraphAction::Remove {
            name: "nano".into(),
            backend: "apt".into(),
        });
        graph.add_node(GraphAction::Remove {
            name: "amp".into(),
            backend: "cargo".into(),
        });
        let changes = SyncChanges {
            graph,
            ..Default::default()
        };

        let report = changes.generate_report();
        let installs: Vec<(&str, &str)> = report
            .install
            .iter()
            .map(|e| (e.backend.as_str(), e.name.as_str()))
            .collect();
        assert_eq!(
            installs,
            vec![("apt", "acl"), ("apt", "zsh"), ("cargo", "bat")]
        );
        let removes: Vec<(&str, &str)> = report
            .remove
            .iter()
            .map(|e| (e.backend.as_str(), e.name.as_str()))
            .collect();
        assert_eq!(removes, vec![("apt", "nano"), ("cargo", "amp")]);
    }
}
