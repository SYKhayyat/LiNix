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
/// 
/// Functioning as the primary dependency injection container, AppCore 
/// ensures that system state, configuration, and executors are 
/// synchronized across the entire application lifecycle.
#[derive(Clone)]
pub struct AppCore {
    /// Global application configuration.
    pub config: Arc<Config>,
    /// High-speed metadata and search cache.
    pub cache: Arc<PackageCache>,
    /// Registry of all supported package manager backends.
    pub registry: Arc<BackendRegistry>,
    /// Low-level system command and file orchestrator.
    pub executor: CommandExecutor,
    /// Telemetry and transaction performance collector.
    pub metrics: MetricsCollector,
    /// Unified interface for progress bars and spinners.
    pub progress: Arc<dyn ProgressReporter>,
    /// Multi-engine scripting controller (Lua / Rhai).
    pub hooks: Arc<LuaHooks>,
    /// The mission-critical system state registry (Single Source of Truth).
    pub state: Arc<Mutex<StateRegistry>>,
    /// Orchestrator for atomic system snapshots.
    pub snapshot_manager: Arc<SnapshotManager>,
    /// Write-Ahead Log for crash recovery.
    pub journal: Arc<Mutex<Journal>>,
    /// Feature 5: Native background task automation engine.
    pub scheduler: Arc<SchedulerManager>,
    /// Feature 5: System-wide alert and multi-channel notification dispatcher.
    pub notifications: Arc<NotificationManager>,
    /// Modernized Failure Diagnosis Engine.
    pub diagnostics: Arc<FailureDiagnosticEngine>,
}

/// Container for the "Logic Layer" orchestrators.
/// 
/// This struct holds the complex sub-systems that depend on the 
/// AppCore kernel to perform coordinated system modifications.
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
    /// 
    /// # Arguments
    /// * `app` - A reference to the fully initialized application context.
    /// 
    /// Resolves Argument Mismatches: Injects diagnostics into all managers.
    /// Resolves rustdoc warnings: Standard comments inside struct literal.
    pub async fn new(app: &crate::App) -> Result<Self> {
        debug!("AppServices: Assembling logic engines from kernel context.");

        let shim_manager = ShimManager::new().await?;

        Ok(Self {
            // Ingest manual installs into LiNix management
            migrator: Migrator::new(
                app.registry.clone(),
                app.state.clone(),
                &app.config
            ),
            // Cross-backend package movement (Modernized DI)
            teleporter: Teleporter::new(
                app.registry.clone(),
                app.journal.clone(),
                app.state.clone(),
                app.diagnostics.clone(), // DI
                &app.config.groups_dir
            ),
            // Feature 6: Ephemeral sub-shells (Exhaustive 10-argument init)
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
                app.diagnostics.clone(), // 10th argument
            ),
            // Deployment of high-performance binary shims
            shim_manager,
            // System-level time travel and snapshot gallery
            undo_manager: UndoManager::new(
                app.snapshot_manager.clone(),
                app.state.clone(),
                app.executor.clone()
            ),
            // Environment identity switching (Modernized DI)
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
                app.diagnostics.clone(), // DI
            ),
            // Resolves E0609: Reference modernized field 'diagnostics'
            diagnostics: app.diagnostics.clone(), 
        })
    }
}

impl AppCore {
    /// High-performance asynchronous bootstrapper for the LiNix kernel.
    /// 
    /// Initializes all sub-systems, discovers native backends, and loads 
    /// the mission-critical state registry.
    pub async fn from_config(config: Config) -> Result<Self> {
        info!("AppCore: Initializing LiNix v3.6.0 mission-critical kernel.");

        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let hooks = Arc::new(LuaHooks::new(&config)?);
        
        // Discover and catalog all package manager backends
        let registry = Arc::new(create_default_registry(executor.duplicate(), &config, hooks.clone()).await);
        let progress = create_progress_reporter(config.show_progress);
        
        // Load StateRegistry (Offloaded to task for async safety)
        let state_val = tokio::task::spawn_blocking(StateRegistry::load)
            .await
            .map_err(|e| Error::Other(format!("Kernel panic during state load: {}", e)))??;
        let state = Arc::new(Mutex::new(state_val));
        
        // Initialize recovery and safety managers
        let snapshot_manager = Arc::new(SnapshotManager::new(executor.duplicate(), &config).await);
        let journal = Arc::new(Mutex::new(Journal::new()?));
        
        // Initialize 3.6.0 automation and diagnostic sub-systems
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