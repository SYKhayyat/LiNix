use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::{LuaHooks, MetricsCollector};
use crate::backends::BackendRegistry;
use crate::config::Config;
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

        // The machine and the configuration disagree — which is what `on_drift` is for. Fired
        // before anything is applied, so a hook that wants to veto by other means (page someone,
        // open a ticket) is told while the drift is still the truth.
        let events = self.events();
        events
            .fire(
                crate::model::event::Event::OnDrift,
                serde_json::to_value(changes.generate_report()).unwrap_or_default(),
            )
            .await;

        // Before the snapshot and before any package is touched: refuse a removal set
        // that is oversized or takes something the system needs. `on_guard_refusal` fires
        // inside the guard, not here, so every command that removes gets it — see
        // `guard::refuse`.
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

        // 7f: a declared health check with no way to revert is refused here, before the first
        // package is touched — the only moment the answer is still actionable.
        self.require_revert_path(&changes)?;

        // U31: a health-check COMMAND is argv from the config, so it rides II.12's ledger. An
        // unapproved command cannot run, and a check that cannot run is a failed check — so this
        // refuses before the change rather than doing it and then reverting on a check LiNix was
        // never allowed to execute.
        self.require_health_commands_approved(&changes)?;

        // The pre-sync snapshot is a safety NET, not a precondition: a Windows System
        // Restore checkpoint needs admin (and System Restore enabled), and btrfs/timeshift
        // may be unavailable — none of which should abort a package sync. Policies that
        // TRULY require a snapshot gate on `has_provider()` upstream; here we warn and
        // proceed so a missing restore point never blocks the actual work.
        // Kept: a failing health check restores exactly this snapshot (7f), so the id has to
        // outlive the call that took it.
        let restore_point = match self
            .snapshot_manager
            .auto_snapshot(crate::core::snapshot::SnapshotLabel::PreSync)
            .await
        {
            Ok(snap) => snap.map(|s| s.id),
            Err(e) => {
                warn!(
                    "pre-sync safety snapshot unavailable ({}); proceeding without a restore point.",
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

            let state_to_save = self.state.lock().await.clone();
            tokio::task::spawn_blocking(move || state_to_save.save())
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

        let mut failed = Vec::new();
        for check in &checks {
            if Self::probe_ok(&check.probe).await {
                info!("  OK   {} ({})", check.subject, check.probe);
            } else {
                warn!("  FAIL {} ({})", check.subject, check.probe);
                failed.push(format!("{} ({})", check.subject, check.probe));
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
            Probe::Port(p) => tokio::net::TcpStream::connect(("127.0.0.1", *p))
                .await
                .is_ok(),
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
        // `max_parallel` is the user's knob for this engine, so it must be read here and
        // not left at the `patient()` default — a hardcoded default silently narrows the
        // setting's reach to `search` alone. Floor at 1: zero would stall the transaction.
        let mut tx_config = TransactionConfig::patient();
        tx_config.max_concurrent = self.config.max_parallel.max(1);
        tx_config.purge = self.config.remove.purge || self.config.purge_this_run;

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

    pub async fn heal(&self) -> Result<()> {
        let incomplete_actions = {
            let j = self.journal.lock().await;
            j.get_incomplete_actions()
        };

        if incomplete_actions.is_empty() {
            debug!("nothing to heal");
            return Ok(());
        }

        // S25: recovery is a mutation, so a preview reports it and stops. The check is here
        // rather than at the call sites because every caller of this function mutates, and
        // the call site that was missed is how a `--dry-run` came to reinstall packages.
        if self.config.dry_run {
            info!(
                "[DRY-RUN] would recover {} interrupted operation(s) from a previous run:",
                incomplete_actions.len()
            );
            for entry in &incomplete_actions {
                match &entry.action {
                    crate::core::journal::JournalAction::Install(spec) => {
                        info!("[DRY-RUN]   reinstall {}:{}", spec.backend, spec.name)
                    }
                    crate::core::journal::JournalAction::Remove { name, backend } => {
                        info!(
                            "[DRY-RUN]   remove {}:{} (subject to the guard)",
                            backend, name
                        )
                    }
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

                    // Completing an interrupted *removal* routes through the guard, so a
                    // protected package is never removed even during recovery. On refusal we
                    // KEEP the package and treat the entry as resolved — recovery completes,
                    // protection holds, and heal never gets stuck retrying a removal it will
                    // always refuse.
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
                            let _ = j.record_success(&entry.id, std::collections::HashMap::new());
                            kept.push(key.clone());
                            continue;
                        }
                    }

                    // V.64: recovery reinstates what was wanted and does not delete to get
                    // there. Re-running the install over a half-installed package is what
                    // every manager LiNix drives can do; uninstalling first was a removal the
                    // plan could not show and the guard never saw (S24).
                    let remediation_res = if is_install {
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
                        info!(
                            "{} {} (completing an interrupted {}).",
                            verb,
                            key,
                            if is_install { "install" } else { "removal" }
                        );
                        recovered.push(format!("{} {}", verb, key));
                    } else {
                        let e = remediation_res.err();
                        error!(
                            "could not recover {} — {}. {} The entry stays recorded as \
                             interrupted, so nothing claims this package is installed.",
                            key,
                            e.as_ref()
                                .map_or("no reason given".to_string(), |e| e.to_string()),
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
        {
            let mut j = self.journal.lock().await;
            let _ = j.cleanup();
        }

        // A command that says "1 operation(s) could NOT be recovered" and then exits 0 has
        // told a script the opposite of what it told the person reading it — `linix heal &&
        // echo ok` printed ok. U21 gave this program an exit vocabulary; the recovery path
        // was the last one not using it.
        if !failed.is_empty() {
            return Err(Error::Other(format!(
                "{} interrupted operation(s) could not be recovered: {}. Each is still \
                 recorded as interrupted, so `heal` will try again — read the error above \
                 for the one that says what to change.",
                failed.len(),
                failed.join(", ")
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
