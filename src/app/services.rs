use crate::app::scheduler::notify::NotificationManager;
use crate::app::scheduler::SchedulerManager;
use crate::app::{
    diagnostics::FailureDiagnosticEngine, EphemeralShell, LuaHooks, MetricsCollector, Adopter,
    ProfileManager, ShimManager, UndoManager,
};
use crate::backends::{create_default_registry, BackendRegistry};
use crate::config::Config;
use crate::core::{
    CommandExecutor, Error, Journal, PackageCache, Result, SnapshotManager, StateRegistry,
};
use crate::utils::progress::{create_progress_reporter, ProgressReporter};

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::debug;

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

pub struct AppServices {
    pub adopter: Adopter,
    pub shell: EphemeralShell,
    pub shim_manager: ShimManager,
    pub undo_manager: UndoManager,
    pub profile_manager: ProfileManager,
    pub diagnostics: Arc<FailureDiagnosticEngine>,
}

impl AppServices {
    pub async fn new(app: &crate::App) -> Result<Self> {
        debug!("assembling services");

        let shim_manager = ShimManager::with_bin_dir(app.config.bin_dir.clone()).await?;

        Ok(Self {
            adopter: Adopter::new(app.registry.clone(), app.state.clone(), &app.config),
            shell: EphemeralShell::new(
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
            undo_manager: UndoManager::new(app.snapshot_manager.clone(), app.state.clone()),
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
    pub async fn from_config(config: Config) -> Result<Self> {
        debug!("assembling services");

        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let hooks = Arc::new(LuaHooks::new(&config)?);

        let registry =
            Arc::new(create_default_registry(executor.duplicate(), &config, hooks.clone()).await);
        let progress = create_progress_reporter(config.show_progress);

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
