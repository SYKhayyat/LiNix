// src/app/services.rs

use crate::app::{
    Migrator, Teleporter, GhostShell, ShimManager, UndoManager, ProfileManager,
    LuaHooks, MetricsCollector, diagnostics::FailureDiagnosticEngine
};
use crate::config::Config;
use crate::core::{
    CommandExecutor, PackageCache, StateRegistry, Journal,
    SnapshotManager, Result, Error
};
use crate::backends::{create_default_registry, BackendRegistry};
use crate::utils::progress::{create_progress_reporter, ProgressReporter};
use crate::app::scheduler::SchedulerManager;
use crate::app::scheduler::notify::NotificationManager;

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

/// Core kernel dependencies shared across all high-level logic engines.
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
    pub scheduler: Arc<SchedulerManager>,
    pub notifications: Arc<NotificationManager>,
    pub diagnostics: Arc<FailureDiagnosticEngine>,
}

/// Container for the "Logic Layer" orchestrators.
pub struct AppServices {
    pub migrator: Migrator,
    pub teleporter: Teleporter,
    pub shell: GhostShell,
    pub shim_manager: ShimManager,
    pub undo_manager: UndoManager,
    pub profile_manager: ProfileManager,
    pub diagnostics: Arc<FailureDiagnosticEngine>,
}

impl AppServices {
    /// Initializes the logic layer orchestrators by destructuring the App kernel.
    pub async fn new(app: &crate::App) -> Result<Self> {
        debug!("AppServices: Assembling logic engines from kernel context.");

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
                app.diagnostics.clone(),
                &app.config.groups_dir
            ),
            shell: GhostShell::new(
                app.registry.clone(),
                app.state.clone(),
                app.config.clone(),
                app.executor.duplicate(),
                app.metrics.clone(),
                app.progress.clone(),
                app.hooks.clone(),
                app.snapshot_manager.clone(),
                app.journal.clone(),
                app.diagnostics.clone(),
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
                app.config.clone(),
                app.diagnostics.clone(),
            ),
            diagnostics: app.diagnostics.clone(),
        })
    }
}

impl AppCore {
    /// High-performance asynchronous bootstrapper for the LiNix kernel.
    pub async fn from_config(config: Config) -> Result<Self> {
        info!("AppCore: Initializing LiNix v3.6.0 mission-critical kernel.");

        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let hooks = Arc::new(LuaHooks::new(&config)?);

        let registry = Arc::new(create_default_registry(executor.duplicate(), &config, hooks.clone()).await);
        let progress = create_progress_reporter(config.show_progress);

        // Load StateRegistry using the new `load_default()` method.
        let state_val = tokio::task::spawn_blocking(StateRegistry::load_default)
            .await
            .map_err(|e| Error::Other(format!("Kernel panic during state load: {}", e)))??;
        let state = Arc::new(Mutex::new(state_val));

        let snapshot_manager = Arc::new(SnapshotManager::new(executor.duplicate(), &config).await);
        let journal = Arc::new(Mutex::new(Journal::new()?));

        let diagnostics = Arc::new(FailureDiagnosticEngine::init(&config).await);
        let scheduler = Arc::new(SchedulerManager::new()?);
        let config_arc = Arc::new(config);
        let notifications = Arc::new(NotificationManager::new(config_arc.clone()));

        Ok(Self {
            config: config_arc,
            cache: Arc::new(PackageCache::new()),
            registry,
            executor,
            metrics: MetricsCollector::new(),
            progress,
            hooks,
            state,
            snapshot_manager,
            journal,
            diagnostics,
            scheduler,
            notifications,
        })
    }
}