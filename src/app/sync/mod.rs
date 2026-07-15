use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::{LuaHooks, MetricsCollector, ShimManager};
use crate::backends::BackendRegistry;
use crate::config::manifest::ManifestEngine;
use crate::config::Config;
use crate::core::security::generate_checksum;
use crate::core::{
    CommandExecutor, Error, GraphAction, Journal, PackageSpec, Result, SnapshotManager,
    StateRegistry, Transaction, TransactionConfig,
};
use crate::utils::progress::ProgressReporter;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing::{debug, error, info, instrument, warn};

pub mod planner;
pub mod resolver;
pub mod saved_plan;

pub use self::planner::{ChangePlanner, ScopedFilter, SyncChanges};
pub use self::resolver::StateResolver;
pub use self::saved_plan::{SavedPlan, PLAN_SCHEMA};

/// Interface for system state resolution logic.
#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    async fn resolve_desired_state(
        &self,
    ) -> Result<std::collections::HashMap<String, Vec<PackageSpec>>>;
}

/// Interface for transaction planning logic.
#[async_trait::async_trait]
pub trait Planner: Send + Sync {
    async fn plan(
        &self,
        desired: &std::collections::HashMap<String, Vec<PackageSpec>>,
        scope: ScopedFilter,
    ) -> Result<SyncChanges>;
}

/// Primary entry point for parallel system synchronization.
///
/// The SyncEngine manages the full lifecycle of a system transformation,
/// ensuring that safety snapshots are taken before modifications and that
/// system state is correctly consolidated in the registry.
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
    pub manifest_engine: ManifestEngine,
    pub diagnostics: Arc<FailureDiagnosticEngine>,
}

impl<'a> SyncEngine<'a> {
    /// Initializes the engine with the full kernel context.
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
        let manifest_engine = ManifestEngine::new(&config.groups_dir);
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
            manifest_engine,
            diagnostics,
        }
    }

    /// Primary execution driver. Translates SyncChanges into OS-level modifications.
    #[instrument(skip(self, changes))]
    pub async fn sync(&self, changes: SyncChanges) -> Result<()> {
        let _heartbeat = self.executor.start_sudo_keepalive().await;
        let _ = self.hooks.run_before_sync().await;

        if changes.is_empty() {
            info!("Sync: OS state is consistent with declarative manifests.");
            return Ok(());
        }

        // 1. Capture atomic safety snapshot (Feature 2). This is a safety NET, not a
        //    precondition: a Windows System Restore checkpoint needs admin (and System
        //    Restore enabled), and btrfs/timeshift may be unavailable — none of which
        //    should abort a package sync. Policies that TRULY require a snapshot gate on
        //    `has_provider()` upstream; here we warn and proceed so a missing restore
        //    point never blocks the actual work.
        if let Err(e) = self.snapshot_manager.auto_snapshot("pre_sync").await {
            warn!(
                "Sync: pre-sync safety snapshot unavailable ({}); proceeding without a restore point.",
                e
            );
        }

        // 2. Execute Transaction
        let result = {
            let mut state_guard = self.state.lock().await;
            self.execute_transaction(&changes, &mut state_guard).await
        };

        // 3. Post-Transaction Consolidation
        if result.is_ok() {
            debug!("Sync: Finalizing transaction state and persistence.");

            let state_to_save = self.state.lock().await.clone();
            tokio::task::spawn_blocking(move || state_to_save.save())
                .await
                .map_err(|e| {
                    Error::Other(format!("Kernel panic during state persistence: {}", e))
                })??;

            // Scope the state guard to shim reconciliation ONLY. `record_generation()`
            // below re-acquires `self.state` internally, and `tokio::sync::Mutex` is not
            // re-entrant — holding the guard across that call self-deadlocks the whole
            // sync/prune/upgrade after the transaction has already succeeded.
            {
                let final_state = self.state.lock().await;
                self.reconcile_all_shims(&final_state).await?;
            }

            let _ = self.hooks.run_after_sync().await;
            if self.config.quiet {
                self.metrics.print_summary_quiet();
            } else {
                self.metrics.print_summary();
            }

            // Post-apply health probes: verify any freshly-installed package that declared
            // `@check=…` actually works, so a green install that left a broken service is
            // surfaced immediately (with the pre-sync snapshot available to revert).
            self.run_health_probes(&changes).await;

            // Record a generation of this realized state (+ a frozen manifest copy), then
            // apply the configured generation-retention policy. Non-fatal: a bookkeeping
            // failure must never fail an otherwise-successful sync.
            if let Err(e) = self.record_generation().await {
                warn!("Sync: could not record generation: {}", e);
            }

            // Maintenance: Cleanup the WAL Journal (Bug Fix 10)
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
        info!("Sync: running {} post-install health probe(s)...", probes.len());
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
                "Sync: {} package(s) failed their @check probe: {}.",
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
                Ok(p) => tokio::net::TcpStream::connect(("127.0.0.1", p)).await.is_ok(),
                Err(_) => false,
            };
        }
        let cmd = check.strip_prefix("cmd:").unwrap_or(check);
        crate::app::bisect::run_test(cmd).await
    }

    /// Capture a generation (realized state + a frozen copy of the manifests) after a
    /// successful change, then prune generations per `retention.generations`.
    async fn record_generation(&self) -> Result<()> {
        // A preview never writes history.
        if self.config.dry_run {
            return Ok(());
        }
        let ts = chrono::Utc::now();
        let rfc = ts.to_rfc3339();
        let id = ts.timestamp().to_string();

        // All three histories live beside the state registry, so the same path redirection
        // that makes state hermetic in tests also contains them.
        let base = {
            let state = self.state.lock().await;
            state
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(crate::utils::safe_data_dir)
        };

        // 1. Generation: realized state + a frozen manifest copy.
        let gen_store = crate::app::generation::GenerationStore::new(base.join("generations"));
        {
            let state = self.state.lock().await;
            gen_store
                .capture(&id, &rfc, "", &state, &self.config.groups_dir)
                .await?;
        }
        match gen_store
            .prune(&self.config.retention.generations, ts)
            .await
        {
            Ok(r) if !r.is_empty() => debug!("Sync: pruned {} generation(s).", r.len()),
            Err(e) => warn!("Sync: generation retention prune failed: {}", e),
            _ => {}
        }

        // 2. Manifest archive: independent history of the instructions.
        let archive = crate::app::generation::ManifestArchive::new(base.join("manifest_archive"));
        if let Err(e) = archive.capture(&id, &rfc, &self.config.groups_dir).await {
            warn!("Sync: manifest archive capture failed: {}", e);
        }
        match archive.prune(&self.config.retention.manifests, ts).await {
            Ok(r) if !r.is_empty() => debug!("Sync: pruned {} archived manifest(s).", r.len()),
            Err(e) => warn!("Sync: manifest retention prune failed: {}", e),
            _ => {}
        }

        // 3. Filesystem snapshots: prune LiNix-owned ones per their own policy.
        match self
            .snapshot_manager
            .prune_with_policy(&self.config.retention.snapshots, ts, false)
            .await
        {
            Ok(r) if !r.is_empty() => debug!("Sync: pruned {} snapshot(s).", r.len()),
            Err(e) => warn!("Sync: snapshot retention prune failed: {}", e),
            _ => {}
        }

        Ok(())
    }

    /// Internal orchestrator for the DAG execution.
    async fn execute_transaction(
        &self,
        changes: &SyncChanges,
        state: &mut StateRegistry,
    ) -> Result<()> {
        // Honor the user-configured parallelism (`max_parallel`) for the install/remove
        // engine. Previously this hardcoded `patient()` (max_concurrent = 4), so the
        // documented `max_parallel` knob only affected `search`. Floor at 1.
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

                    let lockable_backends = ["web", "github", "appimage"];
                    if lockable_backends.contains(&spec.backend.as_str())
                        && !spec.options.contains_key("sha256")
                        && self.config.auto_lock_checksums
                    {
                        self.attempt_auto_lock(spec).await;
                    }
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

    /// Extended auto-lock logic (Bug Fix 7: AppImage support).
    async fn attempt_auto_lock(&self, spec: &PackageSpec) {
        if let Some(backend_cap) = self.registry.get(&spec.backend) {
            if let Some(queryable) = backend_cap.as_queryable() {
                if let Ok(Some(pkg)) = queryable.info(&spec.name).await {
                    let path_key = match spec.backend.as_str() {
                        "github" => "install_path",
                        "appimage" => "local_path",
                        _ => "local_path",
                    };

                    if let Some(local_path) = pkg.properties.get(path_key) {
                        let path = std::path::Path::new(local_path).to_path_buf();
                        let hash_res =
                            tokio::task::spawn_blocking(move || generate_checksum(&path)).await;

                        if let Ok(Ok(hash)) = hash_res {
                            info!("Auto-Lock: Generated checksum for {}: {}", spec.name, hash);
                            let mut new_options = spec.options.clone();
                            new_options.insert("sha256".into(), hash);
                            let opt_parts: Vec<String> = new_options
                                .iter()
                                .filter(|(k, _)| !k.starts_with("__"))
                                .map(|(k, v)| format!("{}={}", k, v))
                                .collect();
                            let new_spec_str =
                                format!("{}:{}@{}", spec.backend, spec.name, opt_parts.join(","));
                            let _ = self
                                .manifest_engine
                                .update_package(&spec.name, &new_spec_str)
                                .await;
                        }
                    }
                }
            }
        }
    }

    /// Feature 4/6: High-performance parallel shim reconciliation.
    async fn reconcile_all_shims(&self, state: &StateRegistry) -> Result<()> {
        let shim_mgr = Arc::new(ShimManager::new().await?);
        let mut worker_set = JoinSet::new();

        debug!(
            "Sync: Initiating parallel shim audit for {} packages.",
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
                error!("Sync: Shim worker task panicked: {}", e);
            } else if let Ok(Err(e)) = res {
                warn!("Sync: Non-critical shim reconciliation failure: {}", e);
            }
        }

        Ok(())
    }

    /// System-wide self-healing logic. Resolves incomplete WAL records.
    ///
    /// Resolves Critical Path Failure: Correctly updates Journal entry status
    /// after remediation to ensure system reports a clean state.
    pub async fn heal(&self) -> Result<()> {
        let incomplete_actions = {
            let j = self.journal.lock().await;
            j.get_incomplete_actions()
        };

        if incomplete_actions.is_empty() {
            info!("Heal: System consistency is already verified via WAL.");
            return Ok(());
        }

        warn!(
            "Heal: Resolving {} interrupted system modifications.",
            incomplete_actions.len()
        );
        for entry in incomplete_actions {
            let (backend, package, is_install) = match &entry.action {
                crate::core::journal::JournalAction::Install(spec) => {
                    (spec.backend.clone(), spec.name.clone(), true)
                }
                crate::core::journal::JournalAction::Remove { name, backend } => {
                    (backend.clone(), name.clone(), false)
                }
            };

            if let Some(backend_cap) = self.registry.get(&backend) {
                if let Some(handler) = backend_cap.as_installable() {
                    let sudo = backend_cap.sudo_for_write();
                    let remediation_res = if is_install {
                        // Re-attempting installation sequence
                        let _ = handler.remove(std::slice::from_ref(&package), sudo).await;
                        if let crate::core::journal::JournalAction::Install(spec) = &entry.action {
                            handler.install(std::slice::from_ref(spec), sudo).await
                        } else {
                            Ok(())
                        }
                    } else {
                        // Re-attempting removal
                        handler.remove(std::slice::from_ref(&package), sudo).await
                    };

                    // Logic Fix: Update the WAL record once physically resolved
                    if remediation_res.is_ok() {
                        let mut j = self.journal.lock().await;
                        let _ = j.record_success(&entry.id, std::collections::HashMap::new());
                        debug!(
                            "Heal: Task {} successfully resolved and marked in WAL.",
                            entry.id
                        );
                    } else {
                        error!(
                            "Heal: Failed to resolve task {}: {:?}",
                            entry.id,
                            remediation_res.err()
                        );
                    }
                }
            }
        }

        // Finalize the WAL maintenance
        let mut j = self.journal.lock().await;
        let _ = j.cleanup();
        Ok(())
    }
}
