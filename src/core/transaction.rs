use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::LuaHooks;
use crate::backends::BackendRegistry;
use crate::core::journal::JournalAction;
use crate::core::{Error, Journal, PackageSpec, Result, Retryability};
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableDiGraph;
use petgraph::Direction;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

#[derive(Debug, Clone)]
pub struct TransactionConfig {
    pub max_concurrent: usize,
    pub node_timeout: Duration,
    pub total_timeout: Duration,
    pub max_retries: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub auto_rollback: bool,
    /// Finish every node that still can, instead of stopping at the first failure.
    ///
    /// Off for `sync`, and it must stay off: a plan is one change to one machine, so a member
    /// that fails makes the whole plan wrong and the rest of it must not be half-applied.
    /// Recovery is the opposite shape — each entry is a separate piece of interrupted work
    /// left by a run that already died, and one that cannot be finished is not a reason to
    /// leave the others unfinished. A node whose *dependency* failed is still never attempted;
    /// it is reported as skipped, naming the one that stopped it.
    pub continue_on_error: bool,
    /// Remove also destroys configuration (`[remove] purge`, or `uninstall --purge`). A
    /// backend that draws no such distinction removes as usual — the decision cannot be
    /// per-package because a removal happens after the line that carried it is gone.
    pub purge: bool,
}

impl Default for TransactionConfig {
    fn default() -> Self {
        Self::patient()
    }
}

impl TransactionConfig {
    /// The defaults. `sync` overrides `max_concurrent` from `max_parallel`; this is what every
    /// other constructor gets, so it is the machine's parallelism rather than the number 4.
    pub fn patient() -> Self {
        Self {
            max_concurrent: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            node_timeout: Duration::from_secs(300),
            total_timeout: Duration::from_secs(3600),
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            auto_rollback: true,
            continue_on_error: false,
            purge: false,
        }
    }

    /// The settings a run's `Config` decides, in one place.
    ///
    /// These were three ad-hoc reads at the one call site, and the comment above the first of
    /// them recorded what that costs: `max_concurrent` had been left at the `patient()`
    /// default, which "silently narrows the setting's reach to `search` alone". A named
    /// constructor is where the fourth one goes instead of becoming a fourth ad-hoc line.
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            max_concurrent: config.max_parallel.max(1),
            purge: config.remove.purge || config.purge_this_run,
            continue_on_error: config.keep_going_this_run,
            ..Self::patient()
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum GraphAction {
    Install(PackageSpec),
    Remove { name: String, backend: String },
}

/// What a node's target looked like *before* that node ran.
///
/// Rollback compensates by putting this back, so it has to be a fact rather than an
/// assumption. Compensating an `Install` with a removal is right only when the package was
/// absent to begin with — and it often is not: a `@version=` or `@channel=` change schedules an
/// `Install` node for a package that is already there, so removing it uninstalls software the
/// user had instead of reverting a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prior {
    /// The package was not installed before this node ran.
    Absent,
    /// It was installed, at this version when the manager reported one.
    Present(Option<String>),
    /// The manager could not be asked, or has no query capability. Nothing is inferred from
    /// this — "I could not tell" is not "it was not there".
    Unknown,
}

/// The nodes one manager command covers, paired with what each of them asks for.
type Batch = Vec<(NodeIndex, GraphAction)>;

#[derive(Debug, Clone)]
pub struct TaskResult {
    pub node_index: NodeIndex,
    pub backend_name: String,
    pub package_name: String,
    /// How many times this node was *retried* — 0 on a first-try success. Named for what it
    /// holds: it fed a parameter called `retry_count` while being called `attempt`, so the
    /// arithmetic below reads like an off-by-one to everyone who checks it.
    pub retries: u32,
    pub duration: Duration,
    pub bytes_downloaded: u64,
    pub start_time: chrono::DateTime<chrono::Utc>,
    /// What this node's target looked like before it ran. Only read by `rollback`.
    pub prior: Prior,
    /// How many packages the single manager command that covered this one carried.
    ///
    /// `1` unless this node was batched. Reported, because six packages sharing one `apt
    /// install` produce six identical durations — and six identical durations under a heading
    /// that says "Parallel Task Breakdown" is exactly how a serialised run read as a parallel
    /// one for as long as it did. Now they are identical for a reason the output states.
    pub batch_size: usize,
    pub result: Result<()>,
}

pub struct Transaction {
    pub graph: StableDiGraph<GraphAction, ()>,
    registry: Arc<BackendRegistry>,
    journal: Arc<Mutex<Journal>>,
    diagnostics: Arc<FailureDiagnosticEngine>,
    config: TransactionConfig,
    /// The user's configuration, for the removal guard. A rollback's compensating removals are
    /// issued here, at execution time, and never pass the plan-time gate in `sync` — so this is
    /// the only place they can be checked, and a guard on one path is a guard on nothing.
    app_config: Arc<crate::config::Config>,
    /// Optional lifecycle hooks. When set, `before_install`/`after_install` fire
    /// per package at the moment it is installed (interleaved with parallel execution).
    hooks: Option<Arc<LuaHooks>>,
    completed_indices: HashSet<NodeIndex>,
    /// Each finished node with what its target looked like before it ran. Rollback walks this
    /// backwards, and cannot compensate correctly without the second half.
    history: Vec<(NodeIndex, Prior)>,
    cancellation_token: CancellationToken,
}

impl Transaction {
    pub fn new(
        graph: StableDiGraph<GraphAction, ()>,
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        diagnostics: Arc<FailureDiagnosticEngine>,
        app_config: Arc<crate::config::Config>,
    ) -> Self {
        Self::with_config(
            graph,
            registry,
            journal,
            diagnostics,
            app_config,
            TransactionConfig::default(),
        )
    }

    pub fn with_config(
        graph: StableDiGraph<GraphAction, ()>,
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        diagnostics: Arc<FailureDiagnosticEngine>,
        app_config: Arc<crate::config::Config>,
        config: TransactionConfig,
    ) -> Self {
        Self {
            graph,
            registry,
            journal,
            diagnostics,
            config,
            app_config,
            hooks: None,
            completed_indices: HashSet::new(),
            history: Vec::new(),
            cancellation_token: CancellationToken::new(),
        }
    }

    /// Packages with no configured hook (and no `*` wildcard) incur only a cheap map
    /// lookup, so this is safe to always set.
    pub fn with_hooks(mut self, hooks: Arc<LuaHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    pub async fn execute_with_telemetry(&mut self) -> Result<Vec<TaskResult>> {
        let total_timeout = self.config.total_timeout;
        let start_time = Instant::now();

        info!(
            "Initializing parallel execution for {} nodes.",
            self.graph.node_count()
        );

        match tokio::time::timeout(total_timeout, self.execute_internal()).await {
            Ok(res) => {
                debug!("DAG closure reached in {:?}", start_time.elapsed());
                res
            }
            Err(_) => {
                error!(
                    "CRITICAL FAILURE - Global timeout of {:?} reached.",
                    total_timeout
                );
                self.cancellation_token.cancel();
                if self.config.auto_rollback {
                    if let Err(e) = self.rollback().await {
                        error!("{}", e);
                    }
                }
                Err(Error::Transaction(format!(
                    "Transaction timed out after {:?}",
                    total_timeout
                )))
            }
        }
    }

    pub async fn execute(&mut self) -> Result<()> {
        self.execute_with_telemetry().await.map(|_| ())
    }

    async fn execute_internal(&mut self) -> Result<Vec<TaskResult>> {
        let total_nodes = self.graph.node_count();
        let mut in_progress = HashSet::new();
        let mut worker_pool = JoinSet::new();
        let mut telemetry_results = Vec::new();

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent));

        // How many unfinished dependencies each node still has. Decremented as they finish,
        // instead of rescanning every node and every incoming edge on every pass — which was
        // O(V·(V+E)) over a run, or ~100k redundant edge checks for 300 packages, and it only
        // gets worse as the batching below makes the graph wide enough to notice.
        let mut pending_deps: HashMap<NodeIndex, usize> = self
            .graph
            .node_indices()
            .map(|idx| {
                let n = self
                    .graph
                    .neighbors_directed(idx, Direction::Incoming)
                    .filter(|dep| !self.completed_indices.contains(dep))
                    .count();
                (idx, n)
            })
            .collect();
        let mut ready: Vec<NodeIndex> = pending_deps
            .iter()
            .filter(|(idx, n)| **n == 0 && !self.completed_indices.contains(idx))
            .map(|(idx, _)| *idx)
            .collect();
        // Node order, not hash order, so a plan runs the same way twice.
        ready.sort();

        while self.completed_indices.len() < total_nodes {
            if self.cancellation_token.is_cancelled() {
                worker_pool.abort_all();
                if self.config.auto_rollback {
                    if let Err(e) = self.rollback().await {
                        error!("{}", e);
                    }
                }
                return Err(Error::Transaction("Transaction cancelled.".into()));
            }

            for batch in Self::batches(&self.graph, std::mem::take(&mut ready)) {
                let permit = match semaphore.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(e) => return Err(Error::Transaction(format!("Semaphore failure: {}", e))),
                };

                for (idx, _) in &batch {
                    in_progress.insert(*idx);
                }

                let registry = self.registry.clone();
                let journal = self.journal.clone();
                let cancel_token = self.cancellation_token.clone();
                let config = self.config.clone();
                let hooks = self.hooks.clone();

                worker_pool.spawn(async move {
                    let _permit_holder = permit;
                    Self::execute_batch_with_retry(
                        batch,
                        registry,
                        journal,
                        config,
                        hooks,
                        cancel_token,
                    )
                    .await
                });
            }

            if let Some(finished_task) = worker_pool.join_next().await {
                let results = finished_task
                    .map_err(|e| Error::Transaction(format!("Worker Panic: {}", e)))?;

                // Every result is recorded before any failure is acted on. A batch that fails
                // fails every package in it, and reporting only the first would make the
                // summary say one package did not install when six did not.
                let mut first_failure: Option<Error> = None;
                let mut failed_now: Vec<(NodeIndex, String)> = Vec::new();
                for task_data in results {
                    if task_data.result.is_ok() {
                        trace!(
                            "Node {}:{} succeeded.",
                            task_data.backend_name,
                            task_data.package_name
                        );
                        in_progress.remove(&task_data.node_index);
                        self.completed_indices.insert(task_data.node_index);
                        self.history
                            .push((task_data.node_index, task_data.prior.clone()));
                        // Whatever was waiting only on this one is ready now.
                        for dependent in self
                            .graph
                            .neighbors_directed(task_data.node_index, Direction::Outgoing)
                        {
                            if let Some(n) = pending_deps.get_mut(&dependent) {
                                *n = n.saturating_sub(1);
                                if *n == 0 && !self.completed_indices.contains(&dependent) {
                                    ready.push(dependent);
                                }
                            }
                        }
                        telemetry_results.push(task_data);
                        continue;
                    }

                    let error_msg = task_data
                        .result
                        .as_ref()
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "Execution Error".into());

                    // `debug!`, not `error!`. This failure is returned below and printed once,
                    // as itself, by `main`. Printing it here as well said the same thing twice
                    // and called the package a "Node" — the DAG's word for it, which no user
                    // asked about. The suggestions below are the part worth keeping.
                    debug!(
                        "Node {}:{} FAILED: {}",
                        task_data.backend_name, task_data.package_name, error_msg
                    );

                    // Once per failure, not once per package in a batch: six packages sharing
                    // one command share one reason, and printing the same paragraph six times
                    // is the noise the diagnostics exist to cut through.
                    if first_failure.is_none() {
                        self.diagnostics
                            .print_suggestions(&error_msg, &task_data.backend_name);
                        // Named here because this is the only place that still knows which
                        // node it was. `install X` converges the whole configuration, so the
                        // line that failed is often not the one anybody typed, and the error
                        // used to arrive as the manager's own words about a command the user
                        // never asked for (`Q34`).
                        let origin = match &self.graph[task_data.node_index] {
                            GraphAction::Install(s) => s.options.get("__source").cloned(),
                            GraphAction::Remove { .. } => None,
                        };
                        first_failure =
                            Some(task_data.result.clone().err().unwrap().about_declaration(
                                &format!("{}:{}", task_data.backend_name, task_data.package_name),
                                origin.as_deref(),
                            ));
                    }
                    failed_now.push((
                        task_data.node_index,
                        format!("{}:{}", task_data.backend_name, task_data.package_name),
                    ));
                    telemetry_results.push(task_data);
                }

                if self.config.continue_on_error {
                    // A failed node is terminal: it will not be retried and nothing waiting on
                    // it can run, so both it and everything downstream come off the board here
                    // — otherwise the loop below never reaches `total_nodes` and reports a
                    // cycle that does not exist.
                    for (idx, named) in failed_now {
                        in_progress.remove(&idx);
                        self.completed_indices.insert(idx);
                        for skipped in self.unreachable_from(idx) {
                            if self.completed_indices.insert(skipped) {
                                telemetry_results.push(Self::skipped_result(
                                    &self.graph[skipped],
                                    skipped,
                                    &named,
                                ));
                            }
                        }
                    }
                    ready.sort();
                    continue;
                }

                if let Some(final_err) = first_failure {
                    if self.config.auto_rollback {
                        info!("rolling back");
                        worker_pool.abort_all();
                        if let Err(e) = self.rollback().await {
                            error!("{}", e);
                        }
                    }
                    return Err(final_err);
                }
                ready.sort();
            } else if in_progress.is_empty() && self.completed_indices.len() < total_nodes {
                return Err(Error::Transaction(
                    "DAG Logic Stall: Cycle detected in closure.".into(),
                ));
            }
        }
        Ok(telemetry_results)
    }

    /// Every node that can only be reached through `failed` — the work a failure has just made
    /// impossible. Excludes `failed` itself, which the caller has already accounted for.
    fn unreachable_from(&self, failed: NodeIndex) -> Vec<NodeIndex> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeIndex> = self
            .graph
            .neighbors_directed(failed, Direction::Outgoing)
            .collect();
        let mut seen: HashSet<NodeIndex> = HashSet::new();
        while let Some(idx) = stack.pop() {
            if !seen.insert(idx) {
                continue;
            }
            out.push(idx);
            stack.extend(self.graph.neighbors_directed(idx, Direction::Outgoing));
        }
        out.sort();
        out
    }

    /// A node nobody attempted, reported as itself. Not a success and not a failure of its own:
    /// the reason names the node that stopped it, because "jq failed" about a package LiNix
    /// never ran a command for is the attribution problem this engine is meant to be free of.
    fn skipped_result(action: &GraphAction, idx: NodeIndex, blocked_by: &str) -> TaskResult {
        let (backend_name, package_name) = match action {
            GraphAction::Install(s) => (s.backend.clone(), s.name.clone()),
            GraphAction::Remove { backend, name } => (backend.clone(), name.clone()),
        };
        TaskResult {
            node_index: idx,
            result: Err(Error::Transaction(format!(
                "not attempted: it needs `{}`, which could not be completed",
                blocked_by
            ))),
            backend_name,
            package_name,
            retries: 0,
            duration: Duration::ZERO,
            bytes_downloaded: 0,
            start_time: chrono::Utc::now(),
            prior: Prior::Unknown,
            batch_size: 0,
        }
    }

    /// The most packages LiNix will put on one manager command line.
    ///
    /// A bound on argv, not on ambition: `cmd.exe` caps a command line at 8191 characters and
    /// every manager has some limit. A hundred names is far below any of them and far above
    /// the point where batching has taken the win — the cost this removes is per *invocation*,
    /// so the second hundred saves almost nothing the first did not.
    const MAX_BATCH: usize = 100;
    /// …and a byte bound, because package names are not all short. `github:owner/repo@…`
    /// spends far more per name than `jq` does.
    const MAX_BATCH_BYTES: usize = 6000;

    /// Split a ready set into the commands it becomes.
    ///
    /// Everything in one batch shares a manager and a kind of change, and no two of them have
    /// an edge between them — they are ready at the same moment, which is what "ready" means.
    /// Batches come out in node order so a plan runs the same way twice.
    ///
    /// **Every edge in this graph is an `@requires` somebody wrote** (`Y9`). The planner used
    /// to add one per native dependency it discovered, which split this batch for a
    /// relationship the manager was going to resolve by itself anyway.
    fn batches(graph: &StableDiGraph<GraphAction, ()>, mut ready: Vec<NodeIndex>) -> Vec<Batch> {
        ready.sort();
        /// One manager, one kind of change, and the nodes gathered for it so far.
        struct Group {
            backend: String,
            is_install: bool,
            members: Batch,
        }
        let mut groups: Vec<Group> = Vec::new();
        for idx in ready {
            let action = graph[idx].clone();
            let (backend, is_install) = match &action {
                GraphAction::Install(s) => (s.backend.clone(), true),
                GraphAction::Remove { backend, .. } => (backend.clone(), false),
            };
            match groups
                .iter_mut()
                .find(|g| g.backend == backend && g.is_install == is_install)
            {
                Some(g) => g.members.push((idx, action)),
                None => groups.push(Group {
                    backend,
                    is_install,
                    members: vec![(idx, action)],
                }),
            }
        }

        let mut out = Vec::new();
        for Group { members, .. } in groups {
            let mut current: Batch = Vec::new();
            let mut bytes = 0usize;
            for (idx, action) in members {
                let cost = match &action {
                    GraphAction::Install(s) => s.name.len() + 1,
                    GraphAction::Remove { name, .. } => name.len() + 1,
                };
                if !current.is_empty()
                    && (current.len() >= Self::MAX_BATCH || bytes + cost > Self::MAX_BATCH_BYTES)
                {
                    out.push(std::mem::take(&mut current));
                    bytes = 0;
                }
                bytes += cost;
                current.push((idx, action));
            }
            if !current.is_empty() {
                out.push(current);
            }
        }
        out
    }

    /// Run one manager command covering every node in `batch`, with retry.
    ///
    /// **A batch is one command, not one package.** Every node here is ready at the same
    /// moment, goes to the same manager, and is the same kind of change, with no `@requires`
    /// edge between any two of them — which is precisely the set that manager's own command
    /// line was built to take. Measured on Ubuntu, six declared packages produced six separate `apt install`
    /// processes and 12,465 ms; `apt install <8 packages>` as one command took 3,161 ms. Eight
    /// packages one at a time took 31,901 ms — superlinear, because each invocation re-reads
    /// the package cache, re-takes the dpkg lock and re-resolves a dependency graph the batch
    /// resolves once.
    ///
    /// Every backend in this tree already accepts multiple names on one command line, and
    /// `generic::install_group` was already written to batch — it partitions `@unverified`
    /// specs into their own command and accumulates names across specs. It had never been
    /// handed more than one.
    ///
    /// The returned vector has one `TaskResult` per node, so rollback, the journal and the
    /// telemetry all still work per package: `Prior` is captured per package before the command
    /// runs, and a batch that fails fails every package in it — which is the same outcome a
    /// single failure had before, since any node failure rolls the whole transaction back.
    #[allow(clippy::too_many_arguments)]
    async fn execute_batch_with_retry(
        batch: Batch,
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        config: TransactionConfig,
        hooks: Option<Arc<LuaHooks>>,
        cancel_token: CancellationToken,
    ) -> Vec<TaskResult> {
        let start_time_utc = chrono::Utc::now();
        let start_instant = Instant::now();
        let is_install = matches!(batch.first().map(|(_, a)| a), Some(GraphAction::Install(_)));
        let b_name = match batch.first().map(|(_, a)| a) {
            Some(GraphAction::Install(s)) => s.backend.clone(),
            Some(GraphAction::Remove { backend, .. }) => backend.clone(),
            None => return Vec::new(),
        };

        // One `TaskResult` for a node that never reached the manager.
        let stillborn = |idx: NodeIndex, name: String, prior: Prior, e: Error| TaskResult {
            node_index: idx,
            backend_name: b_name.clone(),
            package_name: name,
            retries: 0,
            duration: Duration::ZERO,
            bytes_downloaded: 0,
            start_time: start_time_utc,
            prior,
            batch_size: 1,
            result: Err(e),
        };

        let mut refused: Vec<TaskResult> = Vec::new();
        let mut members: Vec<(NodeIndex, GraphAction, String)> = Vec::new();

        // The grammar checks what a *file* declares. A removal target comes from
        // `registry.json`, which apt's post-invoke hook also writes, so it has not been
        // through the grammar at all. A name that cannot be validated is refused on its own
        // and never reaches the shared command line.
        for (idx, action) in batch {
            let p_name = match &action {
                GraphAction::Install(s) => s.name.clone(),
                GraphAction::Remove { name, .. } => name.clone(),
            };
            match crate::core::Validator::validate_package_name_for(&p_name, &b_name) {
                Ok(()) => members.push((idx, action, p_name)),
                Err(e) => refused.push(stillborn(idx, p_name, Prior::Unknown, e)),
            }
        }
        if members.is_empty() {
            return refused;
        }

        let backend_cap = match registry.get(&b_name) {
            Some(cap) => cap,
            None => {
                for m in members {
                    refused.push(stillborn(
                        m.0,
                        m.2,
                        Prior::Unknown,
                        Error::BackendNotFound(b_name.clone()),
                    ));
                }
                return refused;
            }
        };

        // Read before anything is done to it, per package. Rollback compensates by putting
        // this back, and "what was there before" is unknowable once the command has run.
        // Skipped entirely when there is no rollback to feed. Concurrent, and cheap now that
        // one listing per manager serves every question in the run.
        let priors: Vec<Prior> = if config.auto_rollback {
            use futures::stream::StreamExt;
            futures::stream::iter(members.iter().map(|m| m.2.clone()).collect::<Vec<_>>())
                .map(|name| {
                    let backend_cap = backend_cap.clone();
                    async move { Self::prior_state(&backend_cap, &name).await }
                })
                .buffered(members.len().max(1))
                .collect()
                .await
        } else {
            vec![Prior::Unknown; members.len()]
        };

        // The WAL, per package and before the manager is invoked. Recovery depends on the
        // entry reaching disk first, and a batch does not change that — it changes how many
        // bytes each entry costs (see `core::journal`).
        let mut ids: Vec<String> = Vec::with_capacity(members.len());
        {
            let mut j = journal.lock().await;
            for (_, action, _) in &members {
                let j_action = match action {
                    GraphAction::Install(s) => JournalAction::Install(s.clone()),
                    GraphAction::Remove { name, backend } => JournalAction::Remove {
                        name: name.clone(),
                        backend: backend.clone(),
                    },
                };
                match j.record_start(j_action) {
                    Ok(id) => ids.push(id),
                    Err(e) => {
                        drop(j);
                        for i in 0..members.len() {
                            refused.push(stillborn(
                                members[i].0,
                                members[i].2.clone(),
                                priors[i].clone(),
                                Error::Journal(format!("WAL error: {}", e)),
                            ));
                        }
                        return refused;
                    }
                }
            }
        }

        // `before_install` fires per package, before any install attempt. A failing pre-hook
        // takes that package out of the batch — its declared prerequisites were not met — and
        // leaves the rest of the command alone.
        let mut keep: Vec<usize> = Vec::with_capacity(members.len());
        if is_install {
            if let Some(h) = &hooks {
                for (i, (idx, _, name)) in members.iter().enumerate() {
                    match h.run_hook("before_install", name).await {
                        Ok(_) => keep.push(i),
                        Err(e) => {
                            let msg = format!("before_install hook failed: {}", e);
                            let mut j = journal.lock().await;
                            let _ = j.record_failure(&ids[i], &msg);
                            drop(j);
                            refused.push(stillborn(
                                *idx,
                                name.clone(),
                                priors[i].clone(),
                                Error::Transaction(msg),
                            ));
                        }
                    }
                }
            } else {
                keep.extend(0..members.len());
            }
        } else {
            keep.extend(0..members.len());
        }
        if keep.is_empty() {
            return refused;
        }

        let batch_size = keep.len();
        let specs: Vec<PackageSpec> = keep
            .iter()
            .filter_map(|&i| match &members[i].1 {
                GraphAction::Install(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        let names: Vec<String> = keep.iter().map(|&i| members[i].2.clone()).collect();

        let mut attempt = 0;
        let mut last_error = None;

        while attempt <= config.max_retries {
            attempt += 1;
            if cancel_token.is_cancelled() {
                refused.extend(keep.iter().map(|&i| {
                    let (idx, _, name) = &members[i];
                    TaskResult {
                        node_index: *idx,
                        backend_name: b_name.clone(),
                        package_name: name.clone(),
                        retries: attempt - 1,
                        duration: start_instant.elapsed(),
                        bytes_downloaded: 0,
                        start_time: start_time_utc,
                        prior: priors[i].clone(),
                        batch_size,
                        result: Err(Error::Cancelled),
                    }
                }));
                return refused;
            }

            if attempt > 1 {
                let backoff = std::cmp::min(
                    config.initial_backoff * (1 << (attempt - 2)),
                    config.max_backoff,
                );
                tokio::time::sleep(backoff).await;
            }

            // One node's timeout, scaled by how many packages the command carries: eight
            // packages in one `apt install` legitimately take longer than one, and a bound
            // sized for one would turn the batching win into a timeout.
            let deadline = config
                .node_timeout
                .saturating_mul(batch_size.min(16) as u32);
            let result = tokio::time::timeout(deadline, async {
                let Some(handler) = backend_cap.as_installable() else {
                    return Err(Error::Transaction(format!(
                        "Backend '{}' is not {}.",
                        b_name,
                        if is_install {
                            "installable"
                        } else {
                            "removable"
                        }
                    )));
                };
                if is_install {
                    handler.install(&specs, backend_cap.sudo_for_write()).await
                } else {
                    let sudo = backend_cap.sudo_for_write();
                    if config.purge && handler.supports_purge() {
                        handler.purge(&names, sudo).await
                    } else {
                        handler.remove(&names, sudo).await
                    }
                }
            })
            .await;

            match result {
                Ok(Ok(())) => {
                    {
                        let mut j = journal.lock().await;
                        for &i in &keep {
                            let _ = j.record_success(&ids[i]);
                        }
                    }
                    // Fire `after_install` once each package is physically installed. A
                    // post-hook failure is logged but does not undo a successful install
                    // (rolling back a healthy package over a cosmetic hook error would be
                    // more surprising than the failure itself).
                    if is_install {
                        if let Some(h) = &hooks {
                            for &i in &keep {
                                let name = &members[i].2;
                                if let Err(e) = h.run_hook("after_install", name).await {
                                    warn!("after_install hook for '{}' failed: {}", name, e);
                                }
                            }
                        }
                    }
                    refused.extend(keep.iter().map(|&i| {
                        let (idx, _, name) = &members[i];
                        TaskResult {
                            node_index: *idx,
                            backend_name: b_name.clone(),
                            package_name: name.clone(),
                            retries: attempt - 1,
                            duration: start_instant.elapsed(),
                            bytes_downloaded: 0,
                            start_time: start_time_utc,
                            prior: priors[i].clone(),
                            batch_size,
                            result: Ok(()),
                        }
                    }));
                    return refused;
                }
                Ok(Err(e)) => {
                    // A name no repository carries is not found by waiting; three rounds of
                    // backoff only delay the report and hold the manager's lock while they
                    // do it. `Unknown` still retries — that is what every failure did before
                    // this distinction existed, and only a classified verdict overrides it.
                    let give_up = e.retryability() == Retryability::Permanent;
                    last_error = Some(e);
                    if give_up {
                        break;
                    }
                }
                Err(_) => {
                    last_error = Some(Error::Transaction(format!(
                        "`{}` did not finish {} package(s) within {:?}.",
                        b_name, batch_size, deadline
                    )));
                }
            }
        }

        let final_err = falsify_transience(
            last_error.unwrap_or(Error::Transaction("Unknown error".into())),
            attempt,
        );
        {
            let mut j = journal.lock().await;
            for &i in &keep {
                let _ = j.record_failure(&ids[i], &format!("{}", final_err));
            }
        }

        refused.extend(keep.iter().map(|&i| {
            let (idx, _, name) = &members[i];
            TaskResult {
                node_index: *idx,
                backend_name: b_name.clone(),
                package_name: name.clone(),
                retries: attempt - 1,
                duration: start_instant.elapsed(),
                bytes_downloaded: 0,
                start_time: start_time_utc,
                prior: priors[i].clone(),
                batch_size,
                result: Err(final_err.clone()),
            }
        }));
        refused
    }

    /// What the package looks like right now, before this node touches it.
    async fn prior_state(backend_cap: &Arc<crate::core::BackendCapabilities>, name: &str) -> Prior {
        let Some(q) = backend_cap.as_queryable() else {
            return Prior::Unknown;
        };
        match q.info(name).await {
            Ok(Some(pkg)) => Prior::Present(pkg.version),
            Ok(None) => Prior::Absent,
            // A query that failed is not a package that is absent. Reading it as one is how
            // a rollback ends up removing software this run never installed.
            Err(_) => Prior::Unknown,
        }
    }

    /// Put one package back the way it was, at the version it was at.
    async fn reinstate(&self, backend: &str, name: &str, version: &Option<String>) -> Result<()> {
        let Some(b) = self.registry.get(backend) else {
            return Err(Error::BackendNotFound(backend.to_string()));
        };
        let Some(h) = b.as_installable() else {
            return Err(Error::Transaction(format!(
                "backend `{}` cannot install",
                backend
            )));
        };
        let mut options = HashMap::new();
        if let Some(v) = version {
            // Without this the reinstall takes whatever is newest, so a rolled-back removal
            // silently loses its pin — the package comes back at a version nobody declared.
            options.insert("version".to_string(), v.clone());
        }
        h.install(
            &[PackageSpec {
                name: name.to_string(),
                backend: backend.to_string(),
                options,
                requires: vec![],
                present: true,
            }],
            b.sudo_for_write(),
        )
        .await
        .map(|_| ())
    }

    async fn rollback(&mut self) -> Result<()> {
        debug!("reverting modification history");
        // A compensating action that itself fails leaves the system in a partial state —
        // most dangerously, a package the user HAD, that this transaction removed, and that
        // the reinstall could not bring back. Swallowing that error (the old `let _ =`) is
        // the worst place in the codebase to be quiet (H2): the user is told the transaction
        // failed and rolled back, while a package is silently gone. Report every failure by
        // name, and return Err so the caller can say the rollback was incomplete.
        let mut failures: Vec<String> = Vec::new();
        let history = self.history.clone();

        // Recovery paths are removal paths, and they need the guard more than ordinary ones
        // because nobody is watching (V.64). These removals are issued at execution time and
        // never pass the plan-time gate in `sync`, so this is the only place they can be
        // checked.
        let backends: HashSet<String> = history
            .iter()
            .filter_map(|(idx, _)| match &self.graph[*idx] {
                GraphAction::Install(s) => Some(s.backend.clone()),
                GraphAction::Remove { .. } => None,
            })
            .collect();
        let os_essential = crate::app::sync::guard::essential_names(
            &self.registry,
            &backends,
            self.config.max_concurrent,
        )
        .await;

        for (idx, prior) in history.iter().rev() {
            match self.graph[*idx].clone() {
                GraphAction::Install(spec) => {
                    match prior {
                        // It was already there. Undoing an upgrade is putting the old version
                        // back — removing the package uninstalls software the user had, which
                        // is the opposite of a rollback.
                        Prior::Present(version) => {
                            if version.is_none() {
                                warn!(
                                    "rollback cannot revert {}:{}: its manager did not report \
                                     a version before the change, so there is none to go back \
                                     to. It stays at the version this run installed.",
                                    spec.backend, spec.name
                                );
                                failures.push(format!(
                                    "{}:{} (left at the new version)",
                                    spec.backend, spec.name
                                ));
                                continue;
                            }
                            if let Err(e) = self.reinstate(&spec.backend, &spec.name, version).await
                            {
                                error!(
                                    "rollback could not put {}:{} back to {}: {}",
                                    spec.backend,
                                    spec.name,
                                    version.as_deref().unwrap_or("its previous version"),
                                    e
                                );
                                failures.push(format!(
                                    "{}:{} (left at the new version)",
                                    spec.backend, spec.name
                                ));
                            }
                        }
                        Prior::Absent => {
                            if let Some(p) = crate::app::sync::guard::protection_of(
                                &self.app_config,
                                Some(&spec.backend),
                                &spec.name,
                                &os_essential,
                            ) {
                                error!(
                                    "rollback will not remove {}:{} — {}. It stays installed, \
                                     and this transaction is left partly applied.",
                                    spec.backend,
                                    spec.name,
                                    p.reason()
                                );
                                failures.push(format!(
                                    "{}:{} (protected, left installed)",
                                    spec.backend, spec.name
                                ));
                                continue;
                            }
                            let Some(b) = self.registry.get(&spec.backend) else {
                                continue;
                            };
                            let Some(h) = b.as_installable() else {
                                continue;
                            };
                            if let Err(e) = h
                                .remove(std::slice::from_ref(&spec.name), b.sudo_for_write())
                                .await
                            {
                                error!(
                                    "rollback could not remove {}:{} that this \
                                     run installed — it remains on the system: {}",
                                    spec.backend, spec.name, e
                                );
                                failures.push(format!(
                                    "{}:{} (left installed)",
                                    spec.backend, spec.name
                                ));
                            }
                        }
                        // Not knowing whether the user already had it is not permission to
                        // delete it. Say so instead.
                        Prior::Unknown => {
                            warn!(
                                "rollback will not remove {}:{}: LiNix could not tell whether \
                                 it was already installed before this run, and removing what \
                                 you may have had is not something you asked for.",
                                spec.backend, spec.name
                            );
                            failures.push(format!(
                                "{}:{} (left installed — prior state unknown)",
                                spec.backend, spec.name
                            ));
                        }
                    }
                }
                GraphAction::Remove { name, backend } => {
                    // Nothing was there to lose.
                    if prior == &Prior::Absent {
                        continue;
                    }
                    let version = match prior {
                        Prior::Present(v) => v.clone(),
                        _ => None,
                    };
                    if let Err(e) = self.reinstate(&backend, &name, &version).await {
                        error!(
                            "rollback could not reinstall {}:{} that this \
                             run removed — it is now MISSING: {}",
                            backend, name, e
                        );
                        failures.push(format!("{}:{} (now missing)", backend, name));
                    }
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(Error::Transaction(format!(
                "rollback was incomplete — {} compensating action(s) failed: {}",
                failures.len(),
                failures.join(", ")
            )))
        }
    }
}

/// A failure that survived its own retries is not transient, whatever the string said.
///
/// `Retryability::Transient` is a claim: *a second attempt could differ*. The container harness
/// proves that claim the only way it can be proved — it retries once and calls a repeat a
/// defect. The product asserted it from a substring and never checked, so `luarocks install
/// luafilesystem` on a machine whose `wget` is a scoop shim matched `"failed downloading"`,
/// was called transient, and told the user `sync` would try it again. It fails identically
/// forever; `exit_policy::luarocks`'s own doc comment names that exact cause and classifies it
/// as the network anyway.
///
/// The evidence was already being collected and thrown away. This loop retries a transient
/// failure with backoff, so by the time it gives up it **has** re-run the command and seen the
/// same answer. That is the experiment; this records its result. `Unknown` rather than
/// `Permanent`, because "we tried and it did not differ" is not "this can never work" — the
/// wget on the PATH could be fixed tomorrow. Withdrawing a declaration is not this function's
/// to trigger either way: that reads `Error::says_a_name_is_absent`, and no amount of repeating
/// turns "the download failed" into "the rock does not exist".
fn falsify_transience(err: Error, attempts: u32) -> Error {
    if attempts < 2 {
        return err; // never retried, so nothing was tested
    }
    match err {
        Error::CommandFailed {
            message,
            retry: Retryability::Transient,
            absent_name,
        } => Error::CommandFailed {
            message: format!(
                "{} (tried {} times; the failure did not change, so a further retry will not \
                 help — this is not the transient failure its output looks like)",
                message, attempts
            ),
            retry: Retryability::Exhausted,
            // Carried, not recomputed. Nothing here re-reads the manager's output, so
            // dropping the flag would turn "the name is not there" into "something failed
            // repeatedly" purely by passing through the retry loop.
            absent_name,
        },
        other => other,
    }
}

#[cfg(test)]
mod transience_tests {
    use super::*;

    fn transient(msg: &str) -> Error {
        Error::CommandFailed {
            message: msg.to_string(),
            retry: Retryability::Transient,
            absent_name: false,
        }
    }

    #[test]
    fn a_transient_failure_that_repeated_stops_calling_itself_transient() {
        let out = falsify_transience(transient("`luarocks` failed: Failed downloading …"), 3);
        assert_eq!(out.retryability(), Retryability::Exhausted);
        assert!(
            out.to_string().contains("did not change"),
            "the message must say what was tried: {out}"
        );
    }

    #[test]
    fn a_failure_that_was_never_retried_keeps_its_classification() {
        // The control. Downgrading on the first attempt would delete the distinction entirely
        // and make every transient failure Unknown, which is the opposite of the fix.
        let out = falsify_transience(transient("`apt` failed: Could not get lock"), 1);
        assert_eq!(out.retryability(), Retryability::Transient);
        assert!(!out.to_string().contains("did not change"));
    }

    #[test]
    fn a_permanent_failure_is_not_touched_by_the_retry_count() {
        // It never entered the retry loop a second time — `give_up` breaks on Permanent — so
        // seeing one here at all would mean something else changed. Pinned so it cannot.
        let e = Error::CommandFailed {
            message: "`scoop` failed: Couldn't find manifest".into(),
            retry: Retryability::Permanent,
            absent_name: true,
        };
        assert_eq!(
            falsify_transience(e, 3).retryability(),
            Retryability::Permanent
        );
    }

    #[test]
    fn an_unknown_failure_is_left_alone() {
        let e = Error::CommandFailed {
            message: "`mix` failed: something".into(),
            retry: Retryability::Unknown,
            absent_name: false,
        };
        let out = falsify_transience(e, 3);
        assert_eq!(out.retryability(), Retryability::Unknown);
        assert!(!out.to_string().contains("did not change"));
    }
}

#[cfg(test)]
mod batching_tests {
    use super::*;
    use crate::core::manager::{BackendCapabilities, BackendCore, HealthReport, HealthStatus};
    use crate::core::{Installable, Package, Queryable};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A manager that counts how many separate commands it was asked to run, and how many
    /// packages the widest of them carried.
    struct Counting {
        name: String,
        calls: AtomicUsize,
        widest: AtomicUsize,
        listings: crate::core::installed::InstalledListings,
    }

    #[async_trait::async_trait]
    impl BackendCore for Counting {
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
        async fn check_health(&self) -> Result<HealthReport> {
            Ok(HealthReport {
                status: HealthStatus::Ok,
                message: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl Installable for Counting {
        async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.widest.fetch_max(specs.len(), Ordering::SeqCst);
            Ok(())
        }
        async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.widest.fetch_max(names.len(), Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Queryable for Counting {
        fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
            (&self.listings, &self.name)
        }
        async fn fetch_installed(&self) -> Result<Vec<Package>> {
            Ok(Vec::new())
        }
        async fn list_manual(&self) -> Result<Vec<Package>> {
            Ok(Vec::new())
        }
        async fn info(&self, _name: &str) -> Result<Option<Package>> {
            Ok(None)
        }
    }

    fn spec(backend: &str, name: &str) -> PackageSpec {
        PackageSpec {
            name: name.to_string(),
            backend: backend.to_string(),
            options: HashMap::new(),
            requires: Vec::new(),
            present: true,
        }
    }

    struct Harness {
        tx: Transaction,
        counters: Vec<Arc<Counting>>,
        _tmp: tempfile::TempDir,
    }

    async fn harness(graph: StableDiGraph<GraphAction, ()>, backends: &[&str]) -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = BackendRegistry::new();
        let mut counters = Vec::new();
        for b in backends {
            let counting = Arc::new(Counting {
                name: b.to_string(),
                calls: AtomicUsize::new(0),
                widest: AtomicUsize::new(0),
                listings: crate::core::installed::InstalledListings::new(),
            });
            counters.push(counting.clone());
            registry.register(Arc::new(
                BackendCapabilities::builder(counting.clone())
                    .with_installable(counting.clone())
                    .with_queryable(counting)
                    .build(),
            ));
        }
        let config = crate::config::Config::default();
        let journal = Journal::at(tmp.path().join("journal.jsonl")).unwrap();
        let diagnostics = crate::app::diagnostics::FailureDiagnosticEngine::init(&config).await;
        let mut tx_config = TransactionConfig::patient();
        // Rollback off: it would ask each backend what was there before, which is not what
        // these tests are measuring.
        tx_config.auto_rollback = false;
        let tx = Transaction::with_config(
            graph,
            Arc::new(registry),
            Arc::new(Mutex::new(journal)),
            Arc::new(diagnostics),
            Arc::new(config),
            tx_config,
        );
        Harness {
            tx,
            counters,
            _tmp: tmp,
        }
    }

    #[tokio::test]
    async fn six_independent_installs_are_one_command() {
        // Measured before this: six declared packages produced six separate apt processes and
        // 12,465 ms, against 3,161 ms for the same packages as one command. The batching code
        // in `generic::install_group` was already written and had never been handed more than
        // one spec.
        let mut graph = StableDiGraph::new();
        for name in ["lolcat", "cowsay", "pv", "sl", "toilet", "cmatrix"] {
            graph.add_node(GraphAction::Install(spec("apt", name)));
        }
        let mut h = harness(graph, &["apt"]).await;
        let results = h.tx.execute_with_telemetry().await.unwrap();

        assert_eq!(
            h.counters[0].calls.load(Ordering::SeqCst),
            1,
            "six packages, one manager, no edges between them — that is one command"
        );
        assert_eq!(h.counters[0].widest.load(Ordering::SeqCst), 6);
        assert_eq!(results.len(), 6, "every package still gets its own result");
        assert!(
            results.iter().all(|r| r.batch_size == 6),
            "the telemetry has to say why six durations are identical"
        );
    }

    #[tokio::test]
    async fn two_managers_are_two_commands_and_not_one() {
        let mut graph = StableDiGraph::new();
        graph.add_node(GraphAction::Install(spec("apt", "jq")));
        graph.add_node(GraphAction::Install(spec("apt", "ripgrep")));
        graph.add_node(GraphAction::Install(spec("npm", "prettier")));
        let mut h = harness(graph, &["apt", "npm"]).await;
        h.tx.execute_with_telemetry().await.unwrap();

        assert_eq!(h.counters[0].calls.load(Ordering::SeqCst), 1);
        assert_eq!(h.counters[0].widest.load(Ordering::SeqCst), 2);
        assert_eq!(h.counters[1].calls.load(Ordering::SeqCst), 1);
        assert_eq!(h.counters[1].widest.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_install_and_a_removal_never_share_a_command() {
        let mut graph = StableDiGraph::new();
        graph.add_node(GraphAction::Install(spec("apt", "jq")));
        graph.add_node(GraphAction::Remove {
            name: "nano".into(),
            backend: "apt".into(),
        });
        let mut h = harness(graph, &["apt"]).await;
        h.tx.execute_with_telemetry().await.unwrap();

        assert_eq!(
            h.counters[0].calls.load(Ordering::SeqCst),
            2,
            "installing and removing are two different commands to the same manager"
        );
        assert_eq!(h.counters[0].widest.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_requires_edge_still_orders_the_two_sides() {
        // A batch is made of what is ready *at the same moment*, so an edge splits it —
        // otherwise a package would go on the same command line as the thing it requires.
        // Only a written `@requires` produces one (`Y9`); a native dependency the manager
        // resolves for itself does not, and the two used to be indistinguishable here.
        let mut graph = StableDiGraph::new();
        let first = graph.add_node(GraphAction::Install(spec("apt", "libfoo")));
        let second = graph.add_node(GraphAction::Install(spec("apt", "foo-tool")));
        graph.add_edge(first, second, ());
        let mut h = harness(graph, &["apt"]).await;
        h.tx.execute_with_telemetry().await.unwrap();

        assert_eq!(
            h.counters[0].calls.load(Ordering::SeqCst),
            2,
            "a required package and its dependent cannot go on one command line"
        );
        assert_eq!(h.counters[0].widest.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_command_line_is_bounded() {
        // Windows caps a command line at 8191 characters, and every manager has some limit.
        let mut graph = StableDiGraph::new();
        for i in 0..(Transaction::MAX_BATCH + 40) {
            graph.add_node(GraphAction::Install(spec("apt", &format!("pkg{}", i))));
        }
        let mut h = harness(graph, &["apt"]).await;
        h.tx.execute_with_telemetry().await.unwrap();

        assert_eq!(h.counters[0].calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            h.counters[0].widest.load(Ordering::SeqCst),
            Transaction::MAX_BATCH
        );
    }
}
