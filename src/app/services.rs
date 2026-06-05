use crate::app::{
    Migrator, Teleporter, GhostShell, ShimManager, UndoManager, ProfileManager,
    LuaHooks, MetricsCollector
};
use crate::config::Config;
use crate::core::{CommandExecutor, PackageCache, StateRegistry, Journal, SnapshotManager};
use crate::backends::BackendRegistry;
use crate::utils::progress::ProgressReporter;
use crate::app::bridge::DependencyBridge;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Core dependencies that are shared across all high-level services.
/// Hardened for Version 3.5.0 with derived Clone support for the executor.
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

/// Container for all high-level application services.
/// This decouples orchestrator lifetimes from the core application kernel.
pub struct AppServices {
    pub migrator: Migrator<'static>,
    pub teleporter: Teleporter<'static>,
    pub shell: GhostShell<'static>,
    pub shim_manager: ShimManager,
    pub undo_manager: UndoManager<'static>,
    pub profile_manager: ProfileManager<'static>,
}

impl AppServices {
    /// Creates a new service container.
    /// This accepts a leaked static reference to the App context.
    /// This is a safe pattern for the main application entry point to ensure
    /// that services can always reference the core config and state.
    pub fn new(app: &'static crate::App) -> Self {
        Self {
            migrator: Migrator::new(app),
            teleporter: Teleporter::new(app),
            shell: GhostShell::new(app),
            shim_manager: ShimManager::new().expect("Failed to initialize ShimManager"),
            undo_manager: UndoManager::new(app),
            profile_manager: ProfileManager::new(app),
        }
    }
}

impl AppCore {
    /// Initializer for the shared application state.
    /// Uses duplicate() to ensure the executor is correctly shared.
    pub async fn from_config(config: Config) -> crate::core::Result<Self> {
        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let hooks = Arc::new(LuaHooks::new(&config)?);
        let registry = Arc::new(crate::backends::create_default_registry(executor.duplicate(), &config, hooks.clone()).await);
        let progress = crate::utils::progress::create_progress_reporter(config.show_progress);
        
        let state = Arc::new(Mutex::new(StateRegistry::load()?));
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