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
/// Hardened for Phase 3.1 & 3.2: Async-compliant Manifest and Shim engines.
pub struct SyncEngine<'a> {
    pub config: &'a Config,
    pub registry: Arc<BackendRegistry>,
    pub executor: CommandExecutor,
    pub metrics: MetricsCollector,
    pub progress: Arc<dyn ProgressReporter>,
    pub hooks: Arc<LuaHooks>,
    pub snapshot_manager: Arc<SnapshotManager>,
    pub journal: Arc<Mutex<Journal>>,
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
            manifest_engine,
        }
    }

    /// Primary entry point for system synchronization.
    /// Accepts a pre-calculated plan to respect user filtering in the TUI.
    pub async fn sync(&self, changes: SyncChanges) -> Result<()> {
        let _heartbeat = self.executor.start_sudo_keepalive().await;
        let _ = self.hooks.run_before_sync().await;
        
        // StateRegistry::load is blocking, wrap in spawn_blocking
        let mut state = tokio::task::spawn_blocking(StateRegistry::load)
            .await
            .map_err(|e| crate::core::Error::Other(e.to_string()))??;

        if changes.is_empty() {
            info!("Success: System is consistent with declarative manifests.");
            return Ok(());
        }

        let _snapshot = self.snapshot_manager.auto_snapshot("pre_sync").await?;

        let result = self.execute_transaction(&changes, &mut state).await;

        if result.is_ok() {
            // StateRegistry::save is blocking, wrap in spawn_blocking
            tokio::task::spawn_blocking(move || state.save())
                .await
                .map_err(|e| crate::core::Error::Other(e.to_string()))??;

            // Phase 3.1: State is now persistent, re-load for shim reconciliation
            let final_state = tokio::task::spawn_blocking(StateRegistry::load)
                .await
                .map_err(|e| crate::core::Error::Other(e.to_string()))??;

            self.reconcile_all_shims(&final_state).await?;
            let _ = self.hooks.run_after_sync().await;
            self.metrics.print_summary();
            
            let mut j = self.journal.lock().await;
            let _ = j.cleanup();
        }
        result
    }

    async fn execute_transaction(&self, changes: &SyncChanges, state: &mut StateRegistry) -> Result<()> {
        let tx_config = TransactionConfig {
            max_concurrent: self.config.max_parallel,
            node_timeout: std::time::Duration::from_secs(300),
            total_timeout: std::time::Duration::from_secs(3600),
            max_retries: 3,
            initial_backoff: std::time::Duration::from_millis(500),
            max_backoff: std::time::Duration::from_secs(30),
            auto_rollback: true,
        };

        let mut tx = Transaction::with_config(
            changes.graph.clone(),
            self.registry.clone(),
            self.journal.clone(),
            tx_config,
        );

        let pb = self.progress.spinner("Applying parallel system transformations...");
        let result = tx.execute().await;
        pb.finish();

        if result.is_ok() {
            for idx in changes.graph.node_indices() {
                match &changes.graph[idx] {
                    GraphAction::Install(spec) => {
                        state.add(&spec.backend, &spec.name, None, spec.options.clone());
                        // Phase 3.1: Auto-lock checksums for remote resources
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
        }
        result
    }

    async fn attempt_auto_lock(&self, spec: &PackageSpec) {
        if let Some(backend_cap) = self.registry.get(&spec.backend) {
            if let Some(queryable) = backend_cap.as_queryable() {
                if let Ok(Some(pkg)) = queryable.info(&spec.name).await {
                    let path_key = if spec.backend == "github" { "install_path" } else { "local_path" };
                    if let Some(local_path) = pkg.properties.get(path_key) {
                        let path = std::path::Path::new(local_path);
                        
                        // Checksum generation involves blocking file I/O
                        let path_owned = path.to_path_buf();
                        let hash_res = tokio::task::spawn_blocking(move || generate_checksum(&path_owned)).await;
                        
                        if let Ok(Ok(hash)) = hash_res {
                            info!("Auto-Lock: Generated SHA256 for {}: {}", spec.name, hash);
                            let mut new_options = spec.options.clone();
                            new_options.insert("sha256".into(), hash);
                            let opt_parts: Vec<String> = new_options.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
                            let new_spec_str = format!("{}:{}@{}", spec.backend, spec.name, opt_parts.join(","));
                            
                            // Phase 3.1: ManifestEngine update is now async
                            let _ = self.manifest_engine.update_package(&spec.name, &new_spec_str).await;
                        }
                    }
                }
            }
        }
    }

    async fn reconcile_all_shims(&self, state: &StateRegistry) -> Result<()> {
        // Phase 3.2: ShimManager instantiation is now async
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