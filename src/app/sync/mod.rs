use crate::core::{
    Result, PackageSpec, StateRegistry, CommandExecutor, Transaction, 
    GraphAction, SnapshotManager, Journal, TransactionConfig
};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::config::manifest::ManifestEngine;
use crate::app::{LuaHooks, MetricsCollector, ShimManager};
use crate::utils::progress::ProgressReporter;
use crate::core::security::generate_checksum;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

pub mod planner;
pub mod resolver;

pub use self::planner::{ChangePlanner, SyncChanges};
pub use self::resolver::StateResolver;

#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    async fn resolve_desired_state(&self) -> Result<std::collections::HashMap<String, Vec<PackageSpec>>>;
}

#[async_trait::async_trait]
pub trait Planner: Send + Sync {
    async fn plan(&self, desired: &std::collections::HashMap<String, Vec<PackageSpec>>) -> Result<SyncChanges>;
}

/// Primary entry point for system synchronization.
/// Hardened for Phase 4.1: Uses a shared StateRegistry Arc to ensure a single source of truth.
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
}

impl<'a> SyncEngine<'a> {
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
        }
    }

    /// Primary entry point for system synchronization.
    pub async fn sync(&self, changes: SyncChanges) -> Result<()> {
        let _heartbeat = self.executor.start_sudo_keepalive().await;
        let _ = self.hooks.run_before_sync().await;
        
        if changes.is_empty() {
            info!("Success: System is consistent with declarative manifests.");
            return Ok(());
        }

        let _snapshot = self.snapshot_manager.auto_snapshot("pre_sync").await?;

        // Phase 4.1 Fix: Lock the shared state registry directly instead of loading from disk
        let result = {
            let mut state_guard = self.state.lock().await;
            self.execute_transaction(&changes, &mut state_guard).await
        };

        if result.is_ok() {
            // Persist the shared state to disk
            let state_to_save = self.state.lock().await.clone();
            tokio::task::spawn_blocking(move || state_to_save.save())
                .await
                .map_err(|e| crate::core::Error::Other(e.to_string()))??;

            // Perform post-transaction reconciliations using the updated state
            let final_state = self.state.lock().await;
            self.reconcile_all_shims(&final_state).await?;
            
            let _ = self.hooks.run_after_sync().await;
            self.metrics.print_summary();
            
            let mut j = self.journal.lock().await;
            let _ = j.cleanup();
        }
        result
    }

    async fn execute_transaction(&self, changes: &SyncChanges, state: &mut StateRegistry) -> Result<()> {
        let tx_config = TransactionConfig::patient();

        let mut tx = Transaction::with_config(
            changes.graph.clone(),
            self.registry.clone(),
            self.journal.clone(),
            tx_config,
        );

        let pb = self.progress.spinner("Applying parallel system transformations...");
        let results = tx.execute_with_telemetry().await?;
        pb.finish();

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
                    state.add(&spec.backend, &spec.name, None, spec.options.clone());
                    if (spec.backend == "web" || spec.backend == "github") && !spec.options.contains_key("sha256") {
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

    async fn attempt_auto_lock(&self, spec: &PackageSpec) {
        if let Some(backend_cap) = self.registry.get(&spec.backend) {
            if let Some(queryable) = backend_cap.as_queryable() {
                if let Ok(Some(pkg)) = queryable.info(&spec.name).await {
                    let path_key = if spec.backend == "github" { "install_path" } else { "local_path" };
                    if let Some(local_path) = pkg.properties.get(path_key) {
                        let path = std::path::Path::new(local_path);
                        let path_owned = path.to_path_buf();
                        let hash_res = tokio::task::spawn_blocking(move || generate_checksum(&path_owned)).await;
                        
                        if let Ok(Ok(hash)) = hash_res {
                            info!("Auto-Lock: Generated SHA256 for {}: {}", spec.name, hash);
                            let mut new_options = spec.options.clone();
                            new_options.insert("sha256".into(), hash);
                            let opt_parts: Vec<String> = new_options.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
                            let new_spec_str = format!("{}:{}@{}", spec.backend, spec.name, opt_parts.join(","));
                            let _ = self.manifest_engine.update_package(&spec.name, &new_spec_str).await;
                        }
                    }
                }
            }
        }
    }

    async fn reconcile_all_shims(&self, state: &StateRegistry) -> Result<()> {
        let shim_mgr = ShimManager::new().await?;
        for pkg in &state.packages {
            let needs_shim = pkg.options.get("sandbox") == Some(&"true".to_string())
                || pkg.options.get("shim") == Some(&"true".to_string());
            shim_mgr.reconcile_shims(&pkg.name, needs_shim).await?;
        }
        Ok(())
    }

    pub async fn heal(&self) -> Result<()> {
        let incomplete_actions = {
            let j = self.journal.lock().await;
            j.get_incomplete_actions()
        };
        
        if incomplete_actions.is_empty() {
            info!("Self-Healing: No inconsistent states detected.");
            return Ok(());
        }
        
        warn!("Self-Healing: Restoring system consistency...");
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
                        let spec = PackageSpec {
                            name: package.clone(),
                            backend: backend.clone(),
                            options: std::collections::HashMap::new(),
                            requires: vec![],
                        };
                        let _ = handler.install(&[spec], sudo).await;
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