use crate::app::{
    Migrator, Teleporter, GhostShell, ShimManager, UndoManager, ProfileManager,
    LuaHooks, MetricsCollector
};
use crate::config::Config;
use crate::core::{CommandExecutor, PackageCache, StateRegistry, Journal, SnapshotManager, Result, Error};
use crate::backends::{create_default_registry, BackendRegistry};
use crate::utils::progress::{create_progress_reporter, ProgressReporter};
use crate::app::bridge::DependencyBridge;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Core dependencies that are shared across all high-level services.
/// 
/// Hardened for Version 3.5.0: Functions as the primary dependency injection 
/// container for the LiNix kernel.
#[derive(Clone)]
pub struct AppCore {
    pub config: Arc<Config>,
    pub cache: Arc<PackageCache>,
    pub registry: Arc<BackendRegistry>,
    pub executor: CommandExecutor,
    pub metrics: MetricsCollector,
    pub progress: Arc<dyn ProgressReporter>,
    pub hooks: Arc<LuaHooks>,
    pub state: Arc<Mutex<StateRegistry>>,
    pub snapshot_manager: Arc<SnapshotManager>,
    pub journal: Arc<Mutex<Journal>>,
    pub bridge: Arc<DependencyBridge>,
}

/// Container for all high-level application orchestrators.
/// 
/// Phase 4.1 Refactor: Decoupled from monolithic lifetimed references.
/// These services now use internal Arc-based sharing for safety and performance.
pub struct AppServices {
    pub migrator: Migrator,
    pub teleporter: Teleporter,
    pub shell: GhostShell,
    pub shim_manager: ShimManager,
    pub undo_manager: UndoManager,
    pub profile_manager: ProfileManager,
}

impl AppServices {
    /// Creates a new service container asynchronously.
    /// 
    /// Phase 4.1: Destructures the App kernel to provide specific dependencies 
    /// to each orchestrator constructor.
    pub async fn new(app: &'static crate::App) -> Result<Self> {
        let shim_manager = ShimManager::new().await?;

        Ok(Self {
            migrator: Migrator::new(
                app.registry.clone(),
                app.state.clone(),
                &app.config
            ),
            teleporter: Teleporter::new(
                app.registry.clone(),
                app.journal.clone(),
                app.state.clone(),
                &app.config.groups_dir
            ),
            shell: GhostShell::new(
                app.registry.clone(),
                app.config.clone()
            ),
            shim_manager,
            undo_manager: UndoManager::new(
                app.snapshot_manager.clone(),
                app.state.clone(),
                app.executor.clone()
            ),
            profile_manager: ProfileManager::new(
                app.registry.clone(),
                app.executor.clone(),
                app.metrics.clone(),
                app.progress.clone(),
                app.hooks.clone(),
                app.snapshot_manager.clone(),
                app.journal.clone(),
                app.state.clone(),
                app.config.clone()
            ),
        })
    }
}

impl AppCore {
    /// High-performance asynchronous initializer for the shared application state.
    pub async fn from_config(config: Config) -> Result<Self> {
        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let hooks = Arc::new(LuaHooks::new(&config)?);
        
        let registry = Arc::new(create_default_registry(executor.duplicate(), &config, hooks.clone()).await);
        let progress = create_progress_reporter(config.show_progress);
        
        let state_val = tokio::task::spawn_blocking(StateRegistry::load)
            .await
            .map_err(|e| Error::Other(e.to_string()))??;
        
        let state = Arc::new(Mutex::new(state_val));
        let snapshot_manager = Arc::new(SnapshotManager::new(executor.duplicate(), &config).await);
        let journal = Arc::new(Mutex::new(Journal::new()?));
        let bridge = Arc::new(DependencyBridge::new());

        Ok(Self {
            config: Arc::new(config),
            cache: Arc::new(PackageCache::new()),
            registry,
            executor,
            metrics: MetricsCollector::new(),
            progress,
            hooks,
            state,
            snapshot_manager,
            journal,
            bridge,
        })
    }
}