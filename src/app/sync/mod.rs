use crate::core::{
    Result, PackageSpec, StateRegistry, CommandExecutor, Transaction, 
    GraphAction, SnapshotManager, Journal, TransactionConfig, Error
};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::config::manifest::ManifestEngine;
use crate::app::{LuaHooks, MetricsCollector, ShimManager};
use crate::app::diagnostics::FailureDiagnosticEngine; // Modernized: DI Import
use crate::utils::progress::ProgressReporter;
use crate::core::security::generate_checksum;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing::{info, warn, error, debug, instrument}; // Modernized: Pruned unused 'trace'

pub mod planner;
pub mod resolver;

pub use self::planner::{ChangePlanner, SyncChanges, ScopedFilter};
pub use self::resolver::StateResolver;

/// Primary entry point for parallel system synchronization.
/// 
/// The SyncEngine manages the full lifecycle of a system transformation:
/// 1. Pre-flight checks and hooks.
/// 2. Atomic safety (Snapshotting).
/// 3. Parallel Execution (DAG-based Transaction with Diagnostic support).
/// 4. State Consolidation (Registry updates).
/// 5. Post-transaction maintenance (Parallel Shim reconciliation & Pruning).
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
    /// Modernized v3.6.0: Injected engine for autonomous failure analysis.
    pub diagnostics: Arc<FailureDiagnosticEngine>,
}

impl<'a> SyncEngine<'a> {
    /// Initializes the engine with the full kernel context.
    /// 
    /// # Arguments
    /// Modernized: Now accepts the diagnostic engine to support DI for Transactions.
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
            info!("Sync: OS state is already consistent with declarative manifests.");
            return Ok(());
        }

        // 1. Capture atomic safety snapshot (Feature 2)
        let _snapshot = self.snapshot_manager.auto_snapshot("pre_sync").await?;

        // 2. Execute Transaction
        // We lock the shared state registry directly to ensure it acts as the 
        // Single Source of Truth during parallel execution.
        let result = {
            let mut state_guard = self.state.lock().await;
            self.execute_transaction(&changes, &mut state_guard).await
        };

        // 3. Post-Transaction Consolidation
        if result.is_ok() {
            debug!("Sync: Finalizing transaction state and persistence.");

            // Persist the state registry (Async-wrapped blocking IO)
            let state_to_save = self.state.lock().await.clone();
            tokio::task::spawn_blocking(move || state_to_save.save())
                .await
                .map_err(|e| Error::Other(format!("Kernel panic during state persistence: {}", e)))??;

            // Feature 4/6: Parallelized Shim Reconciliation (A+ High Performance)
            let final_state = self.state.lock().await;
            self.reconcile_all_shims(&final_state).await?;
            
            let _ = self.hooks.run_after_sync().await;
            self.metrics.print_summary();
            
            // Maintenance: Cleanup the WAL Journal (Bug Fix 10)
            let mut j = self.journal.lock().await;
            let _ = j.cleanup();
        }

        result
    }

    /// Internal orchestrator for the DAG execution.
    /// 
    /// Resolves E0061: Passes the injected diagnostic engine into the transaction.
    async fn execute_transaction(&self, changes: &SyncChanges, state: &mut StateRegistry) -> Result<()> {
        let tx_config = TransactionConfig::patient();

        // Modernized v3.6.0: Transaction initialized with DI diagnostics
        let mut tx = Transaction::with_config(
            changes.graph.clone(),
            self.registry.clone(),
            self.journal.clone(),
            self.diagnostics.clone(), // Correctly providing the 4th argument
            tx_config,
        );

        let pb = self.progress.spinner("Applying parallel system modifications...");
        let results = tx.execute_with_telemetry().await?;
        pb.finish();

        // Identify if we are in an ephemeral shell session (Feature 6)
        let session_active = state.active_session_id.is_some();

        for res in results {
            self.metrics.record_operation(
                &res.package_name, &res.backend_name, res.start_time,
                res.result.is_ok(), res.result.err().map(|e| e.to_string()),
                res.attempt, res.bytes_downloaded,
            );
        }

        for idx in changes.graph.node_indices() {
            match &changes.graph[idx] {
                GraphAction::Install(spec) => {
                    let source = spec.options.get("__source").cloned();
                    state.add(
                        &spec.backend, &spec.name, None, spec.options.clone(), 
                        source, session_active
                    );

                    // Feature 7: Auto-locking unauthenticated resources (Bug Fix 7)
                    let lockable_backends = ["web", "github", "appimage"];
                    if lockable_backends.contains(&spec.backend.as_str()) && !spec.options.contains_key("sha256") {
                        if self.config.auto_lock_checksums {
                            self.attempt_auto_lock(spec).await;
                        }
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
                        _ => "local_path"
                    };

                    if let Some(local_path) = pkg.properties.get(path_key) {
                        let path = std::path::Path::new(local_path).to_path_buf();
                        let hash_res = tokio::task::spawn_blocking(move || generate_checksum(&path)).await;
                        
                        if let Ok(Ok(hash)) = hash_res {
                            info!("Auto-Lock: Generated checksum for {}: {}", spec.name, hash);
                            let mut new_options = spec.options.clone();
                            new_options.insert("sha256".into(), hash);
                            let opt_parts: Vec<String> = new_options.iter()
                                .filter(|(k, _)| !k.starts_with("__"))
                                .map(|(k, v)| format!("{}={}", k, v)).collect();
                            let new_spec_str = format!("{}:{}@{}", spec.backend, spec.name, opt_parts.join(","));
                            let _ = self.manifest_engine.update_package(&spec.name, &new_spec_str).await;
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

        debug!("Sync: Initiating parallel shim audit for {} closure packages.", state.packages.len());

        for pkg in &state.packages {
            let mgr = shim_mgr.clone();
            let pkg_name = pkg.name.clone();
            let needs_shim = pkg.options.get("sandbox") == Some(&"true".to_string())
                || pkg.options.get("shim") == Some(&"true".to_string());

            worker_set.spawn(async move {
                mgr.reconcile_shims(&pkg_name, needs_shim).await
            });
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

    /// Mission-critical healing logic for unresolved journal records.
    pub async fn heal(&self) -> Result<()> {
        let incomplete_actions = {
            let j = self.journal.lock().await;
            j.get_incomplete_actions()
        };
        
        if incomplete_actions.is_empty() {
            info!("Heal: System consistency is already verified via WAL.");
            return Ok(());
        }
        
        warn!("Heal: Resolving {} interrupted system modifications.", incomplete_actions.len());
        for entry in incomplete_actions {
            let (backend, package, is_install) = match &entry.action {
                crate::core::journal::JournalAction::Install(spec) => (spec.backend.clone(), spec.name.clone(), true),
                crate::core::journal::JournalAction::Remove { name, backend } => (backend.clone(), name.clone(), false),
            };
            
            if let Some(backend_cap) = self.registry.get(&backend) {
                if let Some(handler) = backend_cap.as_installable() {
                    let sudo = backend_cap.needs_root();
                    if is_install {
                        let _ = handler.remove(&[package.clone()], sudo).await;
                        if let crate::core::journal::JournalAction::Install(spec) = &entry.action {
                            let _ = handler.install(&[spec.clone()], sudo).await;
                        }
                    } else {
                        let _ = handler.remove(&[package.clone()], sudo).await;
                    }
                }
            }
        }
        
        let mut j = self.journal.lock().await;
        let _ = j.cleanup();
        Ok(())
    }
}