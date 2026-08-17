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
    /// How long to wait for another package manager that holds its own lock. Zero does not
    /// wait. See `manager_lock_wait_secs` in the config for why this is not a backoff.
    pub manager_lock_wait: Duration,
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
            manager_lock_wait: Duration::from_secs(
                crate::config::config::default_manager_lock_wait_secs(),
            ),
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
            manager_lock_wait: Duration::from_secs(config.manager_lock_wait_secs),
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
    /// Proof that the plan this graph came from passed the removal guard.
    ///
    /// `None` until [`Transaction::guarded_by`] is called, and a graph carrying a removal node
    /// **refuses to execute without it**. The guard runs at plan time, in the engine, over the
    /// whole plan at once — which is where it has to run, because `max_removals` is a ceiling
    /// over a plan and cannot be checked one argv at a time. What was missing was any way for
    /// the executor to know it had happened; a plan built by some other path and handed
    /// straight here would have removed packages with nothing in between.
    ///
    /// This one is a runtime refusal rather than a compile error, and that is worth saying
    /// plainly: making it a compile error would mean typing the graph itself by whether it
    /// contains a removal, which is a larger change than this finding earns. The five effectors
    /// **are** compile-enforced; this is the seam that hands them their token.
    reaped: Option<crate::app::sync::guard::Reaped>,
    /// **What this plan intends the machine to end up holding**, as `backend:name` — the
    /// `Install` nodes of the graph being executed.
    ///
    /// **Rollback consults it in both directions, and that is one rule, not two** (`U41`,
    /// amended 2026-08-09). *Rollback does not undo work that moved the machine toward the
    /// declared state.*
    ///
    /// - **An install that succeeded, of something still declared,** is not failed work — it is
    ///   the goal, reached early. `Prior::Absent` says the package was not here before this run;
    ///   it does not say nobody wants it, and the manifest holds the second fact. Removing it
    ///   hands the next `sync` the same work to do again.
    /// - **A removal that succeeded, of something still undeclared,** is the same event seen
    ///   from the other side. The fact that authorised the removal — nothing declares this — is
    ///   still true when the rollback fires, and it is knowable the same way it was knowable
    ///   then: the package is not in this set. Re-installing it un-converges exactly as
    ///   symmetrically.
    ///
    /// **What is lost by the second half, stated plainly:** a package the user had, that this
    /// run removed, stays removed after a failed transaction. The durable put-it-back is
    /// generations and snapshots, which is what they are for; the WAL records the removal but
    /// not the version, so a `Prior` that outlived the process would be the alternative and it
    /// is deferred, not rejected (`U41`).
    ///
    /// `None` for a transaction that is not reconciling against a manifest — see
    /// [`GuardScope::reconciles`](crate::app::sync::guard::GuardScope::reconciles). There the
    /// old behaviour is right in both arms: a `rebuild`'s removal phase is half of a reinstall
    /// of declared packages, and a hand-typed `uninstall` was not derived from anything.
    declared: Option<Arc<std::collections::HashSet<String>>>,
    /// What the last execution's scheduling actually looked like — packages, graph depth, and
    /// the number of times the engine went idle and handed out more work.
    ///
    /// Recorded rather than only logged, because the shape rule for `Mutating` is otherwise
    /// asserted by reading a `warn!` line. A warning nothing can read is the shape of a gate
    /// that cannot fail, which is what the rule was written to replace.
    pub last_scheduling: Option<Scheduling>,
}

/// How serially one execution actually ran, against how serial its graph forced it to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scheduling {
    /// Nodes this run had left to do — the whole graph, less anything a resume had finished.
    pub packages: usize,
    /// The longest dependency chain among them, which is the fewest waves any correct scheduler
    /// could take.
    pub depth: usize,
    /// Times the engine had nothing in flight and then handed work out. At most `depth` for a
    /// scheduler that dispatches every ready node the moment it is ready; `packages` for a
    /// serial loop. Below `depth` whenever independent chains overlap, which is why the rule is
    /// an inequality and not an equality.
    pub waves: usize,
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
            reaped: None,
            declared: None,
            hooks: None,
            completed_indices: HashSet::new(),
            history: Vec::new(),
            cancellation_token: CancellationToken::new(),
            last_scheduling: None,
        }
    }

    /// Packages with no configured hook (and no `*` wildcard) incur only a cheap map
    /// lookup, so this is safe to always set.
    pub fn with_hooks(mut self, hooks: Arc<LuaHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Hand the executor proof that this plan's removals passed the guard.
    ///
    /// Required before executing any graph that contains a `Remove` node — see the `reaped`
    /// field. A graph of pure installs needs nothing, which is why this is a builder step
    /// rather than a constructor argument: an install-only plan should not have to produce a
    /// removal authorisation it has no removals for.
    pub fn guarded_by(mut self, reaped: crate::app::sync::guard::Reaped) -> Self {
        self.reaped = Some(reaped);
        self
    }

    /// Tell rollback what the manifest still asks for, as `backend:name`.
    ///
    /// See the `declared` field. Without it, rollback compensates work that succeeded and is
    /// still wanted, and `auto_rollback: true` — the default at `transaction.rs` — becomes an
    /// anti-convergent step. `heal`, whose entire job is the same failure shape, sets
    /// `auto_rollback: false`; nothing explained the split, and this is what it was standing in
    /// for.
    pub fn reconciling(mut self, declared: Arc<std::collections::HashSet<String>>) -> Self {
        self.declared = Some(declared);
        self
    }

    /// Does this plan intend the machine to end up holding this package?
    ///
    /// `None` when the run is not reconciling against a manifest and the question has no
    /// answer. **The whole of `U41` is that both rollback arms ask this one question**: the
    /// install arm skips its removal on `Some(true)`, the removal arm skips its reinstate on
    /// `Some(false)`, and neither does anything on `None`. Written as a function rather than
    /// twice inline so the symmetry is a fact about the code and not about two comments.
    fn plan_intends_present(&self, backend: &str, name: &str) -> Option<bool> {
        self.declared
            .as_ref()
            .map(|d| d.contains(&format!("{}:{}", backend, name)))
    }

    /// The WAL entries that were already open before this run started.
    ///
    /// **Snapshotted so that [`close_stranded`](Self::close_stranded) can tell the two kinds of
    /// open entry apart.** An entry left open by an *earlier* run is the record that a process
    /// died holding it, and it is the only thing that tells `heal` a real crash happened —
    /// closing it would erase the recovery state this log exists to keep.
    async fn open_before_this_run(&self) -> std::collections::HashSet<String> {
        self.journal
            .lock()
            .await
            .interrupted_actions()
            .into_iter()
            .map(|e| e.id)
            .collect()
    }

    /// Close every entry *this* run opened and then abandoned.
    ///
    /// **A batch that is aborted never reaches either of the calls that close its entry.** Both
    /// ways out of a failed run kill their workers — `abort_all` on the first failure, and the
    /// `JoinSet` being dropped on the way out — so a task that had called `record_start` and was
    /// still inside the manager's command dies with its entry `InProgress`.
    ///
    /// That state means "a process died holding this". Left behind by a run Shall itself
    /// stopped, it makes [`Journal::needs_recovery`] answer yes for ever and sends `heal`
    /// looking for a crash that never happened — the same class of defect as reporting that
    /// nothing is wrong, pointing the other way. Shall aborted these and knows it did, so it
    /// says so.
    ///
    /// Measured: the macOS nightly of 2026-08-14 ran an unrefused `purge-undeclared` over 276
    /// removals, `gem:logger` failed, `continue_on_error` was off, and the rollback aborted
    /// every other manager's batch mid-command. The harness reported *"22 operation(s) are
    /// still open in the write-ahead log and nothing crashed"*.
    async fn close_stranded(&self, open_before: &std::collections::HashSet<String>, why: &Error) {
        let mut j = self.journal.lock().await;
        let stranded: Vec<String> = j
            .interrupted_actions()
            .into_iter()
            .map(|e| e.id)
            .filter(|id| !open_before.contains(id))
            .collect();
        if stranded.is_empty() {
            return;
        }
        let reason = format!("abandoned when the run stopped: {}", why);
        warn!(
            "{} operation(s) were still open when the run stopped; closing them as failed \
             rather than leaving them to read as a crash",
            stranded.len()
        );
        for id in stranded {
            let _ = j.record_failure(&id, &reason);
        }
    }

    pub async fn execute_with_telemetry(&mut self) -> Result<Vec<TaskResult>> {
        let total_timeout = self.config.total_timeout;
        let start_time = Instant::now();

        info!(
            "Initializing parallel execution for {} nodes.",
            self.graph.node_count()
        );

        // Taken before any work, and read on both failing paths below. One place, because the
        // two ways a run can strand an entry are one bug.
        let open_before = self.open_before_this_run().await;

        match tokio::time::timeout(total_timeout, self.execute_internal()).await {
            Ok(Ok(results)) => {
                debug!("DAG closure reached in {:?}", start_time.elapsed());
                Ok(results)
            }
            Ok(Err(e)) => {
                self.close_stranded(&open_before, &e).await;
                Err(e)
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
                let e =
                    Error::Transaction(format!("Transaction timed out after {:?}", total_timeout));
                self.close_stranded(&open_before, &e).await;
                Err(e)
            }
        }
    }

    pub async fn execute(&mut self) -> Result<()> {
        self.execute_with_telemetry().await.map(|_| ())
    }

    /// The most waves this plan can honestly take: its longest remaining dependency chain.
    ///
    /// Measured over the nodes still to run, so a resumed transaction is judged on the work it
    /// has left rather than on work somebody else already did. A cycle has no depth and the
    /// planner rejects one long before here; returning zero switches the check off rather than
    /// inventing a number for a plan that cannot be scheduled at all.
    fn critical_path_depth(&self) -> usize {
        let Ok(order) = petgraph::algo::toposort(&self.graph, None) else {
            return 0;
        };
        let mut level: HashMap<NodeIndex, usize> = HashMap::new();
        let mut deepest = 0;
        for idx in order
            .into_iter()
            .filter(|i| !self.completed_indices.contains(i))
        {
            let depth = self
                .graph
                .neighbors_directed(idx, Direction::Incoming)
                .filter_map(|parent| level.get(&parent).copied())
                .max()
                .map_or(1, |deepest_parent| deepest_parent + 1);
            level.insert(idx, depth);
            deepest = deepest.max(depth);
        }
        deepest
    }

    async fn execute_internal(&mut self) -> Result<Vec<TaskResult>> {
        let total_nodes = self.graph.node_count();
        let mut in_progress = HashSet::new();
        let mut worker_pool = JoinSet::new();
        let mut telemetry_results = Vec::new();

        // The shape budget for `Mutating`, which the fan-out rule in `latency.rs` cannot
        // express: both quantities come from this graph, so the engine reports its own shape
        // rather than being inspected from a layer that is handed a subcommand name and nothing
        // else. Taken before the run, because the loop below is what changes `completed_indices`.
        let depth = self.critical_path_depth();
        let remaining = total_nodes - self.completed_indices.len();
        let mut waves = 0usize;

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

            // One package per command under `--keep-going`, so a name no repository carries
            // cannot take the installable packages beside it down with it.
            let max_batch = match self.config.continue_on_error {
                true => 1,
                false => Self::MAX_BATCH,
            };
            // **A wave is work handed out after the engine went quiet — not every pass that
            // dispatched something.** Counting passes looks equivalent and is not: two
            // independent chains finishing at different moments produce a pass per completion,
            // which is the scheduler dispatching *eagerly*, and a rule that counted those would
            // fail the runs it exists to reward. What cannot happen in a correct run is going
            // idle more times than the graph has levels, because each idle restart means an
            // entire generation finished before the next was handed out.
            let dispatching = std::mem::take(&mut ready);
            if !dispatching.is_empty() && in_progress.is_empty() {
                waves += 1;
            }
            for batch in Self::batches(&self.graph, dispatching, max_batch) {
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
                let app_config = self.app_config.clone();
                let reaped = self.reaped;
                let hooks = self.hooks.clone();

                worker_pool.spawn(async move {
                    let _permit_holder = permit;
                    Self::execute_batch_with_retry(
                        batch,
                        registry,
                        journal,
                        config,
                        app_config,
                        reaped,
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
                            GraphAction::Install(s) => {
                                s.options.one("__source").map(str::to_string)
                            }
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
        // Only on the path where the loop ran to closure. A transaction that returned early
        // went idle fewer times than its graph has levels, so measuring it reports a shape no
        // run had — and `waves <= depth` being unfalsifiable there is exactly why the number is
        // not worth keeping.
        self.last_scheduling = Some(Scheduling {
            packages: remaining,
            depth,
            waves,
        });
        if let Some(why) = crate::core::latency::scheduling_violation(remaining, depth, waves) {
            tracing::warn!(
                "this plan was executed more serially than it is shaped: {}. The seconds a \
                 package manager costs belong to the host; the order Shall asks in does not.",
                why
            );
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
    /// the reason names the node that stopped it, because "jq failed" about a package Shall
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

    /// The most packages Shall will put on one manager command line.
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
    ///
    /// `max_batch` is a cap the caller chooses rather than [`Self::MAX_BATCH`] outright,
    /// because **batching and `--keep-going` are in direct contradiction and batching used to
    /// win silently.** One name no repository carries fails the whole `apt install`, so a run
    /// with `--keep-going` — whose help promises to "finish the packages that still can" —
    /// installed none of the good packages sharing that command line (B1). A flag whose entire
    /// purpose is taking what it can get does not want them on one command line; it wants each
    /// package to succeed or fail on its own. That costs invocations, and invocations are what
    /// this flag is explicitly willing to spend.
    fn batches(
        graph: &StableDiGraph<GraphAction, ()>,
        mut ready: Vec<NodeIndex>,
        max_batch: usize,
    ) -> Vec<Batch> {
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
                    && (current.len() >= max_batch.max(1) || bytes + cost > Self::MAX_BATCH_BYTES)
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
    /// runs, and a batch that fails fails every package in it.
    ///
    /// **That last clause is only harmless because of who is allowed to batch.** Without
    /// `--keep-going` any node failure rolls the whole transaction back, so the packages
    /// sharing a failed command line were going to be undone regardless. With it, they were
    /// not — and batching quietly took the good packages down with the bad name, under a flag
    /// promising the opposite (B1). `batches` caps at one package per command in that mode, so
    /// nothing reaching here shares a fate it did not have to.
    #[allow(clippy::too_many_arguments)]
    async fn execute_batch_with_retry(
        batch: Batch,
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        config: TransactionConfig,
        // The user's configuration, for the one question a failed install asks of it: was the
        // version this batch could not get a version Shall recorded rather than one they typed?
        app_config: Arc<crate::config::Config>,
        reaped: Option<crate::app::sync::guard::Reaped>,
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
                        // **Close what this batch already opened before leaving.** `ids` holds
                        // the entries `record_start` did accept, and an early return past them
                        // leaves each one `InProgress` for ever — which is the state that means
                        // "a process died holding this" and sends `heal` looking for a crash
                        // that never happened. None can already be closed: the manager has not
                        // been invoked yet.
                        let why = format!("WAL error before this batch ran: {}", e);
                        for id in &ids {
                            let _ = j.record_failure(id, &why);
                        }
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
        // Nothing to close on the way out: the only thing that removes a member from `keep` is
        // the `before_install` arm above, which closes that member's entry as it drops it. This
        // is the one early return in the window between opening the WAL entries and closing
        // them that is allowed to leave without touching them.
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

        // **A compensating install carries a version nobody declared** (`Q53`). Rollback
        // reads what a package was on before the transaction touched it and puts that back,
        // so a version reaches this point on a manager the planner never got to vet — and
        // on a manager that cannot be asked for one there is nothing to send.
        //
        // Stripped and **named**, not refused: refusing would end a rollback with the
        // package uninstalled, which is worse than putting it back at what the manager
        // offers and saying which version could not be restored. The declared case is the
        // one that gets refused, and it is refused at plan time.
        //
        // Before this, `brew.rs` answered the same situation by building `pkg-a@1.0` — a
        // formula name that does not exist — so the rollback failed on a real Mac and
        // passed in every test, because a mock matches any string.
        let specs: Vec<PackageSpec> = if is_install
            && backend_cap
                .as_installable()
                .is_some_and(|i| !i.pins_version())
        {
            specs
                .into_iter()
                .map(|mut spec| {
                    match spec.options.one("version").map(str::to_string) {
                        Some(v) if crate::backends::concrete_version(&v) => {
                            warn!(
                                "{}:{} was on {} and `{}` cannot be asked for a version — \
                                 putting it back at whatever `{}` offers.{}",
                                b_name,
                                spec.name,
                                v,
                                b_name,
                                b_name,
                                match crate::backends::capability::cannot_pin_reason(&b_name) {
                                    Some(why) => format!(" {}.", why),
                                    None => String::new(),
                                }
                            );
                            spec.options.remove("version");
                        }
                        _ => {}
                    }
                    spec
                })
                .collect()
        } else {
            specs
        };

        let mut attempt = 0;
        let mut last_error = None;
        let mut lock_waited = Duration::ZERO;

        while attempt <= config.max_retries {
            attempt += 1;
            if cancel_token.is_cancelled() {
                // **A cancelled batch closes its entries, exactly as a failed one does.** This
                // return sits between `record_start` and the `record_success`/`record_failure`
                // below it and took neither, so a batch that noticed the cancellation here left
                // its entries `InProgress` — the state that means a process died holding them.
                //
                // `close_stranded` is the backstop for the batches that are killed outright and
                // never reach any code at all; this is the one that gets to say something more
                // accurate than "abandoned", because the batch is still alive to say it.
                //
                // `keep`, not `ids`: a package whose `before_install` hook failed was closed
                // above, and this is the same set the two closing paths below use.
                {
                    let mut j = journal.lock().await;
                    for &i in &keep {
                        let _ = j.record_failure(
                            &ids[i],
                            "cancelled before this batch ran: another operation in the same run \
                             failed",
                        );
                    }
                }
                refused.extend(keep.iter().map(|&i| {
                    let (idx, _, name) = &members[i];
                    TaskResult {
                        node_index: *idx,
                        backend_name: b_name.clone(),
                        package_name: name.clone(),
                        retries: retries_behind(attempt),
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
                // **Another package manager is not a failure to back off from — it is one to
                // wait for.** A backoff is for a flake; this is a second program holding a lock
                // it will hand back when its own transaction finishes, and three doublings of
                // half a second do not outlast an `apt upgrade`. Only ever entered against a
                // holder proved to be alive: a lock left behind by a killed run is reported at
                // once, because waiting on it would never end.
                // One budget across the whole retry loop, not one per attempt. A queue of
                // holders taking the lock in turn is a real machine state, and three full waits
                // in a row would be three times the bound the setting promises.
                let budget = config.manager_lock_wait.saturating_sub(lock_waited);
                match lock_wait_verdict(&last_error, &b_name, budget, &|b| {
                    crate::app::stale_lock::held_for_on_this_machine(b)
                }) {
                    LockWait::Wait(who) => {
                        match wait_for_manager_lock(&b_name, &who, budget, &cancel_token).await {
                            Ok(spent) => lock_waited += spent,
                            Err(err) => {
                                last_error = Some(err);
                                break;
                            }
                        }
                    }
                    LockWait::Hopeless(err) => {
                        last_error = Some(err);
                        break;
                    }
                    LockWait::Backoff => {
                        let backoff =
                            backoff_for(attempt, config.initial_backoff, config.max_backoff);
                        tokio::time::sleep(backoff).await;
                    }
                }
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
                    let Some(reaped) = reaped else {
                        return Err(crate::core::Error::Refused(format!(
                            "a plan containing removals reached the executor without passing                              the removal guard — refusing to remove {}. This is a defect in                              whichever command built the plan, not in the config: the guard                              runs once over a whole plan (`max_removals` is a ceiling over a                              plan, not over one command), and the engine hands the executor                              the proof it ran.",
                            names.join(", ")
                        )));
                    };
                    if config.purge && handler.supports_purge() {
                        handler.purge(&names, sudo, reaped).await
                    } else {
                        handler.remove(&names, sudo, reaped).await
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
                            retries: retries_behind(attempt),
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

        let mut final_err = falsify_transience(
            last_error.unwrap_or(Error::Transaction("Unknown error".into())),
            attempt,
        );
        // **A version the user never typed must not fail in the user's face unexplained.** The
        // batch names one manager and up to `batch_size` packages, and the manager's complaint
        // quotes whichever of them it choked on — so each member is asked, and only the one
        // whose recorded pin appears in the text answers.
        if let Some(advice) = keep.iter().find_map(|&i| {
            crate::app::sync::pin_advice::on_install_failure(
                &app_config,
                &b_name,
                &members[i].2,
                &final_err.to_string(),
            )
        }) {
            final_err = Error::Transaction(format!("{}{}", final_err, advice));
        }
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
                retries: retries_behind(attempt),
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
        let mut options = crate::config::grammar::Options::default();
        if let Some(v) = version {
            // Without this the reinstall takes whatever is newest, so a rolled-back removal
            // silently loses its pin — the package comes back at a version nobody declared.
            options.set("version", v.clone());
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
                            // **`Prior::Absent` is not permission to remove.** It says the
                            // package was not here before this run; it does not say nobody wants
                            // it. If the manifest still declares it, this install is the goal
                            // reached early, and compensating it hands the next `sync` the same
                            // work to do again — the transaction's own comment at `:637` claims
                            // rollback "puts this back", and removing something nothing asked it
                            // to remove is the opposite.
                            if self.plan_intends_present(&spec.backend, &spec.name) == Some(true) {
                                info!(
                                    "rollback is leaving {}:{} installed — it succeeded and the                                      manifest still declares it, so removing it would only give                                      the next sync the same work to do again.",
                                    spec.backend, spec.name
                                );
                                continue;
                            }
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
                            // Rollback asks `protection_of` itself, four lines above, and its
                            // removals are of packages this same run installed seconds ago —
                            // so it is one of the two named cases that do not re-ask.
                            let reaped = crate::app::sync::guard::Reaped::for_reason(
                                crate::app::sync::guard::GuardScope::Sync,
                                "rollback checks `protection_of` itself at transaction.rs:993,                                  and compensates only work this run performed",
                            );
                            if let Err(e) = h
                                .remove(
                                    std::slice::from_ref(&spec.name),
                                    b.sudo_for_write(),
                                    reaped,
                                )
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
                                "rollback will not remove {}:{}: Shall could not tell whether \
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
                    // **The install arm's rule, from the other side** (`U41`). This removal
                    // happened because nothing in the plan intends the package to be present,
                    // and that fact is still true — it is the same set that authorised the
                    // removal, asked the same way. Re-installing it would hand the next sync
                    // the same work to do again, which is the un-convergence the install arm
                    // already refuses to cause.
                    //
                    // `declared` is `None` for the runs where a removal is not a reconciliation
                    // — a `rebuild`'s down phase, a hand-typed `uninstall` — and there the
                    // reinstate below is exactly right.
                    if self.plan_intends_present(&backend, &name) == Some(false) {
                        info!(
                            "rollback is leaving {}:{} removed — nothing declares it, so putting \
                             it back would only give the next sync the same work to do again. \
                             `shall history` and the pre-sync snapshot are how it comes back.",
                            backend, name
                        );
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

/// What to do about a failed attempt whose manager said its lock was taken.
#[derive(Debug)]
enum LockWait {
    /// A live holder, named. Wait for it.
    Wait(String),
    /// Waiting would never end. Fail now, with the sentence that says why.
    Hopeless(Error),
    /// Not a lock failure, or the lock is free again. The ordinary backoff.
    Backoff,
}

/// Which of the three a failure is.
///
/// The verdict is taken from the machine and not from the message: the manager only says *"could
/// not get lock"*, and whether that is a queue to join or a corpse to clear is the difference
/// between waiting five minutes and waiting forever. `/proc` knows; the string does not.
/// `look` is how the machine is asked, so the three verdicts can be exercised without a second
/// package manager to kill. It is called only *after* the manager's own words have matched, which
/// is what keeps a successful install from ever reading `/proc`.
fn lock_wait_verdict(
    last_error: &Option<Error>,
    backend: &str,
    wait: Duration,
    look: &dyn Fn(&str) -> crate::app::stale_lock::Held,
) -> LockWait {
    let Some(err) = last_error else {
        return LockWait::Backoff;
    };
    if !crate::app::stale_lock::says_the_lock_is_taken(backend, &err.to_string()) {
        return LockWait::Backoff;
    }
    match look(backend) {
        crate::app::stale_lock::Held::Live(who) if !wait.is_zero() => LockWait::Wait(who),
        // Opted out of waiting. The message is still the true one rather than the old
        // "a further retry will not help", because a further retry is exactly what would help,
        // once the holder is done.
        crate::app::stale_lock::Held::Live(who) => LockWait::Hopeless(Error::CommandFailed {
            message: format!(
                "`{backend}` cannot run: {who} holds the manager's lock, and \
                 `manager_lock_wait_secs` is 0, so Shall did not wait for it. Raise that setting \
                 or run this again once the other manager has finished."
            ),
            retry: Retryability::Exhausted,
            absent_name: false,
        }),
        crate::app::stale_lock::Held::Stale(path) => LockWait::Hopeless(Error::CommandFailed {
            message: format!(
                "`{backend}` cannot run: {} is on disk and nothing holds it — a run of this \
                 manager was killed and left its lock behind. Waiting will not clear it. \
                 `shall heal` removes exactly this, after proving again that no manager is \
                 running.",
                path.display()
            ),
            retry: Retryability::Exhausted,
            absent_name: false,
        }),
        crate::app::stale_lock::Held::Free => LockWait::Backoff,
    }
}

/// Wait for whoever holds the manager's lock, and say so while waiting.
///
/// `None` means the lock came free and the caller should try again. `Some(err)` is the wait
/// ending without that happening, and it says which of the two ways it ended.
///
/// **A wait with no reason given is indistinguishable from a hang**, and a hang is what people
/// kill — which is how a machine ends up with the interrupted transaction this whole module is
/// about. It announces once, up front, the way the data-directory lock does.
async fn wait_for_manager_lock(
    backend: &str,
    who: &str,
    wait: Duration,
    cancel_token: &CancellationToken,
) -> std::result::Result<Duration, Error> {
    eprintln!(
        "shall: waiting for {who} to finish — it holds the lock `{backend}` needs \
         (up to {}s; `manager_lock_wait_secs` sets that)",
        wait.as_secs()
    );
    // The polling loop is `stale_lock`'s, because `heal` waits on the same question and the two
    // must not drift. What is decided here is only what to say about each ending.
    match crate::app::stale_lock::wait_until_not_held(backend, wait, &|| {
        cancel_token.is_cancelled()
    })
    .await
    {
        // It let go. Whether it finished or died, the next attempt is the thing that finds out,
        // and a stale lock left by a holder that died mid-wait is reported by the next pass
        // through the verdict rather than guessed at here.
        crate::app::stale_lock::Waited::Freed(spent) => {
            info!(
                "the lock `{}` needs came free after {}s",
                backend,
                spent.as_secs()
            );
            return Ok(spent);
        }
        crate::app::stale_lock::Waited::Cancelled => return Err(Error::Cancelled),
        crate::app::stale_lock::Waited::StillHeld => {}
    }
    Err(Error::CommandFailed {
        message: format!(
            "`{backend}` cannot run: {who} has held the manager's lock for {}s, which is all \
             `manager_lock_wait_secs` allows. It is still running, so nothing here is broken and \
             nothing needs clearing — run this again when it has finished, or raise that setting.",
            wait.as_secs()
        ),
        retry: Retryability::Exhausted,
        absent_name: false,
    })
}

/// How many of `attempt` tries were retries: all but the first.
///
/// `attempt` counts from 1, so a batch that succeeded first time reports nought. Written out
/// because three `TaskResult` constructions computed it inline, and a copy of an expression is
/// a copy of its blind spot — the mutation sweep found the same subtraction three times over
/// and nothing in the suite could tell any of them from `attempt + 1`.
fn retries_behind(attempt: u32) -> u32 {
    attempt - 1
}

/// The wait before retry number `attempt`, doubling each time and capped at `max`.
///
/// Only ever called from inside `attempt > 1`, so the first retry waits `initial` exactly and
/// the exponent cannot go negative. Written out for the same reason as [`retries_behind`], with
/// more to answer for: a shift inside a multiplication inside a `min` inside a match arm is
/// three separate numbers, and all three survived — `<<` read as `>>`, and `attempt - 2` read
/// as both `attempt + 2` and `attempt / 2`, without failing anything.
fn backoff_for(attempt: u32, initial: Duration, max: Duration) -> Duration {
    std::cmp::min(initial * (1 << (attempt - 2)), max)
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

/// **What Shall does when another package manager holds the lock** — the three verdicts, each
/// exercised without a second package manager to kill.
///
/// The shipped behaviour was one verdict for all three: four retries over three and a half
/// seconds, then *"the failure did not change, so a further retry will not help — this is not the
/// transient failure its output looks like"*. That sentence was printed most often in the one
/// case where it was false.
#[cfg(test)]
mod manager_lock_tests {
    use super::*;
    use crate::app::stale_lock::Held;

    fn lock_failure(msg: &str) -> Option<Error> {
        Some(Error::CommandFailed {
            message: msg.to_string(),
            retry: Retryability::Transient,
            absent_name: false,
        })
    }

    const PACMAN_SAID: &str = "`pacman` failed (exit 1): error: failed to init transaction \
                               (unable to lock database)";

    /// A live holder is a queue to join. Waiting is the only thing that helps, and it is what
    /// Shall already does for its own lock.
    #[test]
    fn a_live_holder_is_waited_for() {
        let verdict = lock_wait_verdict(
            &lock_failure(PACMAN_SAID),
            "pacman",
            Duration::from_secs(300),
            &|_| Held::Live("a `pacman`".into()),
        );
        assert!(
            matches!(&verdict, LockWait::Wait(who) if who.contains("pacman")),
            "{verdict:?}"
        );
    }

    /// A lock nothing holds is a corpse, and waiting for it never ends. It fails at once, and
    /// the message names the command that clears it rather than three more retries.
    #[test]
    fn a_stale_lock_fails_at_once_and_names_heal() {
        let verdict = lock_wait_verdict(
            &lock_failure(PACMAN_SAID),
            "pacman",
            Duration::from_secs(300),
            &|_| Held::Stale("/var/lib/pacman/db.lck".into()),
        );
        let LockWait::Hopeless(err) = verdict else {
            panic!("waiting on a lock nothing holds never ends: {verdict:?}");
        };
        let said = err.to_string();
        assert!(said.contains("shall heal"), "{said}");
        assert!(said.contains("db.lck"), "the file has to be named: {said}");
        assert_eq!(err.retryability(), Retryability::Exhausted);
    }

    /// The holder let go between the failure and the question. That is an ordinary race, and the
    /// ordinary backoff is the right answer — not a wait for a lock that is already free.
    #[test]
    fn a_lock_that_came_free_goes_back_to_the_backoff() {
        let verdict = lock_wait_verdict(
            &lock_failure(PACMAN_SAID),
            "pacman",
            Duration::from_secs(300),
            &|_| Held::Free,
        );
        assert!(matches!(verdict, LockWait::Backoff), "{verdict:?}");
    }

    /// **The machine is not consulted for a failure that is not about a lock.** A wait on every
    /// failed install would be a hang on every typo, and the `/proc` scan would be paid on
    /// every one of them.
    #[test]
    fn a_failure_that_is_not_about_a_lock_never_asks_the_machine() {
        let asked = std::cell::Cell::new(false);
        let verdict = lock_wait_verdict(
            &lock_failure("`pacman` failed (exit 1): error: target not found: qqqq"),
            "pacman",
            Duration::from_secs(300),
            &|_| {
                asked.set(true);
                Held::Live("a `pacman`".into())
            },
        );
        assert!(matches!(verdict, LockWait::Backoff), "{verdict:?}");
        assert!(
            !asked.get(),
            "the machine was scanned over a missing package"
        );
    }

    /// And a backend with no lock in the table is never made to wait for one, whatever its
    /// failure happens to say.
    #[test]
    fn a_backend_with_no_manager_lock_backs_off_as_before() {
        let verdict = lock_wait_verdict(
            &lock_failure("`npm` failed: could not get lock"),
            "npm",
            Duration::from_secs(300),
            &|_| panic!("npm has no manager lock, so nothing should have been asked"),
        );
        assert!(matches!(verdict, LockWait::Backoff), "{verdict:?}");
    }

    /// `manager_lock_wait_secs = 0` opts out of waiting — and still does not print the old
    /// sentence, because a further retry is exactly what *would* help once the holder is done.
    #[test]
    fn opting_out_of_the_wait_still_says_something_true() {
        let verdict = lock_wait_verdict(
            &lock_failure(PACMAN_SAID),
            "pacman",
            Duration::ZERO,
            &|_| Held::Live("a `pacman`".into()),
        );
        let LockWait::Hopeless(err) = verdict else {
            panic!("with no wait allowed there is nothing to wait for: {verdict:?}");
        };
        let said = err.to_string();
        assert!(said.contains("manager_lock_wait_secs"), "{said}");
        assert!(
            !said.contains("a further retry will not help"),
            "the old sentence is the false one: {said}"
        );
    }

    /// Nothing has failed yet on the first attempt, so there is nothing to classify.
    #[test]
    fn no_failure_yet_is_not_a_lock_failure() {
        let verdict = lock_wait_verdict(&None, "pacman", Duration::from_secs(300), &|_| {
            panic!("there is no error to have been about a lock")
        });
        assert!(matches!(verdict, LockWait::Backoff), "{verdict:?}");
    }

    /// The wait ends rather than running to its deadline when the run is cancelled — a Ctrl-C
    /// during a five-minute wait must not become a five-minute wait.
    #[tokio::test]
    async fn a_cancelled_run_stops_waiting_immediately() {
        let token = CancellationToken::new();
        token.cancel();
        let out = wait_for_manager_lock("pacman", "a `pacman`", Duration::from_secs(300), &token)
            .await
            .expect_err("a cancelled wait does not succeed");
        assert!(matches!(out, Error::Cancelled), "{out:?}");
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

    /// A run that never retried reports no retries.
    ///
    /// `attempt` counts tries and `retries` counts the ones after the first, and the three
    /// `TaskResult` sites that used to spell that out inline each carried the same untested
    /// subtraction. There is one now, and this is what stops it reading `attempt + 1`.
    #[test]
    fn retries_are_every_attempt_but_the_first() {
        assert_eq!(retries_behind(1), 0, "the first try is not a retry");
        assert_eq!(retries_behind(2), 1);
        assert_eq!(retries_behind(4), 3);
    }

    /// The backoff doubles from the first retry, and the cap is a cap.
    ///
    /// Called only with `attempt >= 2`, so attempt 2 — the first retry — waits `initial`
    /// exactly. Each value below is one the shift, the exponent or the multiplication would
    /// get wrong on its own.
    #[test]
    fn the_backoff_doubles_from_the_first_retry_and_stops_at_the_cap() {
        let initial = Duration::from_millis(500);
        let max = Duration::from_secs(30);

        assert_eq!(
            backoff_for(2, initial, max),
            Duration::from_millis(500),
            "the first retry waits the initial backoff, undoubled"
        );
        assert_eq!(backoff_for(3, initial, max), Duration::from_secs(1));
        assert_eq!(backoff_for(4, initial, max), Duration::from_secs(2));
        assert_eq!(
            backoff_for(7, initial, max),
            Duration::from_secs(16),
            "still doubling while it is under the cap"
        );
        assert_eq!(
            backoff_for(8, initial, max),
            max,
            "thirty-two seconds is over a thirty-second cap, so the cap is what is waited"
        );
    }

    /// Two attempts is the experiment; one is not.
    ///
    /// The tests above use three attempts and one, and `attempts < 2` reads `<= 2` without
    /// either of them noticing — which quietly raises the bar to three, so the *second*
    /// identical failure stops counting as evidence. Two is the smallest number of attempts
    /// that can show a failure repeating, and it is the only value at which the two spellings
    /// disagree.
    #[test]
    fn the_second_attempt_is_already_a_repeat() {
        let out = falsify_transience(transient("`gem` failed: Connection reset"), 2);
        assert_eq!(
            out.retryability(),
            Retryability::Exhausted,
            "a failure seen twice has been tested once, which is the whole experiment"
        );
        assert!(
            out.to_string().contains("tried 2 times"),
            "the message must say how many attempts stand behind it: {out}"
        );
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
mod from_config_tests {
    use super::*;

    /// Every setting this constructor exists to carry is actually carried.
    ///
    /// The doc comment on `from_config` records why it exists: `max_concurrent` had been left
    /// at the `patient()` default and nobody noticed, because a field that silently falls back
    /// to a sensible number looks exactly like a field that was read. `..Self::patient()` makes
    /// that the failure mode for every line in the struct — deleting `max_concurrent`, `purge`
    /// or `manager_lock_wait` left the suite green, which is the same defect the constructor
    /// was written to prevent, three more times.
    ///
    /// `max_concurrent` is asserted at two different values rather than one. The fallback is
    /// this machine's parallelism, so a single value can agree with it by luck on whatever host
    /// happens to run the test; two values cannot both be it.
    #[test]
    fn every_setting_from_config_claims_to_read_reaches_the_transaction() {
        let base = crate::config::Config::default;

        for parallel in [1usize, 64] {
            let mut config = base();
            config.max_parallel = parallel;
            assert_eq!(
                TransactionConfig::from_config(&config).max_concurrent,
                parallel,
                "max_parallel = {parallel} did not reach max_concurrent"
            );
        }
        let mut none = base();
        none.max_parallel = 0;
        assert_eq!(
            TransactionConfig::from_config(&none).max_concurrent,
            1,
            "nought parallelism is one worker, not none"
        );

        // Purge is an OR of a persistent setting and a this-run flag, and each half must be
        // able to turn it on alone — `&&` reads identically until you ask one of them by itself.
        let mut persistent = base();
        persistent.remove.purge = true;
        assert!(
            TransactionConfig::from_config(&persistent).purge,
            "`[remove] purge = true` alone must purge"
        );
        let mut this_run = base();
        this_run.purge_this_run = true;
        assert!(
            TransactionConfig::from_config(&this_run).purge,
            "`--purge` alone must purge"
        );
        assert!(
            !TransactionConfig::from_config(&base()).purge,
            "neither set is not a purge"
        );

        let mut keep_going = base();
        keep_going.keep_going_this_run = true;
        assert!(TransactionConfig::from_config(&keep_going).continue_on_error);
        assert!(!TransactionConfig::from_config(&base()).continue_on_error);

        // A wait no default would produce, so the fallback cannot pass for the setting.
        let mut waiting = base();
        waiting.manager_lock_wait_secs = 4_321;
        assert_eq!(
            TransactionConfig::from_config(&waiting).manager_lock_wait,
            Duration::from_secs(4_321),
            "manager_lock_wait_secs did not reach the transaction"
        );
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
        /// How long this backend takes to answer. Zero for every test but the ones that need
        /// a batch to still be in flight when the run stops.
        stall: Duration,
        /// Whether this backend refuses everything, so a test can make one manager fail while
        /// another is mid-command.
        fails: bool,
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
            tokio::time::sleep(self.stall).await;
            self.answer()
        }
        async fn remove(
            &self,
            names: &[String],
            _sudo: bool,
            _reaped: crate::app::sync::guard::Reaped,
        ) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.widest.fetch_max(names.len(), Ordering::SeqCst);
            tokio::time::sleep(self.stall).await;
            self.answer()
        }
    }

    impl Counting {
        fn answer(&self) -> Result<()> {
            match self.fails {
                true => Err(Error::Transaction(format!("`{}` refuses", self.name))),
                false => Ok(()),
            }
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
            options: Default::default(),
            requires: Vec::new(),
            present: true,
        }
    }

    struct Harness {
        tx: Transaction,
        counters: Vec<Arc<Counting>>,
        /// The same log the transaction writes to, so a test can ask what it was left holding.
        journal: Arc<Mutex<Journal>>,
        _tmp: tempfile::TempDir,
    }

    async fn harness(graph: StableDiGraph<GraphAction, ()>, backends: &[&str]) -> Harness {
        harness_with(
            graph,
            backends,
            Duration::ZERO,
            Duration::from_secs(3600),
            &[],
        )
        .await
    }

    async fn harness_with(
        graph: StableDiGraph<GraphAction, ()>,
        backends: &[&str],
        stall: Duration,
        total_timeout: Duration,
        failing: &[&str],
    ) -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let mut registry = BackendRegistry::new();
        let mut counters = Vec::new();
        for b in backends {
            let fails = failing.contains(b);
            let counting = Arc::new(Counting {
                name: b.to_string(),
                calls: AtomicUsize::new(0),
                widest: AtomicUsize::new(0),
                listings: crate::core::installed::InstalledListings::new(),
                // A backend that is meant to fail does it at once; the stall exists to keep the
                // *other* one inside its command while that happens.
                stall: if fails { Duration::ZERO } else { stall },
                fails,
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
        let journal = Arc::new(Mutex::new(
            Journal::at(tmp.path().join("journal.jsonl")).unwrap(),
        ));
        let diagnostics = crate::app::diagnostics::FailureDiagnosticEngine::init(&config).await;
        let mut tx_config = TransactionConfig::patient();
        // Rollback off: it would ask each backend what was there before, which is not what
        // these tests are measuring.
        tx_config.auto_rollback = false;
        tx_config.total_timeout = total_timeout;
        let tx = Transaction::with_config(
            graph,
            Arc::new(registry),
            journal.clone(),
            Arc::new(diagnostics),
            Arc::new(config),
            tx_config,
        )
        // These tests hand the executor a graph directly, which is precisely the case the
        // `reaped` refusal exists to catch in production — a plan that reached the engine
        // without passing the guard. What they are measuring is how the executor *batches*,
        // and threading a real `Config` and `BackendRegistry` through a guard to measure that
        // would prove nothing about either.
        .guarded_by(crate::app::sync::guard::Reaped::for_reason(
            crate::app::sync::guard::GuardScope::Sync,
            "a unit test measuring how the executor batches, not whether the guard ran",
        ));
        Harness {
            tx,
            counters,
            journal,
            _tmp: tmp,
        }
    }

    /// A run that outlives its own deadline closes the entries it opened.
    ///
    /// The timeout drops the whole `JoinSet`, so the batches are killed inside the manager's
    /// command and reach neither of the calls that close an entry. `close_stranded` is what
    /// stands between that and a log full of operations that read as a crash.
    #[tokio::test]
    async fn a_run_that_times_out_leaves_no_entry_open() {
        let mut graph = StableDiGraph::new();
        for name in ["jq", "ripgrep", "fd", "bat"] {
            graph.add_node(GraphAction::Install(spec("apt", name)));
        }
        // Each call takes far longer than the whole run is allowed, so the deadline lands
        // while work is outstanding — which is the only way to reach the cancellation arm.
        let mut h = harness_with(
            graph,
            &["apt"],
            Duration::from_secs(30),
            Duration::from_millis(150),
            &[],
        )
        .await;

        let outcome = h.tx.execute().await;
        assert!(outcome.is_err(), "the deadline must end the run");

        let open = h.journal.lock().await.interrupted_actions();
        assert!(
            open.is_empty(),
            "a timeout is not a crash, and every entry it opened must be closed — {} left \
             open: {:?}",
            open.len(),
            open.iter().map(|e| e.action.key()).collect::<Vec<_>>()
        );
    }

    /// The macOS nightly's own shape: one manager fails while another is mid-command.
    ///
    /// This is the path that actually stranded 22 operations. `continue_on_error` is off, so
    /// the first failure ends the run — and every batch still inside a manager's command is
    /// killed where it stands, having opened its WAL entries and closed none.
    #[tokio::test]
    async fn a_run_stopped_by_one_managers_failure_leaves_no_entry_open() {
        let mut graph = StableDiGraph::new();
        // The one that fails, and three the slow manager is still working through when it does.
        graph.add_node(GraphAction::Install(spec("gem", "logger")));
        for name in ["jq", "ripgrep", "fd"] {
            graph.add_node(GraphAction::Install(spec("apt", name)));
        }
        let mut h = harness_with(
            graph,
            &["apt", "gem"],
            Duration::from_secs(30),
            Duration::from_secs(3600),
            &["gem"],
        )
        .await;

        assert!(h.tx.execute().await.is_err(), "the failing manager ends it");

        let open = h.journal.lock().await.interrupted_actions();
        assert!(
            open.is_empty(),
            "Shall stopped these itself and knows it did — leaving them open makes `heal` hunt \
             a crash that never happened. {} left open: {:?}",
            open.len(),
            open.iter().map(|e| e.action.key()).collect::<Vec<_>>()
        );
    }

    /// And the entry a *previous* run left open is not touched.
    ///
    /// **This is the half that makes the fix safe rather than merely quiet.** An entry open
    /// from an earlier run is the record that a process died holding it, and it is the only
    /// thing that tells `heal` to look. A close-everything-still-open would have erased
    /// exactly the state this log exists to keep, and the harness assertion it was written to
    /// satisfy would have gone green either way.
    #[tokio::test]
    async fn an_earlier_runs_open_entry_survives_this_runs_failure() {
        let mut graph = StableDiGraph::new();
        graph.add_node(GraphAction::Install(spec("gem", "logger")));
        for name in ["jq", "ripgrep"] {
            graph.add_node(GraphAction::Install(spec("apt", name)));
        }
        let mut h = harness_with(
            graph,
            &["apt", "gem"],
            Duration::from_secs(30),
            Duration::from_secs(3600),
            &["gem"],
        )
        .await;

        // A crash, before this run starts: opened and never closed by anybody.
        let ghost = h
            .journal
            .lock()
            .await
            .record_start(JournalAction::Install(spec("apt", "left-by-a-crash")))
            .unwrap();

        assert!(h.tx.execute().await.is_err());

        let open = h.journal.lock().await.interrupted_actions();
        let ids: Vec<&str> = open.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![ghost.as_str()],
            "the earlier run's entry, and only it, must still be open"
        );
    }

    /// **`U41`, both halves, as one question.** Rollback does not undo work that moved the
    /// machine toward the declared state — an install that succeeded of something still
    /// declared, or a removal that succeeded of something still undeclared. The install arm had
    /// this rule and the removal arm did not, and nothing in the register said the pair had
    /// come apart.
    #[tokio::test]
    async fn one_rollback_rule_answers_both_directions() {
        let mut graph = StableDiGraph::new();
        graph.add_node(GraphAction::Install(spec("apt", "jq")));
        let h = harness(graph, &["apt"]).await;

        // Nothing to reconcile against: no answer, and both arms fall back to compensating.
        assert_eq!(h.tx.plan_intends_present("apt", "jq"), None);
        assert_eq!(h.tx.plan_intends_present("apt", "vim"), None);

        let declared: std::collections::HashSet<String> =
            ["apt:jq".to_string()].into_iter().collect();
        let tx = h.tx.reconciling(Arc::new(declared));

        // The install arm: `jq` installed cleanly and is still declared, so the removal that
        // would compensate it is skipped.
        assert_eq!(
            tx.plan_intends_present("apt", "jq"),
            Some(true),
            "an install of something still declared is the goal reached early"
        );
        // The removal arm: nothing declares `vim`, which is why it was removed, and that fact
        // has not changed — so the reinstate that would compensate it is skipped.
        assert_eq!(
            tx.plan_intends_present("apt", "vim"),
            Some(false),
            "a removal of something still undeclared must not be put back"
        );
        // Keyed by backend and name together, so `apt:jq` does not answer for `cargo:jq`.
        assert_eq!(tx.plan_intends_present("cargo", "jq"), Some(false));
    }

    /// Which runs are reconciliations, asserted as the two exceptions rather than as ten rules.
    #[test]
    fn only_a_reconciling_run_may_leave_a_removal_in_place() {
        use crate::app::sync::guard::GuardScope as S;
        for scope in [
            S::Apply,
            S::RemoveOrphans,
            S::PurgeUndeclared,
            S::Sync,
            S::Watch,
            S::Upgrade,
            S::Canary,
            S::ShellExit,
            S::ExpirySweep,
            S::Heal,
        ] {
            assert!(
                scope.reconciles(),
                "{} removes what nothing declares",
                scope.as_str()
            );
        }
        // A rebuild's removal phase is the first half of a reinstall of DECLARED packages,
        // split into two transactions so the Remove and the Install cannot race in one graph.
        // Leaving one of those removals in place is a machine missing software it declares.
        assert!(!S::Rebuild.reconciles());
        // And an uninstall was typed by a person, not derived from a manifest.
        assert!(!S::Remove.reconciles());
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

    /// The engine's own shape, measured on plans whose right answer is arithmetic.
    ///
    /// This is the `Mutating` half of the latency gate, and it is here rather than in
    /// `latency.rs` because the two numbers it compares are the engine's: one is the graph's
    /// longest chain, the other is how many passes the loop actually had work to hand out.
    /// `latency.rs` owns the rule; nothing owned the measurement, which is why the class read as
    /// exempt for as long as it did.
    ///
    /// A wide plan must take one wave however many packages are in it — that is the assertion a
    /// change awaiting the batch in flight before dispatching more would break, while leaving
    /// every package installed and every other test green. Watched failing against exactly that
    /// change: six packages, depth one, six waves.
    #[tokio::test]
    async fn the_engine_reports_the_shape_it_actually_ran_in() {
        // Wide: nothing depends on anything, so one round is the only correct answer.
        let mut wide = StableDiGraph::new();
        for i in 0..6 {
            wide.add_node(GraphAction::Install(spec("apt", &format!("pkg{}", i))));
        }
        let mut h = harness(wide, &["apt"]).await;
        h.tx.execute_with_telemetry().await.unwrap();
        assert_eq!(
            h.tx.last_scheduling,
            Some(Scheduling {
                packages: 6,
                depth: 1,
                waves: 1
            }),
            "six independent packages took more than one wave to hand out"
        );

        // A chain of three. Serial is the correct schedule here, and the rule must not report
        // it — which is the case a threshold read off a host gets wrong.
        let mut chain = StableDiGraph::new();
        let a = chain.add_node(GraphAction::Install(spec("apt", "libfoo")));
        let b = chain.add_node(GraphAction::Install(spec("apt", "foo-tool")));
        let c = chain.add_node(GraphAction::Install(spec("apt", "foo-plugin")));
        chain.add_edge(a, b, ());
        chain.add_edge(b, c, ());
        let mut h = harness(chain, &["apt"]).await;
        h.tx.execute_with_telemetry().await.unwrap();
        let ran =
            h.tx.last_scheduling
                .expect("a completed run records its shape");
        assert_eq!(ran.depth, 3, "a chain of three has three levels");
        assert_eq!(ran.waves, 3, "and goes idle once per level");
        assert!(
            crate::core::latency::scheduling_violation(ran.packages, ran.depth, ran.waves)
                .is_none(),
            "a chain executed one at a time is the plan's shape, not a regression"
        );

        // A diamond: one, then two together, then one. Four packages, three levels.
        let mut diamond = StableDiGraph::new();
        let root = diamond.add_node(GraphAction::Install(spec("apt", "base")));
        let left = diamond.add_node(GraphAction::Install(spec("apt", "left")));
        let right = diamond.add_node(GraphAction::Install(spec("apt", "right")));
        let top = diamond.add_node(GraphAction::Install(spec("apt", "top")));
        diamond.add_edge(root, left, ());
        diamond.add_edge(root, right, ());
        diamond.add_edge(left, top, ());
        diamond.add_edge(right, top, ());
        let mut h = harness(diamond, &["apt"]).await;
        h.tx.execute_with_telemetry().await.unwrap();
        let ran =
            h.tx.last_scheduling
                .expect("a completed run records its shape");
        assert_eq!(
            (ran.packages, ran.depth),
            (4, 3),
            "four packages across three levels"
        );
        assert!(
            crate::core::latency::scheduling_violation(ran.packages, ran.depth, ran.waves)
                .is_none(),
            "a diamond run in {} waves against a depth of 3",
            ran.waves
        );

        // Two independent chains. **This is the case that killed the first version of the
        // rule**, which counted dispatches: `a` and `b` start together, whichever finishes
        // first hands out its child while the other pair is still running, and the pass count
        // reaches three against a depth of two — a violation reported against a scheduler doing
        // precisely what it should. Counting idle restarts instead, the engine never runs dry
        // here at all.
        let mut chains = StableDiGraph::new();
        let a = chains.add_node(GraphAction::Install(spec("apt", "a")));
        let b = chains.add_node(GraphAction::Install(spec("apt", "b")));
        let a2 = chains.add_node(GraphAction::Install(spec("apt", "a2")));
        let b2 = chains.add_node(GraphAction::Install(spec("apt", "b2")));
        chains.add_edge(a, a2, ());
        chains.add_edge(b, b2, ());
        let mut h = harness(chains, &["apt"]).await;
        h.tx.execute_with_telemetry().await.unwrap();
        let ran =
            h.tx.last_scheduling
                .expect("a completed run records its shape");
        assert_eq!((ran.packages, ran.depth), (4, 2));
        assert!(
            ran.waves <= ran.depth,
            "two independent chains reported {} wave(s) against a depth of {}",
            ran.waves,
            ran.depth
        );
    }

    /// Where the byte bound puts the split, to the name.
    ///
    /// **`batches` had no direct test at all.** Every batching test above drives it through the
    /// async harness and asks how many *commands* ran; none asks where the split fell, and the
    /// difference is the whole of the arithmetic. Six mutants lived in five lines of it — both
    /// `name.len() + 1` costs, the `bytes + cost` sum, the `bytes += cost` accumulation, and the
    /// `>` that compares them — and a run that batches 61 packages into 2 commands does that
    /// under every one of them.
    ///
    /// The lengths are picked so each mutation moves the answer, which is the only reason this
    /// is a table of magic numbers rather than one round case. `MAX_BATCH_BYTES` is 6000 and a
    /// name of length L costs L + 1:
    ///
    ///   - L = 99 (cost 100) makes 60 names come to exactly 6000, which is the one place `>`
    ///     and `>=` disagree.
    ///   - L = 100 (cost 101) fits 59 names where cost 100 or 99 would fit 60, which is where
    ///     dropping the `+ 1` shows up.
    #[test]
    fn the_byte_bound_splits_exactly_where_the_cost_says() {
        /// A name of exactly `len` characters, distinct per index.
        fn padded(len: usize, i: usize) -> String {
            format!("{:0width$}", i, width = len)
        }

        fn sizes(actions: Vec<GraphAction>, max_batch: usize) -> Vec<usize> {
            let mut graph = StableDiGraph::new();
            let ready: Vec<_> = actions.into_iter().map(|a| graph.add_node(a)).collect();
            Transaction::batches(&graph, ready, max_batch)
                .iter()
                .map(|b| b.len())
                .collect()
        }

        let installs = |len: usize, n: usize| {
            (0..n)
                .map(|i| GraphAction::Install(spec("apt", &padded(len, i))))
                .collect::<Vec<_>>()
        };
        let removals = |len: usize, n: usize| {
            (0..n)
                .map(|i| GraphAction::Remove {
                    name: padded(len, i),
                    backend: "apt".into(),
                })
                .collect::<Vec<_>>()
        };

        // Exactly on the bound: 60 × 100 bytes is 6000, and 6000 is not *over* 6000.
        assert_eq!(
            sizes(installs(99, 60), 10_000),
            vec![60],
            "sixty names costing 6000 bytes sit exactly on the bound, which is not over it"
        );
        // One more, and the bound is genuinely crossed.
        assert_eq!(
            sizes(installs(99, 61), 10_000),
            vec![60, 1],
            "the sixty-first name is the one that does not fit"
        );

        // One byte per name wider, and the batch is one name shorter — which is what says the
        // separator is being counted at all.
        assert_eq!(
            sizes(installs(100, 60), 10_000),
            vec![59, 1],
            "at 101 bytes a name only fifty-nine fit under 6000"
        );
        // A removal's name costs the same as an install's; it is the other arm of the same
        // match, and it was the other surviving mutant.
        assert_eq!(
            sizes(removals(100, 60), 10_000),
            vec![59, 1],
            "a removal's name is measured the same way an install's is"
        );

        // And the count cap, which is the caller's and not the byte bound's. Short names, so
        // nothing here is near 6000 bytes and only `max_batch` can be doing the splitting.
        assert_eq!(sizes(installs(2, 10), 3), vec![3, 3, 3, 1]);
        assert_eq!(
            sizes(installs(2, 10), 0),
            vec![1; 10],
            "a cap of nought is read as one, not as no cap at all"
        );
    }
}
