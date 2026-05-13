use crate::app::{LuaHooks, MetricsCollector, ShimManager};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::config::manifest::ManifestEngine;
use crate::core::{
    CommandExecutor, Result, Error, Transaction, StateRegistry, 
    Journal, ActionStatus, GraphAction, SnapshotManager
};
use crate::utils::progress::ProgressReporter;
use crate::core::security::generate_checksum;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn, debug, error};

pub mod planner;
pub mod resolver;

pub use self::planner::{ChangePlanner, SyncChanges, PlanAction};
pub use self::resolver::StateResolver;

/// The primary orchestrator for system synchronization using DAG-based execution.
/// Hardened for Version 3.4.0 with ManifestEngine integration and Sandbox Shim reconciliation.
pub struct SyncEngine<'a> {
    config: &'a Config,
    registry: Arc<BackendRegistry>,
    executor: CommandExecutor,
    metrics: MetricsCollector,
    progress: Arc<dyn ProgressReporter>,
    hooks: Arc<LuaHooks>,
    snapshot_manager: Arc<SnapshotManager>,
    journal: Arc<Mutex<Journal>>,
    manifest_engine: ManifestEngine,
}

impl<'a> SyncEngine<'a> {
    pub fn new(
        config: &'a Config, 
        registry: Arc<BackendRegistry>, 
        executor: CommandExecutor, 
        metrics: MetricsCollector, 
        progress: Arc<dyn ProgressReporter>, 
        hooks: Arc<LuaHooks>,
        snapshot_manager: Arc<SnapshotManager>,
        journal: Arc<Mutex<Journal>>
    ) -> Self {
        Self { 
            config, registry, executor, metrics, 
            progress, hooks, snapshot_manager, journal,
            manifest_engine: ManifestEngine::new(&config.groups_dir),
        }
    }

    /// Primary entry point for a system sync.
    pub async fn sync(&self) -> Result<()> {
        let _heartbeat = self.executor.start_sudo_keepalive().await;
        let _ = self.hooks.run_before_sync().await;
        
        let mut state = StateRegistry::load()?;
        let resolver = StateResolver::new(self.config, self.registry.clone());
        let planner = ChangePlanner::new(self.registry.clone(), &state);

        // 1. Resolve & Plan
        let desired = resolver.resolve_desired_state().await?;
        
        // Point 9: Include cleanup policy in planning
        let mut changes = planner.plan(&desired).await?;

        if changes.is_empty() {
            info!("Success: System is consistent with declarative manifests.");
            return Ok(());
        }

        // 2. TUI / Confirmation handled in main.rs, execute transaction here
        let _snapshot = self.snapshot_manager.auto_snapshot("pre_sync").await?;

        // 3. Execution
        let result = self.execute_transaction(&changes, &mut state).await;

        if result.is_ok() {
            state.save()?;
            
            // Point 4: Declarative Shim Reconciliation
            // After sync, ensure shims exist for all managed packages with @sandbox=true
            self.reconcile_all_shims(&state).await?;

            let _ = self.hooks.run_after_sync().await;
            self.metrics.print_summary();
            
            let mut j = self.journal.lock().await;
            let _ = j.cleanup();
        }
        
        result
    }

    async fn execute_transaction(&self, changes: &SyncChanges, state: &mut StateRegistry) -> Result<()> {
        let mut tx = Transaction::new(
            changes.graph.clone(), 
            self.registry.clone(),
            self.journal.clone()
        );

        let pb = self.progress.spinner("Applying parallel system transformations...");
        let result = tx.execute().await;
        pb.finish();
        
        if result.is_ok() {
            // Update the local state registry to reflect success
            for idx in changes.graph.node_indices() {
                match &changes.graph[idx] {
                    GraphAction::Install(spec) => {
                        state.add(&spec.backend, &spec.name, None, spec.options.clone());
                        
                        // Point 1: Checksum Auto-Locking via ManifestEngine
                        if (spec.backend == "web" || spec.backend == "github") && !spec.options.contains_key("sha256") {
                            self.attempt_auto_lock(spec).await;
                        }
                    },
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

    /// Point 1: Automatically calculates and locks checksums in the manifest.
    async fn attempt_auto_lock(&self, spec: &crate::core::PackageSpec) {
        if let Some(backend) = self.registry.get(&spec.backend) {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(Some(pkg)) = queryable.info(&spec.name).await {
                    let path_key = if spec.backend == "github" { "install_path" } else { "local_path" };
                    if let Some(local_path) = pkg.properties.get(path_key) {
                        let path = std::path::Path::new(local_path);
                        if let Ok(hash) = generate_checksum(path) {
                            info!("Auto-Lock: Generated SHA256 for {}: {}", spec.name, hash);
                            
                            // Construct the new line with the locked hash
                            let mut new_options = spec.options.clone();
                            new_options.insert("sha256".into(), hash);
                            
                            let mut opt_parts = Vec::new();
                            for (k, v) in new_options {
                                opt_parts.push(format!("{}={}", k, v));
                            }
                            let new_spec_str = format!("{}:{}@{}", spec.backend, spec.name, opt_parts.join(","));
                            
                            // Surgical update preserving comments
                            let _ = self.manifest_engine.update_package(&spec.name, &new_spec_str);
                        }
                    }
                }
            }
        }
    }

    /// Point 4: Ensures shims exist for sandboxed applications.
    async fn reconcile_all_shims(&self, state: &StateRegistry) -> Result<()> {
        let shim_mgr = ShimManager::new()?;
        for pkg in &state.packages {
            let needs_shim = pkg.options.get("sandbox") == Some(&"true".to_string()) || 
                             pkg.options.get("shim") == Some(&"true".to_string());
            
            shim_mgr.reconcile_shims(&pkg.name, needs_shim).await?;
        }
        Ok(())
    }

    /// Point 8: Hardened WAL Recovery.
    pub async fn heal(&self) -> Result<()> {
        let incomplete_actions = {
            let j = self.journal.lock().await;
            j.get_incomplete_actions()
        };

        if incomplete_actions.is_empty() {
            info!("Self-Healing: No inconsistent states detected.");
            return Ok(());
        }

        warn!("Self-Healing: Restoring system consistency for {} tasks...", incomplete_actions.len());

        for entry in incomplete_actions {
            if let Some(backend) = self.registry.get(&entry.backend) {
                if let Some(handler) = backend.as_installable() {
                    if entry.is_install {
                        info!("Self-Healing: Cleaning and re-applying install for {}", entry.package);
                        let _ = handler.remove(&[entry.package.clone()], true).await;
                        let spec = PackageSpec {
                            name: entry.package.clone(),
                            backend: entry.backend.clone(),
                            options: std::collections::HashMap::new(),
                            requires: vec![],
                        };
                        let _ = handler.install(&[spec], true).await;
                    } else {
                        info!("Self-Healing: Re-applying removal for {}", entry.package);
                        let _ = handler.remove(&[entry.package.clone()], true).await;
                    }
                }
            }
        }

        let mut j = self.journal.lock().await;
        j.cleanup()?;
        info!("Self-Healing: Consistency restored.");
        Ok(())
    }
}