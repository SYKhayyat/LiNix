use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::{LuaHooks, MetricsCollector};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::LockFile;
use crate::core::{
    CommandExecutor, Error, GraphAction, Journal, PackageSpec, Result, Retryability,
    SnapshotManager, StateRegistry, Transaction, TransactionConfig,
};
use crate::utils::progress::ProgressReporter;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, instrument, warn};

pub mod guard;
pub mod planner;
pub mod resolver;
pub mod saved_plan;

pub use self::planner::{ChangePlanner, HostBackends, PlanScope, Scope, SyncChanges};

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

    /// Start the pre-sync snapshot, and say so.
    ///
    /// Returns a future the caller joins immediately before the first mutating command. The
    /// announcement is the other half of the fix: `Checkpoint-Computer` measured **50.8s** on
    /// Windows and nothing in the output said it was happening, so the pause read as a hang.
    /// **Spawned, not merely constructed.** A future that is only held does nothing at all
    /// until it is awaited, so returning one would have overlapped exactly zero milliseconds.
    fn begin_snapshot(&self) -> tokio::task::JoinHandle<Result<Option<crate::core::Snapshot>>> {
        let manager = self.snapshot_manager.clone();
        if manager.has_provider() {
            info!("taking a pre-sync restore point (this can take a minute on Windows)...");
        }
        tokio::spawn(async move {
            manager
                .auto_snapshot(crate::core::snapshot::SnapshotLabel::PreSync)
                .await
        })
    }

    /// The scripts attached to LiNix's own events (XIII.13).
    ///
    /// Read at the moment of firing rather than held: the three files are tiny, and re-reading
    /// means the hash the approval ledger checks is the hash of what is on disk *now* — a hook
    /// edited during a long sync cannot run on an approval given to its previous contents.
    ///
    /// **Events fire on a real run, never on a preview.** Every fire site is inside `sync`,
    /// which `--dry-run` returns before reaching. That is the intended asymmetry: a hook has
    /// side effects out in the world — it pages someone, it opens a ticket — and a preview that
    /// sent the notification would be a preview that changed something. `plan` and `check` are
    /// the commands for looking.
    fn events(&self) -> crate::app::events::EventHooks {
        crate::app::events::EventHooks::load(self.config)
    }

    /// `scope` names the command that asked, so the removal guard can be enforced here —
    /// at the one point every drift-removal path funnels through — rather than at each
    /// caller, where it only takes one forgotten site to purge a system.
    #[instrument(skip(self, changes))]
    pub async fn sync(&self, mut changes: SyncChanges, scope: guard::GuardScope) -> Result<()> {
        let _heartbeat = self.executor.start_sudo_keepalive().await;

        // II.7c, before anything counts the plan. A graph that arrived from a plan file another
        // machine wrote, or from this machine's journal, can name a manager that is not here —
        // and every measurement below reads the graph: the guard's removal ceiling, the install
        // ceiling, `is_empty`, the report the `on_drift` event carries. Filtering after any of
        // them would count work that is not going to happen.
        changes.withdraw_what_this_machine_cannot_run(&self.registry);

        // The supply-chain gate (II.12), before any hook runs and before anything is touched:
        // a hook whose script is new or changed since you approved it stops the sync. Note the
        // `?` — the `run_before_sync` below swallows its own errors, so the authoritative stop
        // has to live here, where it propagates.
        self.hooks.verify_all_approved()?;
        let _ = self.hooks.run_before_sync().await;

        if changes.is_empty() {
            // `SyncChanges::skipped`'s own header: an empty plan with a non-empty `skipped` is
            // NOT `already up to date`. A machine whose whole config is pinned to managers it
            // does not have has converged nothing, and saying it is up to date is the lie the
            // list was added to stop.
            if changes.skipped.is_empty() {
                info!("already up to date");
            } else {
                for skip in &changes.skipped {
                    warn!("{} — {}", skip.key, skip.reason);
                }
            }
            return Ok(());
        }

        // Started here, joined below, and **not** waited on in between.
        //
        // The pre-sync snapshot is a safety NET, not a precondition — the comment at the join
        // says so and it is what makes this legal. On Windows it is `Checkpoint-Computer`,
        // measured at **50.8s**, with no faster API to swap to; taken as a barrier it was a
        // fixed ~51-second tax on every install and every uninstall, in front of work that had
        // to happen anyway. Everything between here and the join is read-only — an event, the
        // removal guard's per-backend queries, two approval checks — so none of it can outrun
        // the checkpoint it is overlapping.
        //
        // And it says it is happening. A silent 51-second pause reads as a hang; that is how
        // it was first reported.
        let taking_snapshot = self.begin_snapshot();

        // Gathered into one block so a refusal can stop the snapshot it was overlapping. A
        // sync that is refused must not leave a half-taken restore point behind it.
        let events = self.events();
        let preflight: Result<()> = async {
            // The machine and the configuration disagree — which is what `on_drift` is for.
            // Fired before anything is applied, so a hook that wants to veto by other means
            // (page someone, open a ticket) is told while the drift is still the truth.
            events
                .fire(
                    crate::model::event::Event::OnDrift,
                    serde_json::to_value(changes.generate_report()).unwrap_or_default(),
                )
                .await;

            // Before any package is touched: refuse a removal set that is oversized or takes
            // something the system needs. `on_guard_refusal` fires inside the guard, not here,
            // so every command that removes gets it — see `guard::refuse`.
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

            // 7f: a declared health check with no way to revert is refused here, before the
            // first package is touched — the only moment the answer is still actionable.
            self.require_revert_path(&changes)?;

            // U31: a health-check COMMAND is argv from the config, so it rides II.12's ledger.
            // An unapproved command cannot run, and a check that cannot run is a failed check —
            // so this refuses before the change rather than doing it and then reverting on a
            // check LiNix was never allowed to execute.
            self.require_health_commands_approved(&changes)?;
            Ok(())
        }
        .await;
        if let Err(e) = preflight {
            taking_snapshot.abort();
            return Err(e);
        }

        // Joined here, immediately before the first mutating command — which is the whole
        // requirement. A snapshot taken after the change would revert to the change.
        //
        // A safety NET, not a precondition: a Windows System Restore checkpoint needs admin
        // (and System Restore enabled), and btrfs/timeshift may be unavailable — none of which
        // should abort a package sync. Policies that TRULY require a snapshot gate on
        // `has_provider()` upstream; here we warn and proceed so a missing restore point never
        // blocks the actual work.
        // Kept: a failing health check restores exactly this snapshot (7f), so the id has to
        // outlive the call that took it.
        let restore_point = match taking_snapshot.await {
            Ok(Ok(snap)) => snap.map(|s| s.id),
            Ok(Err(e)) => {
                warn!(
                    "pre-sync safety snapshot unavailable ({}); proceeding without a restore point.",
                    e
                );
                None
            }
            Err(e) => {
                warn!(
                    "the pre-sync snapshot task did not finish ({}); proceeding without a \
                     restore point.",
                    e
                );
                None
            }
        };

        let result = {
            let mut state_guard = self.state.lock().await;
            self.execute_transaction(&changes, &mut state_guard).await
        };
        // Set below, once the transaction has succeeded and the out-of-tree modules have been
        // rebuilt. Declared here so it survives the block.
        let mut kernel_outcome: Result<()> = Ok(());

        if result.is_ok() {
            debug!("Finalizing transaction state and persistence.");

            // Serialised under the lock, written after it: the alternative was a deep clone of
            // every managed package — `properties` HashMap included — purely to cross a thread
            // boundary with data that was about to become one string anyway.
            let to_write = self.state.lock().await.snapshot()?;
            tokio::task::spawn_blocking(move || to_write.write())
                .await
                .map_err(|e| {
                    Error::Other(format!("Kernel panic during state persistence: {}", e))
                })??;

            if self.config.quiet {
                self.metrics.print_summary_quiet();
            } else {
                self.metrics.print_summary(narration_for(scope));
            }

            // XIII.1: the out-of-tree modules, before the health checks — a machine whose
            // graphics driver did not rebuild is not healthy in any sense a `@health=` probe
            // would notice, and both are still recoverable here, which is the property that
            // ends at the reboot.
            //
            // Kept rather than logged and dropped: 7g says a module that will not build **fails
            // loudly**, and a message on stderr under an exit code of 0 is not loud — a script
            // that checks the exit code would carry on to the reboot that makes it permanent.
            // The packages stay installed either way: the kernel IS on the machine, and
            // reporting otherwise would be the lie.
            kernel_outcome = self.rebuild_kernel_modules(&changes).await;

            // XIII.5: and then whether the machine still works. A failure restores the snapshot
            // taken above.
            self.verify_health(&changes, restore_point.as_deref()).await;

            // `after_sync` fires LAST, after the health check that can revert everything above
            // it. Firing it earlier meant a hook could post "sync complete" and then have the
            // machine rolled back underneath it — the notification would be the only surviving
            // record of a change that no longer exists.
            let _ = self.hooks.run_after_sync().await;
            events
                .fire(
                    crate::model::event::Event::AfterSync,
                    serde_json::json!({
                        "installed": changes.total_install(),
                        "removed": changes.total_remove(),
                    }),
                )
                .await;

            // The manifest history is git now (the generation format was deleted): the commit
            // that records this change is made by `git_autocommit` in `perform_maintenance`,
            // after a successful sync. Snapshot retention still runs here.
            self.prune_snapshots_after_sync().await;

            let mut j = self.journal.lock().await;
            let _ = j.cleanup();
        }

        // The transaction's own failure comes first: it is the more fundamental one, and a
        // kernel-module failure is only reachable when the transaction succeeded anyway.
        result.and(kernel_outcome)
    }

    /// Rebuild the out-of-tree kernel modules, when this sync changed a kernel (XIII.1).
    ///
    /// **LiNix builds nothing.** DKMS is already on the machine and already knows how to build
    /// a module; what LiNix contributes is the fact DKMS cannot know — that the kernel just
    /// changed, under a manager whose hook does not cover it. The distribution's own DKMS hook
    /// fires for the distribution's own package manager, and LiNix's premise is several at once.
    ///
    /// **It runs before the reboot and fails loudly**, because that is the whole value: a
    /// module that will not build is recoverable while the running kernel still has it, and
    /// after a reboot it is a machine with no graphics driver or no network.
    async fn rebuild_kernel_modules(&self, changes: &SyncChanges) -> Result<()> {
        use crate::model::kernel;

        // Linux only, and only when a kernel actually changed. Both are cheap enough to check
        // that this costs nothing on the runs — nearly all of them — where neither holds.
        if !cfg!(target_os = "linux") || self.config.dry_run {
            return Ok(());
        }
        let names: Vec<String> = changes
            .graph
            .node_weights()
            .map(|w| match w {
                GraphAction::Install(spec) => spec.name.clone(),
                GraphAction::Remove { name, .. } => name.clone(),
            })
            .collect();
        let kernels = kernel::kernels_in(names.iter().map(String::as_str));
        if kernels.is_empty() {
            return Ok(());
        }
        if !self.executor.command_exists_sync("dkms") {
            debug!("a kernel changed and this machine has no dkms — nothing to rebuild.");
            return Ok(());
        }

        info!(
            "a kernel package changed ({}) — rebuilding out-of-tree modules.",
            kernels.join(", ")
        );
        // `autoinstall` is DKMS's own "build whatever needs building for the kernels that are
        // installed". Driving it beats enumerating modules and calling `dkms install` per
        // module: DKMS knows which kernels are present and which modules target them, and a
        // second implementation of that would be wrong the first time a distribution changed.
        let built = self.executor.run("dkms", &["autoinstall"], true).await;

        // Read either way. `autoinstall`'s exit code says something went wrong; `status` says
        // which module, which is what a reader can act on — and a zero exit with a module left
        // at `built` is a case the exit code alone would call success.
        let status = self
            .executor
            .run_output("dkms", &["status"], false)
            .await
            .unwrap_or_default();
        let stuck = kernel::not_installed(&status);

        if stuck.is_empty() {
            match built {
                Ok(_) => {
                    debug!("every out-of-tree module is installed.");
                    Ok(())
                }
                // Nothing is stuck, so whatever autoinstall complained about did not leave a
                // module unbuilt. Worth saying, not worth failing a converged sync over.
                Err(e) => {
                    warn!("`dkms autoinstall` reported an error ({}), but every module it holds is installed.", e);
                    Ok(())
                }
            }
        } else {
            Err(Error::Other(kernel::failed_to_build(
                &stuck,
                &kernels.join(", "),
            )))
        }
    }

    /// Every health check this change is subject to (XIII.5, U7): the `@health=` on each line
    /// being installed, **and** the machine-wide list in `preferences.toml`.
    ///
    /// **Both, from one place.** U7 ruled they are not alternatives, so the code that decides
    /// whether the machine is healthy must never be able to consult one and forget the other —
    /// which is what two collection sites would eventually mean.
    fn declared_health_checks(&self, changes: &SyncChanges) -> Vec<crate::model::health::Check> {
        use crate::model::health::{Check, Probe};

        let mut checks = Vec::new();
        for w in changes.graph.node_weights() {
            if let GraphAction::Install(spec) = w {
                if let Some(probe) = spec.options.get("health").and_then(|s| Probe::parse(s)) {
                    checks.push(Check {
                        subject: format!("{}:{}", spec.backend, spec.name),
                        probe,
                    });
                }
            }
        }
        // The machine-wide half: the boot, the network, the thing two packages away. Declared
        // once and checked after every change, because that is what "is the machine still
        // working" means.
        for written in &self.config.health {
            if let Some(probe) = Probe::parse(written) {
                checks.push(Check {
                    subject: "preferences.toml".to_string(),
                    probe,
                });
            }
        }
        checks
    }

    /// Refuse, **before anything is installed**, when health checks are declared and nothing
    /// could revert them (7f).
    ///
    /// A health check that cannot revert reports the breakage and leaves it in place — strictly
    /// worse than not checking, because you are told the machine is broken and given no way
    /// back. The only moment that fact is actionable is before the change.
    fn require_revert_path(&self, changes: &SyncChanges) -> Result<()> {
        match crate::model::health::refusal_if_unrevertable(
            &self.declared_health_checks(changes),
            self.snapshot_manager.has_provider(),
            self.config.dry_run,
        ) {
            Some(refusal) => Err(Error::Refused(refusal)),
            None => Ok(()),
        }
    }

    /// Refuse before the change when a declared health *command* has not been approved through
    /// the II.12 ledger (U31). Port probes run no code and are never gated. `linix lock` is the
    /// one place that approves, so the refusal names it.
    fn require_health_commands_approved(&self, changes: &SyncChanges) -> Result<()> {
        use crate::core::hook_lock::{hash_script, health_id, HookLedger};
        use crate::model::health::Probe;

        // A dry run changes nothing, so no health command runs — previewing must not be blocked
        // by an approval that only matters when something is actually applied.
        if self.config.dry_run {
            return Ok(());
        }
        let commands: Vec<crate::model::health::Check> = self
            .declared_health_checks(changes)
            .into_iter()
            .filter(|c| matches!(c.probe, Probe::Command(_)))
            .collect();
        if commands.is_empty() {
            return Ok(());
        }
        let ledger = HookLedger::load(&HookLedger::path_in(&self.config.layout().locks_dir()))
            .unwrap_or_default();
        let mut unapproved = Vec::new();
        for check in &commands {
            if let Probe::Command(cmd) = &check.probe {
                if !ledger
                    .verdict(&health_id(cmd), &hash_script(cmd))
                    .is_approved()
                {
                    unapproved.push(format!("{} ({})", check.subject, cmd));
                }
            }
        }
        if unapproved.is_empty() {
            return Ok(());
        }
        Err(Error::Refused(format!(
            "refusing to start: {} health-check command(s) have not been approved and LiNix will \
             not run a command from the configuration it has not seen — a check it cannot run is \
             a failed check, and a failed check reverts the change.\n  {}\n  \
             Review them, then run `linix lock` to approve them.",
            unapproved.len(),
            unapproved.join("\n  ")
        )))
    }

    /// Run the declared checks and act on the result: healthy, or restore the snapshot this
    /// sync took before it started.
    ///
    /// One revert path for both kinds of check (U7). The machine does not care whether it was
    /// a package's own probe or the machine-wide one that noticed — a broken nginx and a broken
    /// boot both mean go back.
    async fn verify_health(&self, changes: &SyncChanges, snapshot: Option<&str>) {
        use crate::model::health::{self, Outcome};

        let checks = self.declared_health_checks(changes);
        if checks.is_empty() {
            return;
        }
        info!("running {} health check(s)...", checks.len());

        // Concurrent: these are independent probes, nothing orders them, and the user is
        // waiting on the answer to learn whether their change is about to be reverted.
        // Ordered, so the log reads in declaration order and the failure list is stable.
        use futures::stream::StreamExt;
        let outcomes: Vec<(String, bool)> = futures::stream::iter(checks.iter())
            .map(|check| async move {
                (
                    format!("{} ({})", check.subject, check.probe),
                    Self::probe_ok(&check.probe).await,
                )
            })
            .buffered(self.config.max_parallel.max(1))
            .collect()
            .await;

        let mut failed = Vec::new();
        for (described, ok) in outcomes {
            if ok {
                info!("  OK   {}", described);
            } else {
                warn!("  FAIL {}", described);
                failed.push(described);
            }
        }

        match Outcome::of(failed, snapshot) {
            Outcome::Healthy => debug!("every health check passed."),
            Outcome::Revert { failed, snapshot } => {
                warn!("{}", health::reverted_message(&failed, &snapshot));
                if let Err(e) = self.snapshot_manager.restore(&snapshot).await {
                    // The revert itself failing is the worst outcome here, so it is reported
                    // as exactly that rather than folded into the health failure above.
                    error!(
                        "restoring {} FAILED: {}. The machine is in the state the change left \
                         it, and the health check that failed is still failing.",
                        snapshot, e
                    );
                } else {
                    info!("restored {}.", snapshot);
                }
            }
            // Only reachable on a dry run, or if the provider vanished between the pre-flight
            // check and here — `require_revert_path` refuses this case before anything runs.
            Outcome::FailedWithoutRevert { failed } => {
                warn!("{}", health::not_reverted_message(&failed))
            }
        }
    }

    /// Evaluate one probe. `Port` succeeds if a TCP connection to localhost opens; `Command`
    /// succeeds if the shell command exits 0.
    async fn probe_ok(probe: &crate::model::health::Probe) -> bool {
        use crate::model::health::Probe;
        match probe {
            // Bounded, and the number is the point. A *closed* localhost port refuses at once,
            // which is the common case — but a **filtered** one, which `apply/firewall.rs` can
            // itself create, is dropped rather than refused, and an unbounded connect then
            // waits out the OS default: ~21s on Windows, ~130s on Linux. This decides whether
            // to roll a sync back; it must not be the thing that hangs. Five seconds is far
            // above any localhost round trip and far below every OS's own give-up.
            Probe::Port(p) => tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tokio::net::TcpStream::connect(("127.0.0.1", *p)),
            )
            .await
            .is_ok_and(|r| r.is_ok()),
            Probe::Command(cmd) => crate::app::bisect::run_test(cmd).await,
        }
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
        let tx_config = TransactionConfig::from_config(self.config);

        // The engine gets the configuration because its rollback removes, and a removal is
        // guarded wherever it is issued (V.64) — including here, where the plan-time gate
        // above cannot see it.
        let tx = Transaction::with_config(
            changes.graph.clone(),
            self.registry.clone(),
            self.journal.clone(),
            self.diagnostics.clone(),
            Arc::new(self.config.clone()),
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
                res.retries,
                res.bytes_downloaded,
                res.batch_size,
            );
        }

        for idx in changes.graph.node_indices() {
            match &changes.graph[idx] {
                GraphAction::Install(spec) => {
                    let source = spec.options.get("__source").map_or("sync", String::as_str);
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

    /// Collapse the journal's unresolved entries to one recovery per operation, carrying every
    /// id that named it so a single attempt resolves them all.
    ///
    /// Keyed on what a recovery would actually *do* — the backend, the name, and whether it is
    /// an install or a removal. Two interrupted installs of the same spec are one reinstall; an
    /// interrupted install and an interrupted removal of the same package are not, and must stay
    /// two. First-seen order is kept so the report reads in the order the journal recorded.
    fn one_per_operation(
        entries: Vec<crate::core::journal::JournalEntry>,
    ) -> Vec<(crate::core::journal::JournalEntry, Vec<String>)> {
        use crate::core::journal::JournalAction;
        let mut order: Vec<(crate::core::journal::JournalEntry, Vec<String>)> = Vec::new();
        let mut seen: std::collections::HashMap<(String, String, bool), usize> =
            std::collections::HashMap::new();
        for entry in entries {
            let (backend, name) = entry.action.identity();
            // The third element is "is this an install": an interrupted install and an
            // interrupted removal of one package are two operations and must stay two. A
            // script's own two kinds are already distinct in `backend` (`exec` / `exec-undo`).
            let key = (
                backend.to_string(),
                name.to_string(),
                matches!(entry.action, JournalAction::Install(_)),
            );
            match seen.get(&key) {
                Some(&at) => order[at].1.push(entry.id),
                None => {
                    seen.insert(key, order.len());
                    let id = entry.id.clone();
                    order.push((entry, vec![id]));
                }
            }
        }
        order
    }

    /// The work recovery would run to finish a logged action, or `None` when re-running it is
    /// not a recovery.
    ///
    /// **The one place the log's vocabulary becomes the engine's.** It used to be six places,
    /// each matching on `JournalAction` for its own reason, and adding a variant that is not
    /// package work to that shape would have meant six chances to route a script down a
    /// package path. Past this function `heal` speaks `GraphAction` and can only say things
    /// about packages.
    fn replay_of(action: &crate::core::journal::JournalAction) -> Option<GraphAction> {
        use crate::core::journal::JournalAction;
        match action {
            JournalAction::Install(spec) => Some(GraphAction::Install(spec.clone())),
            JournalAction::Remove { name, backend } => Some(GraphAction::Remove {
                name: name.clone(),
                backend: backend.clone(),
            }),
            // See `JournalAction::is_replayable`: a half-run script has no recorded progress
            // and no declared end state, so replaying it is inventing a mutation.
            JournalAction::Exec { .. } | JournalAction::ExecUndo { .. } => None,
        }
    }

    pub async fn heal(&self) -> Result<()> {
        // One package, one recovery. `record_start` mints a fresh id per attempt, so a
        // declaration that fails on every sync appends a *new* operation every time and none of
        // them is ever purged — one sweep's journal held **22 operations for a single
        // `scoop:linix-no-such-pkg-zzz`**, and `heal` made 23 real `scoop install` round trips
        // for that one name. The cost is unbounded in the number of past attempts, on the
        // command that runs before every `sync` and inside every `watch` tick.
        //
        // They are not 22 operations. They are one operation attempted 22 times, so one attempt
        // decides all of them: on success every entry that named the same thing is resolved,
        // because the package it named is now installed.
        let interrupted = {
            let j = self.journal.lock().await;
            Self::one_per_operation(j.interrupted_actions())
        };

        if interrupted.is_empty() {
            debug!("nothing to heal");
            return Ok(());
        }

        // S25: recovery is a mutation, so a preview reports it and stops. The check is here
        // rather than at the call sites because every caller of this function mutates, and
        // the call site that was missed is how a `--dry-run` came to reinstall packages.
        if self.config.dry_run {
            info!(
                "[DRY-RUN] would recover {} interrupted operation(s) from a previous run:",
                interrupted.len()
            );
            for (entry, _) in &interrupted {
                match Self::replay_of(&entry.action) {
                    Some(GraphAction::Install(spec)) => {
                        info!("[DRY-RUN]   reinstall {}:{}", spec.backend, spec.name)
                    }
                    Some(GraphAction::Remove { name, backend }) => {
                        info!(
                            "[DRY-RUN]   remove {}:{} (subject to the guard)",
                            backend, name
                        )
                    }
                    // Nothing to replay, so the preview says the only true thing there is:
                    // what was interrupted. Reporting it is the whole of the real run's
                    // action too, so preview and run describe the same machine.
                    None => info!(
                        "[DRY-RUN]   report: {}",
                        entry.action.describe_interruption()
                    ),
                }
            }
            return Ok(());
        }

        // S6: healing is automatic (a half-finished transaction is drift, and removing drift
        // is sync's job — asking permission would ask permission to do sync's own job). But
        // automatic is not silent: a recovery nobody sees is exactly the class of bug this
        // whole document is about (P3). Report every action taken, by name, and summarize.
        info!(
            "recovering {} interrupted operation(s) from a previous run.",
            interrupted.len()
        );
        let mut recovered: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        // Packages whose interrupted removal the guard refused: kept, not removed (owner
        // decision), and the entry resolved so heal completes rather than sticking.
        let mut kept: Vec<String> = Vec::new();
        // Interrupted work recovery can only account for, never finish — a half-run script.
        // Counted separately from `recovered` because nothing on the machine changed.
        let mut reported: Vec<String> = Vec::new();
        // Entries recovery cannot act on at all — the manager is not registered on this machine,
        // or it is registered and cannot install or remove. Until 2026-08-04 the loop below was
        // two nested `if let`s with **no `else` on either**, so such an entry was neither
        // recovered, nor failed, nor mentioned: `heal` did nothing about it and returned Ok.
        // That is W36's finding one branch over — W36 was "says it could not recover and exits
        // 0", this is "says nothing and exits 0", and the second is worse because there is
        // nothing in the output to read.
        let mut unreachable: Vec<String> = Vec::new();
        // Two passes, because they answer different questions and only the second one costs a
        // process. This one decides what recovery can even attempt: a manager that is not set up
        // here, a manager that cannot install or remove, and a removal the guard refuses are all
        // settled without touching the machine.
        let mut runnable: Vec<(GraphAction, Vec<String>)> = Vec::new();
        for (entry, ids) in interrupted {
            // The one place the log's vocabulary is turned into the engine's. Everything below
            // reads a `GraphAction`, so a variant that is not package work cannot silently take
            // a package path — it never gets one.
            let Some(action) = Self::replay_of(&entry.action) else {
                // Recovery cannot finish a script, and must not pretend to. What it owes the
                // user is the account nobody was given while these entries did not exist: a
                // machine killed mid-`exec:` came back, said nothing, and ran the script again
                // from the top on the next sync. The entry is then resolved — it has been
                // acted on as fully as it can be, and leaving it open would keep
                // `needs_recovery` true for ever and re-report it in front of every sync.
                warn!("{}", entry.action.describe_interruption());
                reported.push(entry.action.key());
                // Resolved as a FAILURE, not a success. The entry has been acted on as fully as
                // it can be, and leaving it open would keep `needs_recovery` true for ever and
                // re-report it in front of every sync — but the script did not finish, and a
                // log that records `Completed` for a mutation that was interrupted is exactly
                // the dishonest record this change exists to remove. `Failed` is terminal, so
                // recovery still stops asking, and it ages out on the same rule as any other
                // terminal entry while carrying the reason.
                let mut j = self.journal.lock().await;
                for id in &ids {
                    let _ = j.record_failure(
                        id,
                        "interrupted part-way through; recovery cannot finish a script and \
                         reported it instead",
                    );
                }
                continue;
            };
            let (backend, package, is_install) = match &action {
                GraphAction::Install(spec) => (spec.backend.clone(), spec.name.clone(), true),
                GraphAction::Remove { name, backend } => (backend.clone(), name.clone(), false),
            };
            let key = format!("{}:{}", backend, package);

            let Some(backend_cap) = self.registry.get(&backend) else {
                unreachable.push(format!(
                    "{} (no manager named `{}` is set up on this machine, so there is nothing to \
                     complete the operation with)",
                    key, backend
                ));
                continue;
            };
            if backend_cap.as_installable().is_none() {
                unreachable.push(format!(
                    "{} (`{}` cannot install or remove, so an interrupted install of it is not \
                     something recovery can finish)",
                    key, backend
                ));
                continue;
            }

            // Completing an interrupted *removal* routes through the guard, so a protected
            // package is never removed even during recovery. On refusal we KEEP the package and
            // treat the entry as resolved — recovery completes, protection holds, and heal never
            // gets stuck retrying a removal it will always refuse.
            if !is_install {
                let removal = [(backend.clone(), package.clone())];
                if let Err(objection) = guard::enforce(
                    self.config,
                    &self.registry,
                    &removal,
                    guard::GuardScope::Heal,
                )
                .await
                {
                    let reason = objection
                        .to_string()
                        .lines()
                        .find(|l| l.trim_start().starts_with("- "))
                        .map(|l| l.trim().trim_start_matches("- ").to_string())
                        .unwrap_or_else(|| "protected".to_string());
                    info!(
                        "keeping {} — its interrupted removal is refused ({}).",
                        key, reason
                    );
                    let mut j = self.journal.lock().await;
                    for id in &ids {
                        let _ = j.record_success(id);
                    }
                    kept.push(key.clone());
                    continue;
                }
            }
            runnable.push((action, ids));
        }

        if !runnable.is_empty() {
            // **The recovery runs on the transaction engine, and not on a second copy of it.**
            // This loop used to be `for entry in ... { handler.install(from_ref(spec)).await }`
            // — serial, one package per command, beside a batched parallel DAG. Measured on one
            // host in one minute: `sync --dry-run` 3.9x overlap over 2 waves, `heal` **0.2x over
            // 27 waves for 27 commands**, which is the definition of serial. 23 of those 30
            // attempts were the same package.
            //
            // Two settings differ from a sync's, and both follow from what recovery *is*. It
            // does not roll back: each entry is a separate piece of work a dead run left behind,
            // and undoing a recovery that succeeded to punish one that failed would put the
            // machine further from what was wanted, not closer. And it continues past a failure
            // for the same reason — one operation nobody can finish must not leave every other
            // one unfinished.
            //
            // V.64: recovery reinstates what was wanted and does not delete to get there.
            // Re-running the install over a half-installed package is what every manager LiNix
            // drives can do; uninstalling first was a removal the plan could not show and the
            // guard never saw (S24).
            // An interrupted install whose dependency is also interrupted must wait for it, or
            // recovery reproduces the ordering failure that a dependency edge exists to prevent.
            // `add_installs` is where that rule lives — this was the fourth hand-built copy of
            // it, and of the four, two had no edges at all.
            use petgraph::graph::NodeIndex;
            let mut changes = SyncChanges::default();
            changes.add_installs(
                &runnable
                    .iter()
                    .filter_map(|(action, _)| match action {
                        GraphAction::Install(spec) => Some(spec.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            );
            for (action, _) in &runnable {
                if let GraphAction::Remove { name, backend } = action {
                    changes.add_removal(backend, name);
                }
            }
            // The ids that named each operation, keyed the way the graph's weights identify it
            // — an interrupted install and an interrupted removal of one package are two
            // operations (`one_per_operation`), so the flag is part of the key.
            let mut ids_by_key: std::collections::HashMap<(String, bool), Vec<String>> =
                std::collections::HashMap::new();
            for (action, ids) in runnable {
                let is_install = matches!(action, GraphAction::Install(_));
                ids_by_key
                    .entry((planner::node_key(&action), is_install))
                    .or_default()
                    .extend(ids);
            }
            let nodes: Vec<(NodeIndex, GraphAction, Vec<String>)> = changes
                .graph
                .node_indices()
                .map(|idx| {
                    let action = changes.graph[idx].clone();
                    let is_install = matches!(action, GraphAction::Install(_));
                    let ids = ids_by_key
                        .remove(&(planner::node_key(&action), is_install))
                        .unwrap_or_default();
                    (idx, action, ids)
                })
                .collect();

            let mut tx = Transaction::with_config(
                changes.graph.clone(),
                self.registry.clone(),
                self.journal.clone(),
                self.diagnostics.clone(),
                Arc::new(self.config.clone()),
                TransactionConfig {
                    max_concurrent: self.config.max_parallel.max(1),
                    auto_rollback: false,
                    continue_on_error: true,
                    ..TransactionConfig::patient()
                },
            );
            let results = tx.execute_with_telemetry().await?;

            let mut outcome: std::collections::HashMap<NodeIndex, Result<()>> =
                std::collections::HashMap::new();
            for res in results {
                self.metrics.record_operation(
                    &res.package_name,
                    &res.backend_name,
                    res.start_time,
                    res.result.is_ok(),
                    res.result.as_ref().err().map(|e| e.to_string()),
                    res.retries,
                    res.bytes_downloaded,
                    res.batch_size,
                );
                outcome.insert(res.node_index, res.result);
            }

            for (idx, action, ids) in nodes {
                let (key, verb, is_install) = match &action {
                    GraphAction::Install(spec) => (
                        format!("{}:{}", spec.backend, spec.name),
                        "reinstalled",
                        true,
                    ),
                    GraphAction::Remove { name, backend } => {
                        (format!("{}:{}", backend, name), "removed", false)
                    }
                };
                match outcome.remove(&idx) {
                    Some(Ok(())) => {
                        // The ledger, not only the log. `execute_transaction` records ownership
                        // through `state.add` for every install it performs; recovery performs
                        // the same install by a different route and recorded nothing, so a
                        // package `heal` put back was on the machine and under nobody's
                        // management. Measured, with a control:
                        //
                        //   no crash:  linix -y uninstall apt:pv   -> remove 1, gone
                        //   SIGKILL + heal, then the same command  -> "already up to date", still there
                        //   linix why apt:dos2unix -> 'apt:dos2unix' is not under LiNix management.
                        //
                        // Nothing looked wrong: the sync after `heal` converges, because the
                        // package IS installed. The damage only appears when you try to take it
                        // away — the machine keeps it for ever and the one command for removing
                        // it reports success. That is Q28's class exactly, on the path that runs
                        // when nobody is watching.
                        match &action {
                            GraphAction::Install(spec) => {
                                let source =
                                    spec.options.get("__source").map_or("sync", String::as_str);
                                let mut state = self.state.lock().await;
                                state.add(
                                    &spec.backend,
                                    &spec.name,
                                    None,
                                    spec.options.clone(),
                                    source,
                                    false,
                                );
                            }
                            GraphAction::Remove { name, backend } => {
                                let mut state = self.state.lock().await;
                                state.remove(backend, name);
                            }
                        }
                        let mut j = self.journal.lock().await;
                        for id in &ids {
                            let _ = j.record_success(id);
                        }
                        info!(
                            "{} {} (completing an interrupted {}).",
                            verb,
                            key,
                            if is_install { "install" } else { "removal" }
                        );
                        recovered.push(format!("{} {}", verb, key));
                    }
                    outcome => {
                        let e = match outcome {
                            Some(Err(e)) => Some(e),
                            // A node the engine returned nothing for is one it never reached.
                            // Silence here is what let an unhandled branch report success in
                            // 2026-08-04; it is an error with a reason, not an absence.
                            _ => None,
                        };
                        error!(
                            "could not recover {} — {}. {} The entry stays recorded as \
                             interrupted, so nothing claims this package is installed.",
                            key,
                            e.as_ref()
                                .map_or("the recovery did not reach it".to_string(), |e| e
                                    .to_string()),
                            what_to_do_about(e.as_ref(), &key),
                        );
                        failed.push(key);
                    }
                }
            }
        }

        // The summary a reader sees whether or not they had `--verbose` on: what actually
        // changed, in one line.
        if !recovered.is_empty() {
            info!(
                "recovered {} operation(s): {}.",
                recovered.len(),
                recovered.join(", ")
            );
        }
        if !kept.is_empty() {
            info!(
                "kept {} protected package(s) whose interrupted removal was refused: {}.",
                kept.len(),
                kept.join(", ")
            );
        }
        // `warn!`, and separate from the line above, because nothing was put right: these were
        // reported, not recovered, and folding them into the recovered count would be the
        // summary claiming a repair that did not happen.
        if !reported.is_empty() {
            warn!(
                "{} interrupted script(s) could not be completed by recovery and were reported \
                 above: {}.",
                reported.len(),
                reported.join(", ")
            );
        }
        // And it reaches disk. The registry lives in memory until somebody serialises it, and
        // `heal` is a whole command — nothing runs after it to write what it changed, so an
        // ownership record made above and not written here dies with the process and the orphan
        // comes straight back. Same shape as the finalisation in `run`: serialised under the
        // lock, written outside it.
        if !recovered.is_empty() || !kept.is_empty() {
            let to_write = self.state.lock().await.snapshot()?;
            tokio::task::spawn_blocking(move || to_write.write())
                .await
                .map_err(|e| Error::Other(format!("writing the registry after heal: {}", e)))??;
        }
        {
            let mut j = self.journal.lock().await;
            let _ = j.cleanup();
        }

        // A command that says "1 operation(s) could NOT be recovered" and then exits 0 has
        // told a script the opposite of what it told the person reading it — `linix heal &&
        // echo ok` printed ok. U21 gave this program an exit vocabulary; the recovery path
        // was the last one not using it.
        //
        // `unreachable` is here for the same reason and it is the harder half: those entries
        // produced no error to print, so before they were collected the only evidence that
        // anything had been skipped was a number in the journal nobody was reading.
        if !failed.is_empty() || !unreachable.is_empty() {
            let mut named: Vec<String> = Vec::new();
            named.extend(failed.iter().cloned());
            named.extend(unreachable.iter().cloned());
            let advice = if unreachable.is_empty() {
                "Each is still recorded as interrupted, so `heal` will try again — read the \
                 error above for the one that says what to change."
            } else {
                "Each is still recorded as interrupted. `heal` will try again, but an operation \
                 whose manager is not set up here cannot complete until that manager is — \
                 `linix check health` says which are."
            };
            return Err(Error::Other(format!(
                "{} interrupted operation(s) could not be recovered: {}. {}",
                named.len(),
                named.join(", "),
                advice
            )));
        }
        Ok(())
    }
}

/// What to tell a user about an interrupted operation the recovery could not complete.
///
/// Driven off the classification the error already carries rather than a single sentence for
/// every case: `heal` used to print `retry: Permanent, absent_name: true` — in Rust's `Debug`
/// syntax, internal field names and all — and then advise *"re-run `linix sync`"*, which is
/// the one thing that cannot help when the name does not exist.
fn what_to_do_about(e: Option<&Error>, key: &str) -> String {
    let Some(e) = e else {
        return "Re-run `linix heal`.".to_string();
    };
    if e.says_a_name_is_absent() {
        return format!(
            "The manager says that name does not exist, so `sync` will keep failing the same \
             way until the line naming it is corrected or removed with `linix unmanage {key}`."
        );
    }
    match e.retryability() {
        Retryability::Transient => {
            "That failure is a passing one — a window, a lock or a connection. Run `linix heal` \
             again."
                .to_string()
        }
        Retryability::Permanent | Retryability::Exhausted => {
            "The same command will fail the same way, so fix the cause the error names before \
             re-running `linix heal`."
                .to_string()
        }
        Retryability::Unknown => "Re-run `linix heal`.".to_string(),
    }
}
