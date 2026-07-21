use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::{LuaHooks, MetricsCollector, ShimManager};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{
    CommandExecutor, Error, GraphAction, Journal, PackageSpec, Result, SnapshotManager,
    StateRegistry, Transaction, TransactionConfig,
};
use crate::utils::progress::ProgressReporter;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing::{debug, error, info, instrument, warn};

pub mod guard;
pub mod planner;
pub mod resolver;
pub mod saved_plan;

pub use self::planner::{ChangePlanner, Scope, SyncChanges};

/// K15: a rebuild's two transactions run through this engine like any other sync, so the
/// summary has to be told which run it is narrating or it reports a rebuild's removals as
/// removals.
fn narration_for(scope: guard::GuardScope) -> crate::app::metrics::Narration {
    match scope {
        guard::GuardScope::Rebuild => crate::app::metrics::Narration::Rebuild,
        _ => crate::app::metrics::Narration::Change,
    }
}

pub use self::resolver::StateResolver;
pub use self::saved_plan::{SavedPlan, PLAN_SCHEMA};

#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    async fn resolve_desired_state(
        &self,
    ) -> Result<std::collections::HashMap<String, Vec<PackageSpec>>>;
}

#[async_trait::async_trait]
pub trait Planner: Send + Sync {
    async fn plan(
        &self,
        desired: &std::collections::HashMap<String, Vec<PackageSpec>>,
        scope: Option<Scope>,
    ) -> Result<SyncChanges>;
}

pub struct SyncEngine<'a> {
    pub config: &'a Config,
    pub registry: Arc<BackendRegistry>,
    pub executor: CommandExecutor,
    pub metrics: MetricsCollector,
    pub progress: Arc<dyn ProgressReporter>,
    pub hooks: Arc<LuaHooks>,
    pub snapshot_manager: Arc<SnapshotManager>,
    pub journal: Arc<Mutex<Journal>>,
    pub state: Arc<Mutex<StateRegistry>>,
    pub diagnostics: Arc<FailureDiagnosticEngine>,
}

impl<'a> SyncEngine<'a> {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        config: &'a Config,
        registry: Arc<BackendRegistry>,
        executor: CommandExecutor,
        metrics: MetricsCollector,
        progress: Arc<dyn ProgressReporter>,
        hooks: Arc<LuaHooks>,
        snapshot_manager: Arc<SnapshotManager>,
        journal: Arc<Mutex<Journal>>,
        state: Arc<Mutex<StateRegistry>>,
        diagnostics: Arc<FailureDiagnosticEngine>,
    ) -> Self {
        Self {
            config,
            registry,
            executor,
            metrics,
            progress,
            hooks,
            snapshot_manager,
            journal,
            state,
            diagnostics,
        }
    }

    /// `scope` names the command that asked, so the removal guard can be enforced here —
    /// at the one point every drift-removal path funnels through — rather than at each
    /// caller, where it only takes one forgotten site to purge a system.
    #[instrument(skip(self, changes))]
    pub async fn sync(&self, changes: SyncChanges, scope: guard::GuardScope) -> Result<()> {
        let _heartbeat = self.executor.start_sudo_keepalive().await;

        // The supply-chain gate (II.12), before any hook runs and before anything is touched:
        // a hook whose script is new or changed since you approved it stops the sync. Note the
        // `?` — the `run_before_sync` below swallows its own errors, so the authoritative stop
        // has to live here, where it propagates.
        self.hooks.verify_all_approved()?;
        let _ = self.hooks.run_before_sync().await;

        if changes.is_empty() {
            info!("already up to date");
            return Ok(());
        }

        // Before the snapshot and before any package is touched: refuse a removal set
        // that is oversized or takes something the system needs.
        guard::enforce(
            self.config,
            &self.registry,
            &guard::removal_pairs(&changes),
            scope,
        )
        .await?;

        // The install-side ceiling (II.10): a mis-globbed manifest schedules a flood of
        // installs, and the count is the fact that explains it. Off by default; when set,
        // only `--allow-mass-install` clears it.
        guard::enforce_installs(self.config, changes.total_install(), scope).await?;

        // The pre-sync snapshot is a safety NET, not a precondition: a Windows System
        // Restore checkpoint needs admin (and System Restore enabled), and btrfs/timeshift
        // may be unavailable — none of which should abort a package sync. Policies that
        // TRULY require a snapshot gate on `has_provider()` upstream; here we warn and
        // proceed so a missing restore point never blocks the actual work.
        if let Err(e) = self.snapshot_manager.auto_snapshot(crate::core::snapshot::SnapshotLabel::PreSync).await {
            warn!(
                "pre-sync safety snapshot unavailable ({}); proceeding without a restore point.",
                e
            );
        }

        let result = {
            let mut state_guard = self.state.lock().await;
            self.execute_transaction(&changes, &mut state_guard).await
        };

        if result.is_ok() {
            debug!("Finalizing transaction state and persistence.");

            let state_to_save = self.state.lock().await.clone();
            tokio::task::spawn_blocking(move || state_to_save.save())
                .await
                .map_err(|e| {
                    Error::Other(format!("Kernel panic during state persistence: {}", e))
                })??;

            // Scope the state guard to shim reconciliation ONLY. Steps below (the health
            // probes and snapshot retention) re-acquire `self.state` internally, and
            // `tokio::sync::Mutex` is not re-entrant — holding the guard across those calls
            // self-deadlocks the whole sync/prune/upgrade after the transaction has succeeded.
            {
                let final_state = self.state.lock().await;
                self.reconcile_all_shims(&final_state).await?;
            }

            let _ = self.hooks.run_after_sync().await;
            if self.config.quiet {
                self.metrics.print_summary_quiet();
            } else {
                self.metrics.print_summary(narration_for(scope));
            }

            // Post-apply health probes: verify any freshly-installed package that declared
            // `@check=…` actually works, so a green install that left a broken service is
            // surfaced immediately (with the pre-sync snapshot available to revert).
            self.run_health_probes(&changes).await;

            // The manifest history is git now (the generation format was deleted): the commit
            // that records this change is made by `git_autocommit` in `perform_maintenance`,
            // after a successful sync. Snapshot retention still runs here.
            self.prune_snapshots_after_sync().await;

            let mut j = self.journal.lock().await;
            let _ = j.cleanup();
        }

        result
    }

    /// Run the `@check=…` post-install probe for every freshly-installed package that declared
    /// one. Probes are advisory: a failure is reported loudly (and points at the snapshot for
    /// recovery) but does not itself undo the commit — the safety snapshot already exists, and
    /// `--canary`/`rollback` are the explicit auto-revert paths.
    async fn run_health_probes(&self, changes: &SyncChanges) {
        let mut probes: Vec<(String, String)> = Vec::new();
        for w in changes.graph.node_weights() {
            if let GraphAction::Install(spec) = w {
                if let Some(check) = spec.options.get("check") {
                    probes.push((format!("{}:{}", spec.backend, spec.name), check.clone()));
                }
            }
        }
        if probes.is_empty() {
            return;
        }
        info!(
            "running {} post-install health probe(s)...",
            probes.len()
        );
        let mut failed = Vec::new();
        for (pkg, check) in &probes {
            if Self::probe_ok(check).await {
                info!("  probe OK   {} ({})", pkg, check);
            } else {
                warn!("  probe FAIL {} ({})", pkg, check);
                failed.push(pkg.clone());
            }
        }
        if !failed.is_empty() {
            warn!(
                "{} package(s) failed their @check probe: {}.",
                failed.len(),
                failed.join(", ")
            );
            if self.snapshot_manager.has_provider() {
                warn!("A pre-sync snapshot exists — `linix undo` / `linix rollback` can revert if needed.");
            }
        }
    }

    /// Evaluate one probe spec. `port:<n>` succeeds if a TCP connection to localhost:n opens;
    /// `cmd:<shell>` (or a bare string) succeeds if the shell command exits 0.
    async fn probe_ok(check: &str) -> bool {
        if let Some(port) = check.strip_prefix("port:") {
            return match port.trim().parse::<u16>() {
                Ok(p) => tokio::net::TcpStream::connect(("127.0.0.1", p))
                    .await
                    .is_ok(),
                Err(_) => false,
            };
        }
        let cmd = check.strip_prefix("cmd:").unwrap_or(check);
        crate::app::bisect::run_test(cmd).await
    }

    /// Apply snapshot retention after a successful sync. (The manifest history is git now —
    /// the commit is `git_autocommit`'s job in `perform_maintenance`; there is no generation
    /// capture here anymore.) Non-fatal: a retention hiccup must never fail a good sync.
    async fn prune_snapshots_after_sync(&self) {
        if self.config.dry_run {
            return;
        }
        let ts = chrono::Utc::now();
        match self
            .snapshot_manager
            .prune_with_policy(&self.config.snapshot_retention(), ts, false)
            .await
        {
            Ok(r) if !r.is_empty() => debug!("pruned {} snapshot(s).", r.len()),
            Err(e) => warn!("snapshot retention prune failed: {}", e),
            _ => {}
        }
    }

    async fn execute_transaction(
        &self,
        changes: &SyncChanges,
        state: &mut StateRegistry,
    ) -> Result<()> {
        // `max_parallel` is the user's knob for this engine, so it must be read here and
        // not left at the `patient()` default — a hardcoded default silently narrows the
        // setting's reach to `search` alone. Floor at 1: zero would stall the transaction.
        let mut tx_config = TransactionConfig::patient();
        tx_config.max_concurrent = self.config.max_parallel.max(1);

        let tx = Transaction::with_config(
            changes.graph.clone(),
            self.registry.clone(),
            self.journal.clone(),
            self.diagnostics.clone(),
            tx_config,
        );
        // Per-package `before_install`/`after_install` hooks fire inside the engine,
        // at the moment each package installs (see Transaction::with_hooks).
        let mut tx = tx.with_hooks(self.hooks.clone());

        let pb = self
            .progress
            .spinner("Applying parallel system modifications...");
        let results = tx.execute_with_telemetry().await?;
        pb.finish();

        let session_active = state.active_session_id.is_some();

        for res in results {
            self.metrics.record_operation(
                &res.package_name,
                &res.backend_name,
                res.start_time,
                res.result.is_ok(),
                res.result.err().map(|e| e.to_string()),
                res.attempt,
                res.bytes_downloaded,
            );
        }

        for idx in changes.graph.node_indices() {
            match &changes.graph[idx] {
                GraphAction::Install(spec) => {
                    let source = spec.options.get("__source").cloned();
                    state.add(
                        &spec.backend,
                        &spec.name,
                        None,
                        spec.options.clone(),
                        source,
                        session_active,
                    );

                    // S18: auto-locking used to splice `@sha256=…` into the line you wrote
                    // — II.16 says LiNix must not rewrite your files, and a checksum is a
                    // generated fact, which II.6 keeps in `locks/`. The recording of an
                    // artifact hash is a real supply-chain feature (II.12); it lands in
                    // `locks/<backend>.toml` in Phase 4, not in your module.
                }
                GraphAction::Remove { name, backend } => {
                    state.remove(backend, name);
                }
            }
        }

        self.metrics.record_install(changes.total_install() as u64);
        self.metrics.record_remove(changes.total_remove() as u64);
        Ok(())
    }

    async fn reconcile_all_shims(&self, state: &StateRegistry) -> Result<()> {
        let shim_mgr = Arc::new(ShimManager::new().await?);
        let mut worker_set = JoinSet::new();

        debug!(
            "auditing shims for {} packages",
            state.packages.len()
        );

        for pkg in &state.packages {
            let mgr = shim_mgr.clone();
            let pkg_name = pkg.name.clone();
            let needs_shim = pkg.options.get("sandbox") == Some(&"true".to_string())
                || pkg.options.get("shim") == Some(&"true".to_string());

            worker_set.spawn(async move { mgr.reconcile_shims(&pkg_name, needs_shim).await });
        }

        while let Some(res) = worker_set.join_next().await {
            if let Err(e) = res {
                error!("Shim worker task panicked: {}", e);
            } else if let Ok(Err(e)) = res {
                warn!("Non-critical shim reconciliation failure: {}", e);
            }
        }

        Ok(())
    }

    pub async fn heal(&self) -> Result<()> {
        let incomplete_actions = {
            let j = self.journal.lock().await;
            j.get_incomplete_actions()
        };

        if incomplete_actions.is_empty() {
            debug!("nothing to heal");
            return Ok(());
        }

        // S6: healing is automatic (a half-finished transaction is drift, and removing drift
        // is sync's job — asking permission would ask permission to do sync's own job). But
        // automatic is not silent: a recovery nobody sees is exactly the class of bug this
        // whole document is about (P3). Report every action taken, by name, and summarize.
        info!(
            "recovering {} interrupted operation(s) from a previous run.",
            incomplete_actions.len()
        );
        let mut recovered: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        // Packages whose interrupted removal the guard refused: kept, not removed (owner
        // decision), and the entry resolved so heal completes rather than sticking.
        let mut kept: Vec<String> = Vec::new();
        for entry in incomplete_actions {
            let (backend, package, is_install) = match &entry.action {
                crate::core::journal::JournalAction::Install(spec) => {
                    (spec.backend.clone(), spec.name.clone(), true)
                }
                crate::core::journal::JournalAction::Remove { name, backend } => {
                    (backend.clone(), name.clone(), false)
                }
            };
            let key = format!("{}:{}", backend, package);

            if let Some(backend_cap) = self.registry.get(&backend) {
                if let Some(handler) = backend_cap.as_installable() {
                    let sudo = backend_cap.sudo_for_write();

                    // Owner decision: completing an interrupted *removal* routes through the
                    // guard, so a protected package is never removed even during recovery. On
                    // refusal we KEEP the package and treat the entry as resolved — recovery
                    // completes, protection holds, and heal never gets stuck retrying a removal
                    // it will always refuse. (The remove-before-reinstall of the install path is
                    // not a removal of intent — the same package is reinstalled next — so it is
                    // not guarded here.)
                    if !is_install {
                        let removal = [(backend.clone(), package.clone())];
                        if let Err(objection) =
                            guard::enforce(self.config, &self.registry, &removal, guard::GuardScope::Heal)
                                .await
                        {
                            let reason = objection
                                .to_string()
                                .lines()
                                .find(|l| l.trim_start().starts_with("- "))
                                .map(|l| l.trim().trim_start_matches("- ").to_string())
                                .unwrap_or_else(|| "protected".to_string());
                            info!("keeping {} — its interrupted removal is refused ({}).", key, reason);
                            let mut j = self.journal.lock().await;
                            let _ = j.record_success(&entry.id, std::collections::HashMap::new());
                            kept.push(key.clone());
                            continue;
                        }
                    }

                    let remediation_res = if is_install {
                        // Remove before reinstalling: the interrupted install may have left a
                        // half-written package that a plain install would refuse or skip.
                        let _ = handler.remove(std::slice::from_ref(&package), sudo).await;
                        if let crate::core::journal::JournalAction::Install(spec) = &entry.action {
                            handler.install(std::slice::from_ref(spec), sudo).await
                        } else {
                            Ok(())
                        }
                    } else {
                        handler.remove(std::slice::from_ref(&package), sudo).await
                    };

                    let verb = if is_install { "reinstalled" } else { "removed" };
                    if remediation_res.is_ok() {
                        let mut j = self.journal.lock().await;
                        let _ = j.record_success(&entry.id, std::collections::HashMap::new());
                        info!("{} {} (completing an interrupted {}).", verb, key,
                            if is_install { "install" } else { "removal" });
                        recovered.push(format!("{} {}", verb, key));
                    } else {
                        error!(
                            "could not recover {} — {:?}. The system may be in a partial \
                             state for this package; re-run `linix sync`.",
                            key,
                            remediation_res.err()
                        );
                        failed.push(key);
                    }
                }
            }
        }

        // The summary a reader sees whether or not they had `--verbose` on: what actually
        // changed, in one line.
        if !recovered.is_empty() {
            info!("recovered {} operation(s): {}.", recovered.len(), recovered.join(", "));
        }
        if !kept.is_empty() {
            info!(
                "kept {} protected package(s) whose interrupted removal was refused: {}.",
                kept.len(),
                kept.join(", ")
            );
        }
        if !failed.is_empty() {
            warn!(
                "{} operation(s) could NOT be recovered: {}. Re-run `linix sync`.",
                failed.len(),
                failed.join(", ")
            );
        }

        let mut j = self.journal.lock().await;
        let _ = j.cleanup();
        Ok(())
    }
}
