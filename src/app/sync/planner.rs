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
    /// What the plan left out, and why. In the JSON too: a `--json` consumer that can only see
    /// the actions cannot tell a converged machine from one holding an undeclared package
    /// nothing will ever remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReportEntry {
    pub backend: String,
    pub name: String,
    pub version: Option<String>,
    pub source: Option<String>,
}

/// Narrows a sync to one profile or module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Profile(String),
    Module(String),
}

/// The backends this host's `priority` file names.
///
/// A newtype and not a `Vec<String>`, because this list is the whole of what `priority`
/// promises — the promise is written in the error a new user reads when the file is missing:
/// *"Listed means LiNix uses it. Not listed means LiNix does not touch it at all."* A plan that
/// reaps has to be handed the list, and the only thing that can produce one is the resolver
/// that read the file.
///
/// Empty means every backend, because a host whose `priority` could not be read has said
/// nothing about which managers are its own — and the caller that got here without a readable
/// `priority` is `sync` itself, which has already failed by then.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostBackends(Vec<String>);

impl HostBackends {
    /// Mint one from what the `priority` file said, in its order.
    ///
    /// The resolver is the only caller in `src/`; `planner_scope_enumeration_tests` fails the
    /// build if a second appears, because a list assembled anywhere else is a list that agrees
    /// with `priority` only until someone edits it.
    pub fn from_priority(order: Vec<String>) -> Self {
        Self(order)
    }

    /// Whether `backend` is one this host manages.
    fn allows(&self, backend: &str) -> bool {
        self.0.is_empty() || self.0.iter().any(|b| b == backend)
    }
}

/// What a plan is computed over — and therefore what it may take away.
///
/// **One argument, and it is not optional.** This was `Option<Scope>`, where `None` carried two
/// unrelated facts: *do not filter the desired set*, and *reap every backend on the box*. Four
/// callers wanted the first and got the second thrown in — `plan`/`apply` froze removals across
/// managers `priority` never named, `upgrade --canary` and `activate` did the same, and the
/// transient shell, whose desired set holds only the packages it was asked for, planned a
/// removal for every other package on the machine. Naming the case is now the only way to get
/// a plan at all, and the case that reaps cannot be written without the list that bounds it.
#[derive(Debug, Clone)]
pub enum PlanScope {
    /// The machine's whole declaration set. Drift is real here: anything this host manages and
    /// nothing declares any more is a removal, confined to the backends `priority` names.
    Whole(HostBackends),
    /// One profile or module. `desired` still holds the whole config and is filtered down to
    /// the scope, so a package outside it is outside the question — nothing is removed.
    Narrowed(Scope),
    /// A set of packages that is not the config: a transient shell's requests. Installs only,
    /// and no filter, because the set is already exactly what was asked for.
    JustThese,
}

impl PlanScope {
    /// The scope to filter `desired` by, if any.
    fn filter(&self) -> Option<&Scope> {
        match self {
            PlanScope::Narrowed(s) => Some(s),
            PlanScope::Whole(_) | PlanScope::JustThese => None,
        }
    }

    /// The host's backends when this plan may reap, `None` when it may not.
    ///
    /// The two questions are one match, so a variant added later has to answer both before it
    /// compiles — which is what the old `Option<Scope>` could not make anyone do.
    fn reaps(&self) -> Option<&HostBackends> {
        match self {
            PlanScope::Whole(hosts) => Some(hosts),
            PlanScope::Narrowed(_) | PlanScope::JustThese => None,
        }
    }
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

/// Something the plan left out, and why.
///
/// **The reason travels with the item.** A rollup that counts skips and explains them with one
/// sentence is a sentence that is wrong for every input it does not describe — `adopt` printed
/// *"Left alone: 185 (listed in the manifest)"* about items none of which were listed in the
/// manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub key: String,
    pub reason: String,
}

/// Why a managed package was not scheduled for removal.
///
/// **The type exists so that the question "does the user hear about this?" cannot be answered by
/// omission.** Two of these were a bare `continue` — the machine kept a package nothing declared,
/// forever, and `sync`, `uninstall` and `check` all reported success over it (AU1). A variant
/// added later does not compile until [`Declined::reported`] gains an arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declined {
    /// Another rule in this plan already scheduled it. Not a decline at all — the removal is
    /// happening.
    AlreadyScheduled,
    /// Something still declares it, so there is no drift. This is convergence working.
    StillDeclared,
    /// Its backend is not in this host's `priority` file (II.6), so LiNix does not manage that
    /// manager here and will never reap through it.
    BackendNotInPriority(String),
    /// A `[guard] protected_packages` rule matched, carrying the rule that decided it.
    Protected(String),
}

impl Declined {
    /// The sentence the user is owed, or `None` when there is nothing to tell them.
    ///
    /// **The line between the two is whether the machine is left disagreeing with the files.**
    /// A package that is being removed anyway, or that is still declared, leaves nothing behind
    /// to report. The other two leave software installed that nothing declares and that no
    /// future `sync` will touch — which is a standing disagreement, and reporting it is what
    /// stops `already up to date` being a lie.
    pub fn reported(&self) -> Option<String> {
        match self {
            Self::AlreadyScheduled | Self::StillDeclared => None,
            Self::BackendNotInPriority(backend) => Some(format!(
                "`{}` is not in your `priority` file, so LiNix does not manage that backend on \
                 this host",
                backend
            )),
            Self::Protected(rule) => {
                Some(crate::app::sync::guard::Protection::Rule(rule.clone()).reason())
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SyncChanges {
    pub graph: StableDiGraph<GraphAction, ()>,
    pub install_map: HashMap<String, NodeIndex>,
    pub removal_tracker: HashSet<String>,
    /// Removals the planner declined to schedule, each with its reason.
    ///
    /// A plan that drops something silently reports success over a machine it did not change.
    /// `rebuild` has said so since it was written (`why.md:551`) and prints its skips; this is
    /// the same list on the path `rebuild`'s own comment was about — *"the same lie convergence
    /// was already telling"* (AU1). An empty plan with a non-empty `skipped` is NOT
    /// `already up to date`.
    pub skipped: Vec<Skipped>,
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
        let mut out = SyncChanges {
            // `prune` removes drift and nothing else, so a removal the planner declined is
            // precisely what its user needs told. Dropping the list here would give `prune`
            // the silence this plan just stopped having.
            skipped: self.skipped.clone(),
            ..SyncChanges::default()
        };
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
        // NOT counted as a change: a skip is work that will not happen. It is reported beside
        // the count, never inside it.
        report.skipped = self.skipped.clone();
        report
    }
}

pub struct ChangePlanner<'a> {
    registry: Arc<BackendRegistry>,
    state: &'a StateRegistry,
    config: &'a Config,
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
        }
    }

    /// Why this managed package is not being scheduled for removal, or `None` to schedule it.
    ///
    /// One function, so the drift loop has exactly one place a package can leave it from. The
    /// reasons split two ways and [`Declined::reported`] is where that split is written down.
    fn declined(
        &self,
        pkg: &crate::core::ManagedPackage,
        hosts: &HostBackends,
        scheduled: &HashSet<String>,
        desired_keys: &HashSet<String>,
    ) -> Option<Declined> {
        let key = format!("{}:{}", pkg.backend, pkg.name);
        if scheduled.contains(&key) {
            return Some(Declined::AlreadyScheduled);
        }
        if desired_keys.contains(&key) {
            return Some(Declined::StillDeclared);
        }
        if !hosts.allows(&pkg.backend) {
            return Some(Declined::BackendNotInPriority(pkg.backend.clone()));
        }
        // Protection applies to EVERY removal reason, not only drift. A lease expiring on
        // `apt:dpkg`, or a bloatware file naming it, is a mistake in the input — not a licence
        // to remove it. Checked once here rather than per-branch, which is how the lease and
        // bloatware paths came to skip it.
        //
        // The reason comes from the guard's own vocabulary rather than a sentence written here,
        // so the inspector (`linix protected`), the refusal and this skip all say the same thing
        // about the same package.
        if let Some(rule) = self.config.protection_rule(&pkg.name) {
            return Some(Declined::Protected(rule.to_string()));
        }
        None
    }

    /// What each named backend reports as installed, asked **once per backend**.
    ///
    /// Removal planning needs to know whether a package is actually there, for as many
    /// packages as the manifest and the registry hold between them. Asking per package would
    /// be one subprocess each; asking per backend is one, and the answer is a set.
    ///
    /// **This is also the wave that warms every other question the plan asks.** The fan-out in
    /// [`identify_needed_actions`](Self::identify_needed_actions) is over *specs*, and a spec's
    /// answer usually comes from its manager's whole listing — so a manifest with 256 winget
    /// lines puts 256 futures into a queue `max_parallel` slots wide, every one of them waiting
    /// on the same `winget list`, while scoop, choco and cargo sit unasked because there is no
    /// slot left to ask them from. Measured on a 298-package config: three managers started at
    /// 0.3 s and the other six at 1.9 s, waiting for a question that was never about them.
    /// Asking each manager once, here, is what makes those slots hold work instead of a queue
    /// of duplicates.
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
            // The knob, not a number: the two fan-outs on either side of this one already read
            // `max_parallel`, and a cap that ignores the setting is a cap the user cannot move.
            .buffer_unordered(self.config.max_parallel.max(1))
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
        scope: PlanScope,
    ) -> Result<SyncChanges> {
        let mut changes = SyncChanges::default();

        // `absent:` says a package must NOT exist (II.2). Split off FIRST, before any
        // other work: everything downstream of here reads `desired` as a wish list, so an
        // absent declaration left in it would be installed — the exact opposite of what it
        // says. Partitioning at the top means no later branch can misread one.
        let (wanted, unwanted) = partition_by_presence(desired);

        let filtered_desired = self.apply_scope_filtering(&wanted, scope.filter());
        let declared = Self::declared_specs(&filtered_desired);

        // Precompute desired keys for O(1) lookup
        let desired_keys: HashSet<String> = declared.keys().cloned().collect();

        // Every manager this plan will consult, asked before anything is asked about a package
        // — see `installed_sets`. Hoisted out of the removal block below because a scoped plan
        // skips that block and still asks each of these managers, one spec at a time, for the
        // listing it could have had at the start.
        let consulted: std::collections::BTreeSet<String> = declared
            .values()
            .map(|spec| spec.backend.clone())
            .chain(
                unwanted
                    .iter()
                    .filter(|(_, specs)| !specs.is_empty())
                    .map(|(backend, _)| backend.clone()),
            )
            .collect();
        let installed = self.installed_sets(&consulted).await;

        // Removal planning (drift / bloatware / expired leases) is GLOBAL: it acts on
        // every managed package not present in `desired`. That is only safe when `desired` is
        // the machine's whole declaration set. When the caller narrows to a profile or module,
        // or hands over a list that is not the config at all, a package missing from `desired`
        // is missing from the *question* — removing it would delete every package outside the
        // caller's scope. `PlanScope::reaps` is where that decision is made, once, and it is
        // the only way in here: the four callers that reached this block by passing `None` for
        // a scope they did not have are the reason it is a value and not a bare `Option`.
        if let Some(hosts) = scope.reaps() {
            // Removing something that is not there is not a change — it is a command that
            // fails every time it runs. `absent:jq` on a machine that has never had jq made
            // every sync fail, permanently, with an error from the package manager about a
            // package it does not have.
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

                // **Every reason not to remove this is a value, not a `continue`.** Both of the
                // reasons that leave the machine holding something were bare `continue`s with a
                // `debug!` above them, and a plan that drops a package in silence reports
                // success over a machine it did not change (AU1). As a returned `Declined` each
                // one has to say whether the user hears about it, and `Declined::reported`
                // matches exhaustively — so a reason added later does not compile until someone
                // answers that question.
                if let Some(declined) =
                    self.declined(pkg, hosts, &changes.removal_tracker, &desired_keys)
                {
                    debug!("'{}' will not be removed: {:?}", key, declined);
                    if let Some(reason) = declined.reported() {
                        changes.skipped.push(Skipped { key, reason });
                    }
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
                "{:?} plan — skipping all removal planning (non-destructive).",
                scope
            );
        }

        // Installations and dependency graph
        let target_specs = self.identify_needed_actions(&declared).await?;
        self.build_execution_graph(&mut changes, &target_specs)?;

        // Stable order, for the same reason `generate_report` sorts: the crawl that produced
        // these follows a HashMap, so the same machine printed them differently each run.
        changes.skipped.sort_by(|a, b| a.key.cmp(&b.key));

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

    /// One install node per declared spec, plus the `@requires` edges written between them.
    ///
    /// **The only edges are the ones somebody wrote.** A node's neighbours in this graph are
    /// what splits a manager's wave into two command lines (II.19), and a wave that splits for
    /// a reason nobody declared is a wave that splits for nothing: every manager here resolves
    /// and installs its own dependency closure, so `apt install nginx libfoo` needs no help
    /// ordering the two — and `apt install nginx` then `apt install libfoo` is measurably the
    /// slower way to ask for the same machine (V.115).
    fn build_execution_graph(
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
        }
        Ok(())
    }

    /// Every declared spec, keyed `backend:name`, with duplicates collapsed.
    ///
    /// **What a package depends on is the manager's answer, not LiNix's question.** This used
    /// to ask each backend for a package's dependencies and add each one as an install node of
    /// its own. Three separate things were wrong with that, and only the first was ever
    /// reported:
    ///
    /// - Every install node is written into `registry.json` as a package LiNix manages, so one
    ///   `apt:nginx` line took ownership of nginx's direct dependencies — and a managed package
    ///   nothing declares is drift, which `sync` removes. The dependencies were shielded only
    ///   by being re-derived identically on the next run; a single failed `apt-cache depends`
    ///   dropped every one of them out of the desired set at once. `Queryable::tracks_manual`
    ///   refuses a backend that cannot tell a dependency from a choice, for exactly this
    ///   outcome, and the planner was manufacturing the same rows behind it.
    /// - The node it added wired an edge, and an edge splits the manager's wave into two
    ///   command lines — so the one case where LiNix knew two declared packages were related
    ///   was the one case it refused to put them on one `apt install`.
    /// - It cost a subprocess per declared package and another per discovered dependency,
    ///   before any install began.
    ///
    /// The data path had already reached this conclusion one row at a time — every
    /// `ManagerConfig` in `registry.rs` sets `depends_args: None`, including the shared
    /// `base_config` the rest are built from; apt's carried a test and zypper's a comment
    /// saying re-deriving a closure "adds nodes the planner then tries to install by name".
    /// Seven hand-written backends never got the same treatment. The rule belongs here, at
    /// the one caller, rather than in each of 23 answers.
    fn declared_specs(desired: &HashMap<String, Vec<PackageSpec>>) -> HashMap<String, PackageSpec> {
        let mut out: HashMap<String, PackageSpec> = HashMap::new();
        for spec in desired.values().flatten() {
            // First writing wins, as it did when this was an expansion: two lines naming one
            // package is the resolver's to collapse, and picking the later one here would make
            // which set of `@` options survives depend on a HashMap crawl.
            out.entry(format!("{}:{}", spec.backend, spec.name))
                .or_insert_with(|| spec.clone());
        }
        out
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
        let (s_hash, t_hash) = crate::core::security::checksum_pair(source, target).await;
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
            source: "test".into(),
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
            planner
                .plan(&desired, PlanScope::Whole(HostBackends::default()))
                .await
                .unwrap()
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
                .plan(&desired, PlanScope::Narrowed(Scope::Module("dev".into())))
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
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
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
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
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
        /// Its own, so one fake's answer never reaches another's assertions.
        listings: crate::core::installed::InstalledListings,
        /// How many times the manager itself was actually run.
        fetches: Arc<std::sync::atomic::AtomicUsize>,
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
        fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
            (&self.listings, &self.name)
        }

        async fn fetch_installed(&self) -> Result<Vec<crate::core::Package>> {
            self.fetches
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

    /// Register one fake manager, handing back the counter of how often it was really run.
    fn register_fake(
        registry: &mut BackendRegistry,
        backend: &str,
        installed: &[&str],
    ) -> Arc<std::sync::atomic::AtomicUsize> {
        let fetches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fake = Arc::new(FakeInstalled {
            name: backend.to_string(),
            installed: installed.iter().map(|s| s.to_string()).collect(),
            listings: crate::core::installed::InstalledListings::new(),
            fetches: fetches.clone(),
        });
        registry.register(Arc::new(
            crate::core::manager::BackendCapabilities::builder(fake.clone())
                .with_queryable(fake)
                .build(),
        ));
        fetches
    }

    fn registry_reporting(backend: &str, installed: &[&str]) -> Arc<BackendRegistry> {
        let mut registry = BackendRegistry::new();
        register_fake(&mut registry, backend, installed);
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
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
            .await
            .unwrap()
            .total_remove()
    }

    /// Each manager the plan consults is run **once**, and no manager it does not consult is
    /// run at all.
    ///
    /// The first half is what makes the plan's fan-out worth having: the fan-out is over specs,
    /// so twenty declarations of one backend are twenty futures waiting on one listing, and if
    /// that listing were fetched per spec the wave would be twenty subprocesses deep. The
    /// second half is the guard on the fix for it — asking every manager up front is right only
    /// while "every manager" means the ones this plan was going to ask anyway. Widening it to
    /// the registry would wake a dozen managers for a one-line manifest, which is the cost this
    /// was supposed to remove, spent on a different run.
    #[tokio::test]
    async fn a_plan_runs_each_manager_it_consults_once_and_the_others_never() {
        let mut registry = BackendRegistry::new();
        let declared = register_fake(&mut registry, "declared-mgr", &["jq", "curl"]);
        let bystander = register_fake(&mut registry, "bystander-mgr", &["vim"]);
        let registry = Arc::new(registry);

        let specs: Vec<PackageSpec> = ["jq", "curl", "ripgrep", "fd"]
            .iter()
            .map(|n| PackageSpec {
                name: (*n).to_string(),
                backend: "declared-mgr".to_string(),
                ..Default::default()
            })
            .collect();
        let desired: HashMap<String, Vec<PackageSpec>> =
            [("declared-mgr".to_string(), specs)].into_iter().collect();

        let config = Config::default();
        let state = StateRegistry::new(PathBuf::from("test-state.json"));
        let changes = ChangePlanner::new(registry, &state, &config)
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
            .await
            .unwrap();

        use std::sync::atomic::Ordering;
        assert_eq!(
            declared.load(Ordering::SeqCst),
            1,
            "four declarations asked its manager more than once"
        );
        assert_eq!(
            bystander.load(Ordering::SeqCst),
            0,
            "a manager nothing declares was woken by a plan that had no question for it"
        );
        // And the plan itself is unchanged by any of that: two installed, two to install.
        assert_eq!(changes.total_install(), 2);
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
            .plan(&desired, PlanScope::Narrowed(Scope::Module("dev".into())))
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
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
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
        imp.source = "imperative".into();
        state.packages.push(imp);
        let desired: HashMap<String, Vec<PackageSpec>> = HashMap::new();

        let changes = ChangePlanner::new(registry.clone(), &state, &config)
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
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
        // `purge-undeclared` instead: a command you type, not a mode you inherit.
        let registry = Arc::new(BackendRegistry::new());
        let config = Config::default();
        // Nothing managed, nothing desired: an untouched machine full of software.
        let state = StateRegistry::new(PathBuf::from("test-state.json"));
        let desired: HashMap<String, Vec<PackageSpec>> = HashMap::new();

        let changes = ChangePlanner::new(registry.clone(), &state, &config)
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
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

    /// A manager that answers a dependency query, and counts how many times it was asked.
    ///
    /// Nothing on this machine has the packages it names, so anything the planner decides to
    /// install shows up as a node — which is what makes "one declaration, one node" testable.
    struct DepAnswering {
        deps: HashMap<String, Vec<String>>,
        asked: Arc<std::sync::atomic::AtomicUsize>,
        listings: crate::core::installed::InstalledListings,
    }

    impl DepAnswering {
        fn new(deps: &[(&str, &[&str])]) -> Arc<Self> {
            Arc::new(Self {
                deps: deps
                    .iter()
                    .map(|(k, v)| {
                        (
                            (*k).to_string(),
                            v.iter().map(|s| (*s).to_string()).collect(),
                        )
                    })
                    .collect(),
                asked: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                listings: crate::core::installed::InstalledListings::new(),
            })
        }
    }

    #[async_trait::async_trait]
    impl crate::core::manager::BackendCore for DepAnswering {
        fn name(&self) -> &str {
            "deptest"
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
    impl crate::core::manager::Queryable for DepAnswering {
        fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
            (&self.listings, "deptest")
        }
        async fn fetch_installed(&self) -> Result<Vec<crate::core::Package>> {
            Ok(Vec::new())
        }
        async fn list_manual(&self) -> Result<Vec<crate::core::Package>> {
            Ok(Vec::new())
        }
        async fn info(&self, _name: &str) -> Result<Option<crate::core::Package>> {
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl crate::core::manager::MetadataProvider for DepAnswering {
        async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
            self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.deps.get(name).cloned().unwrap_or_default())
        }
    }

    fn dep_registry(backend: Arc<DepAnswering>) -> Arc<BackendRegistry> {
        let mut registry = BackendRegistry::new();
        registry.register(Arc::new(
            crate::core::manager::BackendCapabilities::builder(backend.clone())
                .with_queryable(backend.clone())
                .with_metadata_provider(backend)
                .build(),
        ));
        Arc::new(registry)
    }

    fn declared(name: &str) -> PackageSpec {
        let mut options = HashMap::new();
        options.insert("__source".to_string(), "modules/dev.txt:1".to_string());
        PackageSpec {
            name: name.into(),
            backend: "deptest".into(),
            options,
            requires: vec![],
            present: true,
        }
    }

    /// **A plan installs what you declared. Not what your declarations depend on.**
    ///
    /// Every install node is written into the state registry as a package LiNix manages, and
    /// anything in that registry is a removal candidate the moment nothing declares it. A
    /// dependency is never declared, so expanding one manufactures a managed package with no
    /// line behind it — `Queryable::tracks_manual` says exactly this about `adopt`, which
    /// refuses a backend that cannot tell a dependency from a choice, and the planner was
    /// doing by construction what `adopt` refuses to do.
    #[tokio::test]
    async fn a_declaration_is_the_only_thing_that_becomes_an_install() {
        let backend = DepAnswering::new(&[("nginx", &["libfoo", "libbar"])]);
        let asked = backend.asked.clone();
        let registry = dep_registry(backend);
        let config = Config::default();
        let state = StateRegistry::new(PathBuf::from("test-state.json"));
        let desired: HashMap<String, Vec<PackageSpec>> =
            [("deptest".to_string(), vec![declared("nginx")])]
                .into_iter()
                .collect();

        let changes = ChangePlanner::new(registry, &state, &config)
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
            .await
            .unwrap();

        let report = changes.generate_report();
        let names: Vec<&str> = report.install.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["nginx"],
            "libfoo and libbar are the manager's business, not LiNix's"
        );
        assert_eq!(
            asked.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "planning must not spend a subprocess asking what a package depends on"
        );
    }

    /// The instrument, self-tested: this fake really does answer, so the zero above is the
    /// planner not asking rather than the fake not knowing.
    #[tokio::test]
    async fn the_fake_manager_really_does_answer_a_dependency_query() {
        use crate::core::manager::MetadataProvider;
        let backend = DepAnswering::new(&[("nginx", &["libfoo", "libbar"])]);
        assert_eq!(
            backend.get_dependencies("nginx").await.unwrap(),
            vec!["libfoo".to_string(), "libbar".to_string()]
        );
        assert_eq!(backend.asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// **Two declared packages of one manager have no edge between them, so they are one
    /// command** (II.19). They did, whenever one happened to depend on the other — which for a
    /// system manager is most of a real config, and it is the case `rebuild --backend apt`
    /// maximises. Measured on Ubuntu: eight packages one at a time took 31,901 ms against
    /// 3,161 ms as one command.
    #[tokio::test]
    async fn a_native_dependency_between_declared_packages_does_not_split_the_command() {
        let backend = DepAnswering::new(&[("nginx", &["libfoo"])]);
        let registry = dep_registry(backend);
        let config = Config::default();
        let state = StateRegistry::new(PathBuf::from("test-state.json"));
        let desired: HashMap<String, Vec<PackageSpec>> = [(
            "deptest".to_string(),
            vec![declared("nginx"), declared("libfoo")],
        )]
        .into_iter()
        .collect();

        let changes = ChangePlanner::new(registry, &state, &config)
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
            .await
            .unwrap();

        assert_eq!(changes.graph.node_count(), 2);
        assert_eq!(
            changes.graph.edge_count(),
            0,
            "nobody wrote `@requires`, so nothing may split the batch"
        );
    }

    /// …and a `@requires` the user *did* write still orders the two sides (`Y1`).
    #[tokio::test]
    async fn a_written_requires_is_still_an_edge() {
        let backend = DepAnswering::new(&[]);
        let registry = dep_registry(backend);
        let config = Config::default();
        let state = StateRegistry::new(PathBuf::from("test-state.json"));
        let mut dependent = declared("nginx");
        dependent.requires = vec!["deptest:libfoo".to_string()];
        let desired: HashMap<String, Vec<PackageSpec>> =
            [("deptest".to_string(), vec![dependent, declared("libfoo")])]
                .into_iter()
                .collect();

        let changes = ChangePlanner::new(registry, &state, &config)
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
            .await
            .unwrap();

        assert_eq!(changes.graph.edge_count(), 1);
    }
}
