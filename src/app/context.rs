use crate::backends::{create_default_registry, BackendRegistry};
use crate::config::Config;
use crate::core::{
    CommandExecutor, PackageCache, Result, Error, 
    Package, StateRegistry, PackageSpec, Validator, SnapshotManager, Journal
};
use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::migrate::Migrator;
use crate::app::teleport::Teleporter;
use crate::app::shell::GhostShell;
use crate::app::profile::ProfileManager;
use crate::app::shim_manager::ShimManager;
use crate::app::undo::UndoManager;
use crate::app::run::Runner;
use crate::app::scheduler::SchedulerManager;
use crate::app::scheduler::notify::NotificationManager;
use crate::app::sync::resolver::StateResolver;
use crate::utils::progress::{create_progress_reporter, ProgressReporter};

use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::{VecDeque, HashSet};
use tracing::{info, debug, instrument}; // Modernized: Removed unused 'trace', 'warn', 'error'
use super::{LuaHooks, MetricsCollector, UniversalSearch};

/// The unified Application Context for LiNix v3.6.0.
/// 
/// Functioning as a high-performance Service Provider and Dependency Injection 
/// container, the App struct coordinates access to the mission-critical 
/// system state, discovered backends, and advanced sub-systems.
pub struct App {
    /// Global application configuration.
    pub config: Arc<Config>,
    /// Thread-safe metadata and search cache.
    pub cache: Arc<PackageCache>,
    /// Registry of all discovered package manager backends.
    pub registry: Arc<BackendRegistry>,
    /// Low-level orchestrator for system commands and file I/O.
    pub executor: CommandExecutor,
    /// Transactional telemetry and performance collector.
    pub metrics: MetricsCollector,
    /// Thread-safe interface for terminal progress bars.
    pub progress: Arc<dyn ProgressReporter>,
    /// Multi-engine scripting controller (Lua / Rhai).
    pub hooks: Arc<LuaHooks>,
    /// The mission-critical system state registry (Single Source of Truth).
    pub state: Arc<Mutex<StateRegistry>>,
    /// Orchestrator for atomic system-level snapshots and recovery.
    pub snapshot_manager: Arc<SnapshotManager>,
    /// Write-Ahead Log (WAL) for transaction integrity.
    pub journal: Arc<Mutex<Journal>>,
    /// Modernized Failure Diagnosis Engine.
    pub diagnostics: Arc<FailureDiagnosticEngine>,
    /// Feature 5: Native background task automation engine.
    pub scheduler: Arc<SchedulerManager>,
    /// Feature 5: System-wide alert and multi-channel notification dispatcher.
    pub notifications: Arc<NotificationManager>,
}

impl App {
    /// Initializes the LiNix Application Kernel.
    /// 
    /// This method performs an asynchronous bootstrap of all core services. 
    pub async fn new(config: Config) -> Result<Self> {
        debug!("LiNix Kernel: Initiating mission-critical service bootstrap.");

        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let hooks = Arc::new(LuaHooks::new(&config)?);
        
        // Discover backends on the host
        let registry = Arc::new(create_default_registry(executor.duplicate(), &config, hooks.clone()).await);
        let progress = create_progress_reporter(config.show_progress);
        
        // Load the persistent state (blocking IO wrapped in task)
        let state_val = tokio::task::spawn_blocking(StateRegistry::load)
            .await
            .map_err(|e| Error::Other(format!("State-load join panic: {}", e)))??;
        let state = Arc::new(Mutex::new(state_val));
        
        // Detect snapshot providers and load transaction journal
        let snapshot_manager = Arc::new(SnapshotManager::new(executor.duplicate(), &config).await);
        let journal = Arc::new(Mutex::new(Journal::new()?));
        
        // Feature 5/3.6.0 Managers
        let scheduler = Arc::new(SchedulerManager::new()?);
        let config_arc = Arc::new(config);
        let notifications = Arc::new(NotificationManager::new(config_arc.clone()));
        
        // Asynchronously initialize the Failure Diagnosis Engine
        let diagnostics = Arc::new(FailureDiagnosticEngine::init(&config_arc).await);

        info!("LiNix Kernel: v3.6.0 initialized successfully.");

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

    // ========================================================================
    // Orchestrator Factories (Service Provider Pattern)
    // ========================================================================

    /// Returns a Migrator for ingesting manual installs into management.
    pub fn migrator(&self) -> Migrator { 
        Migrator::new(self.registry.clone(), self.state.clone(), &self.config) 
    }

    /// Returns a Teleporter for cross-backend package transitions.
    pub fn teleporter(&self) -> Teleporter { 
        Teleporter::new(
            self.registry.clone(), 
            self.journal.clone(), 
            self.state.clone(), 
            self.diagnostics.clone(), 
            &self.config.groups_dir
        ) 
    }

    /// Feature 6: Returns the state-aware Ephemeral Ghost Shell orchestrator.
    /// Modernized: Injects the FailureDiagnosticEngine.
    pub fn shell(&self) -> GhostShell { 
        GhostShell::new(
            self.registry.clone(), 
            self.state.clone(), 
            self.config.clone(),
            self.executor.duplicate(),
            self.metrics.clone(),
            self.progress.clone(),
            self.hooks.clone(),
            self.snapshot_manager.clone(),
            self.journal.clone(),
            self.diagnostics.clone(), // DI
        ) 
    }

    /// Returns a ProfileManager for context-sensitive identity switching.
    /// Modernized: Injects the FailureDiagnosticEngine.
    pub fn profile_manager(&self) -> ProfileManager { 
        ProfileManager::new(
            self.registry.clone(),
            self.executor.clone(),
            self.metrics.clone(),
            self.progress.clone(),
            self.hooks.clone(),
            self.snapshot_manager.clone(),
            self.journal.clone(),
            self.state.clone(),
            self.config.clone(),
            self.diagnostics.clone(), // DI
        ) 
    }

    /// Returns an UndoManager for performing system-level state rollbacks.
    pub fn undo_manager(&self) -> UndoManager { 
        UndoManager::new(self.snapshot_manager.clone(), self.state.clone(), self.executor.clone()) 
    }

    /// Returns a Runner for executing commands in isolated environments.
    pub fn runner(&self) -> Runner {
        Runner::new(
            self.registry.clone(), 
            self.config.clone()
        )
    }
    
    /// Asynchronously initializes the binary shim manager.
    pub async fn shim_manager(&self) -> Result<ShimManager> { 
        ShimManager::new().await 
    }

    // ========================================================================
    // Global Kernel Operations
    // ========================================================================

    /// Resolves a package specification string and all recursive dependencies.
    #[instrument(skip(self))]
    pub async fn resolve_spec(&self, spec_str: &str) -> Result<Vec<PackageSpec>> {
        let mut resolved = Vec::new();
        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();

        let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;
        queue.push_back(resolver.parse_and_probe_spec(spec_str).await?);

        while let Some(spec) = queue.pop_front() {
            let key = format!("{}:{}", spec.backend, spec.name);
            if !seen.insert(key) { continue; }
            Validator::validate_package_name(&spec.name)?;
            for req in &spec.requires {
                queue.push_back(resolver.parse_and_probe_spec(req).await?);
            }
            resolved.push(spec);
        }
        Ok(resolved)
    }

    /// Performs a full metadata refresh across all available backends.
    pub async fn update(&self) -> Result<()> {
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                upgradable.update(backend.needs_root()).await?;
            }
        }
        Ok(())
    }

    /// Performs a system-wide upgrade for all managed packages.
    pub async fn upgrade(&self) -> Result<()> {
        let _ = self.snapshot_manager.auto_snapshot("pre_upgrade").await?;
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                upgradable.upgrade(backend.needs_root()).await?;
            }
        }
        self.metrics.print_summary();
        Ok(())
    }

    /// Lists installed packages across all available backends.
    pub async fn list(&self, backend_filter: Option<&str>) -> Result<Vec<Package>> {
        let mut all_packages = Vec::new();
        for backend in self.registry.available() {
            if let Some(filter) = backend_filter {
                if backend.name() != filter { continue; }
            }
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(pkgs) = queryable.list_installed().await { all_packages.extend(pkgs); }
            }
        }
        Ok(all_packages)
    }

    /// Fetches detailed metadata for a single package.
    pub async fn get_info(&self, package_name: &str) -> Result<Option<Package>> {
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(Some(pkg)) = queryable.info(package_name).await { return Ok(Some(pkg)); }
            }
        }
        Ok(None)
    }

    /// Performs a drift audit to identify packages installed but not managed by LiNix.
    pub async fn get_unmanaged_packages(&self) -> Result<Vec<Package>> {
        let mut unmanaged = Vec::new();
        let state = self.state.lock().await;
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(installed) = queryable.list_installed().await {
                    for pkg in installed {
                        if !state.is_managed(&pkg.backend, &pkg.name) { unmanaged.push(pkg); }
                    }
                }
            }
        }
        Ok(unmanaged)
    }

    /// Prunes unused or orphaned dependencies across all active managers.
    pub async fn clean_orphans(&self) -> Result<()> {
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                let _ = upgradable.clean_orphans(backend.needs_root()).await;
            }
        }
        Ok(())
    }

    /// Feature 2: A+ Grade Snapshot Lifecycle Management.
    /// 
    /// Logic: If `force` is true (CLI flag), it overrides the global dry-run.
    pub async fn prune_snapshots(&self, force: bool) -> Result<()> {
        let settings = &self.config.snapshots;
        
        let is_dry_run = if force {
            false
        } else {
            self.config.dry_run
        };

        info!("Kernel: Initiating system snapshot maintenance cycle.");
        self.snapshot_manager.prune_stale_snapshots(
            settings.max_age_days, 
            settings.max_count, 
            is_dry_run
        ).await
    }

    /// Orchestrates a parallel search across all searchable backend repositories.
    pub async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let searcher = UniversalSearch::new(&self.registry, &self.config);
        searcher.search(query).await
    }

    /// Explicitly creates a binary shim for a package.
    pub async fn create_shim(&self, binary_name: &str, _source_spec: &str) -> Result<()> {
        let manager = self.shim_manager().await?;
        manager.create_shim(binary_name).await
    }
}