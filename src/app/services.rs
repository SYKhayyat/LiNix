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
    pub config: Config,
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
/// Phase 4.1 Refactor: Decouples logic from the core state, allowing 
/// for easier testing and specialized execution contexts.
pub struct AppServices {
    pub migrator: Migrator<'static>,
    pub teleporter: Teleporter<'static>,
    pub shell: GhostShell<'static>,
    pub shim_manager: ShimManager,
    pub undo_manager: UndoManager<'static>,
    pub profile_manager: ProfileManager<'static>,
}

impl AppServices {
    /// Creates a new service container asynchronously.
    /// 
    /// This accepts a leaked static reference to the App context to satisfy 
    /// lifetime requirements of various sub-orchestrators while ensuring 
    /// zero-cost access to core components.
    pub async fn new(app: &'static crate::App) -> Result<Self> {
        // Phase 3.2: ShimManager initialization is now async and must be awaited.
        let shim_manager = ShimManager::new().await?;

        Ok(Self {
            migrator: Migrator::new(app),
            teleporter: Teleporter::new(app),
            shell: GhostShell::new(app),
            shim_manager,
            undo_manager: UndoManager::new(app),
            profile_manager: ProfileManager::new(app),
        })
    }
}

impl AppCore {
    /// High-performance asynchronous initializer for the shared application state.
    /// 
    /// Wraps blocking filesystem operations (like loading the registry) in 
    /// `spawn_blocking` to ensure the tokio executor remains responsive.
    pub async fn from_config(config: Config) -> Result<Self> {
        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let hooks = Arc::new(LuaHooks::new(&config)?);
        
        let registry = Arc::new(create_default_registry(executor.duplicate(), &config, hooks.clone()).await);
        let progress = create_progress_reporter(config.show_progress);
        
        // Phase 3.2: Wrap synchronous StateRegistry load in blocking task
        let state_val = tokio::task::spawn_blocking(StateRegistry::load)
            .await
            .map_err(|e| Error::Other(e.to_string()))??;
        
        let state = Arc::new(Mutex::new(state_val));
        let snapshot_manager = Arc::new(SnapshotManager::new(executor.duplicate()).await);
        let journal = Arc::new(Mutex::new(Journal::new()?));
        let bridge = Arc::new(DependencyBridge::new());

        Ok(Self {
            config,
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