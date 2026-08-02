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
    pub fn patient() -> Self {
        Self {
            max_concurrent: 4,
            node_timeout: Duration::from_secs(300),
            total_timeout: Duration::from_secs(3600),
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            auto_rollback: true,
            purge: false,
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

            let ready_nodes: Vec<NodeIndex> = self
                .graph
                .node_indices()
                .filter(|&idx| {
                    !self.completed_indices.contains(&idx)
                        && !in_progress.contains(&idx)
                        && self
                            .graph
                            .neighbors_directed(idx, Direction::Incoming)
                            .all(|dep| self.completed_indices.contains(&dep))
                })
                .collect();

            for idx in ready_nodes {
                let permit = match semaphore.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(e) => return Err(Error::Transaction(format!("Semaphore failure: {}", e))),
                };

                in_progress.insert(idx);

                let action = self.graph[idx].clone();
                let registry = self.registry.clone();
                let journal = self.journal.clone();
                let cancel_token = self.cancellation_token.clone();
                let config = self.config.clone();
                let hooks = self.hooks.clone();

                worker_pool.spawn(async move {
                    let _permit_holder = permit;
                    Self::execute_node_with_retry(
                        action,
                        registry,
                        journal,
                        config,
                        hooks,
                        cancel_token,
                        idx,
                    )
                    .await
                });
            }

            if let Some(finished_task) = worker_pool.join_next().await {
                let task_data = finished_task
                    .map_err(|e| Error::Transaction(format!("Worker Panic: {}", e)))?;

                // Must be read before `task_data` is moved into `telemetry_results` below.
                let is_failure = task_data.result.is_err();

                if !is_failure {
                    trace!(
                        "Node {}:{} succeeded.",
                        task_data.backend_name,
                        task_data.package_name
                    );
                    in_progress.remove(&task_data.node_index);
                    self.completed_indices.insert(task_data.node_index);
                    self.history
                        .push((task_data.node_index, task_data.prior.clone()));
                    telemetry_results.push(task_data);
                } else {
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

                    self.diagnostics
                        .print_suggestions(&error_msg, &task_data.backend_name);

                    let final_err = task_data.result.clone().err().unwrap();
                    telemetry_results.push(task_data);

                    if self.config.auto_rollback {
                        info!("rolling back");
                        worker_pool.abort_all();
                        if let Err(e) = self.rollback().await {
                            error!("{}", e);
                        }
                    }
                    return Err(final_err);
                }
            } else if in_progress.is_empty() && self.completed_indices.len() < total_nodes {
                return Err(Error::Transaction(
                    "DAG Logic Stall: Cycle detected in closure.".into(),
                ));
            }
        }
        Ok(telemetry_results)
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_node_with_retry(
        action: GraphAction,
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        config: TransactionConfig,
        hooks: Option<Arc<LuaHooks>>,
        cancel_token: CancellationToken,
        node_index: NodeIndex,
    ) -> TaskResult {
        let is_install = matches!(action, GraphAction::Install(_));
        let (p_name, b_name, j_action) = match &action {
            GraphAction::Install(s) => (
                s.name.clone(),
                s.backend.clone(),
                JournalAction::Install(s.clone()),
            ),
            GraphAction::Remove { name, backend } => (
                name.clone(),
                backend.clone(),
                JournalAction::Remove {
                    name: name.clone(),
                    backend: backend.clone(),
                },
            ),
        };

        let start_time_utc = chrono::Utc::now();
        let start_instant = Instant::now();

        // The grammar checks what a *file* declares. A removal target comes from
        // `registry.json`, which apt's post-invoke hook also writes, so it has not been
        // through the grammar at all.
        if let Err(e) = crate::core::Validator::validate_package_name_for(&p_name, &b_name) {
            return TaskResult {
                node_index,
                backend_name: b_name,
                package_name: p_name,
                retries: 0,
                duration: Duration::ZERO,
                bytes_downloaded: 0,
                start_time: start_time_utc,
                prior: Prior::Unknown,
                result: Err(e),
            };
        }

        let backend_cap = match registry.get(&b_name) {
            Some(cap) => cap,
            None => {
                return TaskResult {
                    node_index,
                    backend_name: b_name.clone(),
                    package_name: p_name,
                    retries: 0,
                    duration: Duration::ZERO,
                    bytes_downloaded: 0,
                    start_time: start_time_utc,
                    prior: Prior::Unknown,
                    result: Err(Error::BackendNotFound(b_name)),
                }
            }
        };

        // Read before anything is done to it. Rollback compensates by putting this back, and
        // "what was there before" is unknowable once the node has run. Skipped entirely when
        // there is no rollback to feed — it costs a query per node, and a query on this
        // backend lists everything it has.
        let prior = if config.auto_rollback {
            Self::prior_state(&backend_cap, &p_name).await
        } else {
            Prior::Unknown
        };

        let journal_id = {
            let mut j = journal.lock().await;
            match j.record_start(j_action.clone()) {
                Ok(id) => id,
                Err(e) => {
                    return TaskResult {
                        node_index,
                        backend_name: b_name,
                        package_name: p_name,
                        retries: 0,
                        duration: Duration::ZERO,
                        bytes_downloaded: 0,
                        start_time: start_time_utc,
                        prior: prior.clone(),
                        result: Err(Error::Journal(format!("WAL error: {}", e))),
                    }
                }
            }
        };

        // Fire the per-package `before_install` hook once, before any install attempt.
        // A failing pre-hook aborts the node: the package is intentionally not installed
        // because its declared prerequisites were not met.
        if is_install {
            if let Some(h) = &hooks {
                if let Err(e) = h.run_hook("before_install", &p_name).await {
                    let msg = format!("before_install hook failed: {}", e);
                    let mut j = journal.lock().await;
                    let _ = j.record_failure(&journal_id, &msg);
                    return TaskResult {
                        node_index,
                        backend_name: b_name,
                        package_name: p_name,
                        retries: 0,
                        duration: start_instant.elapsed(),
                        bytes_downloaded: 0,
                        start_time: start_time_utc,
                        prior: prior.clone(),
                        result: Err(Error::Transaction(msg)),
                    };
                }
            }
        }

        let mut attempt = 0;
        let mut last_error = None;

        while attempt <= config.max_retries {
            attempt += 1;
            if cancel_token.is_cancelled() {
                return TaskResult {
                    node_index,
                    backend_name: b_name,
                    package_name: p_name,
                    retries: attempt - 1,
                    duration: start_instant.elapsed(),
                    bytes_downloaded: 0,
                    start_time: start_time_utc,
                    prior: prior.clone(),
                    result: Err(Error::Cancelled),
                };
            }

            if attempt > 1 {
                let backoff = std::cmp::min(
                    config.initial_backoff * (1 << (attempt - 2)),
                    config.max_backoff,
                );
                tokio::time::sleep(backoff).await;
            }

            let result = tokio::time::timeout(config.node_timeout, async {
                match &action {
                    GraphAction::Install(spec) => {
                        if let Some(handler) = backend_cap.as_installable() {
                            handler
                                .install(std::slice::from_ref(spec), backend_cap.sudo_for_write())
                                .await?;
                            // No post-install `info()`. On a `generic` backend that call is a
                            // full listing of every package the manager has — `choco list`,
                            // `winget list` — run once per package just installed, and its only
                            // consumer was a `download_size` property that **no backend in this
                            // tree has ever produced**. The docs measured an `install
                            // choco:bat` at 399s of which 18.75s was the install; this was the
                            // rest of it. A backend that starts reporting a download size
                            // reports it from its own install output, not from a re-listing.
                            Ok(0u64)
                        } else {
                            Err(Error::Transaction(format!(
                                "Backend '{}' is not installable.",
                                b_name
                            )))
                        }
                    }
                    GraphAction::Remove { name, .. } => {
                        if let Some(handler) = backend_cap.as_installable() {
                            let one = std::slice::from_ref(name);
                            let sudo = backend_cap.sudo_for_write();
                            if config.purge && handler.supports_purge() {
                                handler.purge(one, sudo).await?;
                            } else {
                                handler.remove(one, sudo).await?;
                            }
                            Ok(0u64)
                        } else {
                            Err(Error::Transaction(format!(
                                "Backend '{}' is not removable.",
                                b_name
                            )))
                        }
                    }
                }
            })
            .await;

            match result {
                Ok(Ok(bytes)) => {
                    {
                        let mut j = journal.lock().await;
                        let _ = j.record_success(&journal_id);
                    }
                    // Fire `after_install` once the package is physically installed. A
                    // post-hook failure is logged but does not undo a successful install
                    // (rolling back a healthy package over a cosmetic hook error would be
                    // more surprising than the failure itself).
                    if is_install {
                        if let Some(h) = &hooks {
                            if let Err(e) = h.run_hook("after_install", &p_name).await {
                                warn!("after_install hook for '{}' failed: {}", p_name, e);
                            }
                        }
                    }
                    return TaskResult {
                        node_index,
                        backend_name: b_name,
                        package_name: p_name,
                        retries: attempt - 1,
                        duration: start_instant.elapsed(),
                        bytes_downloaded: bytes,
                        start_time: start_time_utc,
                        prior: prior.clone(),
                        result: Ok(()),
                    };
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
                    last_error = Some(Error::Transaction("Node timed out.".into()));
                }
            }
        }

        let final_err = falsify_transience(
            last_error.unwrap_or(Error::Transaction("Unknown error".into())),
            attempt,
        );
        let mut j = journal.lock().await;
        let _ = j.record_failure(&journal_id, &format!("{}", final_err));

        TaskResult {
            node_index,
            backend_name: b_name,
            package_name: p_name,
            retries: attempt - 1,
            duration: start_instant.elapsed(),
            bytes_downloaded: 0,
            start_time: start_time_utc,
            prior: prior.clone(),
            result: Err(final_err),
        }
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
        let os_essential =
            crate::app::sync::guard::essential_names(&self.registry, &backends).await;

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
