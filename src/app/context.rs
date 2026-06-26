// src/app/context.rs

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
use crate::app::sync::SyncEngine;
use crate::utils::progress::{create_progress_reporter, ProgressReporter};

use std::sync::Arc;
use std::path::PathBuf;
use tokio::sync::Mutex;
use std::collections::{VecDeque, HashSet};
use tracing::{info, debug, instrument};
use super::{LuaHooks, MetricsCollector, UniversalSearch};

/// The unified Application Context for LiNix v3.6.0.
pub struct App {
    /// Global application configuration.
    pub config: Arc<Config>,
    /// Thread-safe metadata and search cache.
    pub cache: Arc<PackageCache>,
    /// Registry of all discovered and available package manager backends.
    pub registry: Arc<BackendRegistry>,
    /// Low-level orchestrator for system commands and file I/O.
    pub executor: CommandExecutor,
    /// Transactional telemetry and performance collector.
    pub metrics: MetricsCollector,
    /// Thread-safe interface for terminal progress bars and spinners.
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
    /// Feature 5: Multi-channel alert and notification dispatcher.
    pub notifications: Arc<NotificationManager>,
}

impl App {
    /// Modernized DI Factory: Initializes the kernel with a specific executor and optional state path.
    pub async fn new_with_executor_and_state_path(
        config: Config,
        executor: CommandExecutor,
        state_path: Option<PathBuf>,
    ) -> Result<Self> {
        debug!("LiNix Kernel: Initiating mission-critical service bootstrap.");

        let hooks = Arc::new(LuaHooks::new(&config)?);

        // Discover backends on the host
        let registry = Arc::new(create_default_registry(executor.duplicate(), &config, hooks.clone()).await);
        let progress = create_progress_reporter(config.show_progress);

        // Load the persistent state registry using the provided path or default.
        let state_registry = if let Some(path) = state_path {
            tokio::task::spawn_blocking(move || StateRegistry::load_from(&path))
                .await
                .map_err(|e| Error::Other(format!("Kernel Thread Panic during state load: {}", e)))?
        } else {
            tokio::task::spawn_blocking(StateRegistry::load_default)
                .await
                .map_err(|e| Error::Other(format!("Kernel Thread Panic during state load: {}", e)))?
        }?;
        let state = Arc::new(Mutex::new(state_registry));

        // Detect snapshot providers and load transaction journal
        let snapshot_manager = Arc::new(SnapshotManager::new(executor.duplicate(), &config).await);
        let journal = Arc::new(Mutex::new(Journal::new()?));

        // Feature 5/3.6.0 Managers
        let scheduler = Arc::new(SchedulerManager::new()?);
        let config_arc = Arc::new(config);
        let notifications = Arc::new(NotificationManager::new(config_arc.clone()));

        // Asynchronously initialize the Failure Diagnosis Engine
        let diagnostics = Arc::new(FailureDiagnosticEngine::init(&config_arc).await);

        info!("LiNix Kernel: v5.0.0 kernel initialized successfully.");

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

    /// Modernized DI Factory: Initializes the kernel with a specific executor (uses default state path).
    pub async fn new_with_executor(config: Config, executor: CommandExecutor) -> Result<Self> {
        Self::new_with_executor_and_state_path(config, executor, None).await
    }

    /// Standard entry point using the default system executors and default state path.
    pub async fn new(config: Config) -> Result<Self> {
        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        Self::new_with_executor_and_state_path(config, executor, None).await
    }

    // ========================================================================
    // Orchestrator Factories (Service Provider Pattern)
    // ========================================================================

    pub fn migrator(&self) -> Migrator {
        Migrator::new(self.registry.clone(), self.state.clone(), &self.config)
    }

    pub fn teleporter(&self) -> Teleporter {
        Teleporter::new(
            self.registry.clone(),
            self.journal.clone(),
            self.state.clone(),
            self.diagnostics.clone(),
            &self.config.groups_dir
        )
    }

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
            self.diagnostics.clone(),
        )
    }

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
            self.diagnostics.clone(),
        )
    }

    pub fn undo_manager(&self) -> UndoManager {
        UndoManager::new(self.snapshot_manager.clone(), self.state.clone(), self.executor.clone())
    }

    pub fn runner(&self) -> Runner {
        Runner::new(self.registry.clone(), self.config.clone())
    }

    pub async fn shim_manager(&self) -> Result<ShimManager> {
        ShimManager::new().await
    }

    pub async fn sync_engine(&self) -> SyncEngine<'_> {
        SyncEngine::new(
            &self.config,
            self.registry.clone(),
            self.executor.duplicate(),
            self.metrics.clone(),
            self.progress.clone(),
            self.hooks.clone(),
            self.snapshot_manager.clone(),
            self.journal.clone(),
            self.state.clone(),
            self.diagnostics.clone(),
        ).await
    }

    // ========================================================================
    // Global Kernel Operations
    // ========================================================================

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

    pub async fn update(&self) -> Result<()> {
        info!("Kernel: Initiating metadata synchronization across enabled backends.");
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                upgradable.update(backend.needs_root()).await?;
            }
        }
        Ok(())
    }

    pub async fn upgrade(&self) -> Result<()> {
        let _ = self.snapshot_manager.auto_snapshot("pre_upgrade").await?;
        info!("Kernel: Commencing system-wide batch upgrade.");
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                upgradable.upgrade(backend.needs_root()).await?;
            }
        }
        self.metrics.print_summary();
        Ok(())
    }

    pub async fn list(&self, backend_filter: Option<&str>) -> Result<Vec<Package>> {
        let mut all_packages = Vec::new();
        for backend in self.registry.available() {
            if let Some(filter) = backend_filter {
                if backend.name() != filter { continue; }
            }
            if let Some(queryable) = backend.as_queryable() {
                match queryable.list_installed().await {
                    Ok(pkgs) => all_packages.extend(pkgs),
                    Err(e) => debug!("Kernel: Query failed for backend '{}': {}", backend.name(), e),
                }
            }
        }
        Ok(all_packages)
    }

    pub async fn get_info(&self, package_name: &str) -> Result<Option<Package>> {
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(Some(pkg)) = queryable.info(package_name).await {
                    return Ok(Some(pkg));
                }
            }
        }
        Ok(None)
    }

    pub async fn get_unmanaged_packages(&self) -> Result<Vec<Package>> {
        let mut unmanaged = Vec::new();
        let state = self.state.lock().await;
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(installed) = queryable.list_installed().await {
                    for pkg in installed {
                        if !state.is_managed(&pkg.backend, &pkg.name) {
                            unmanaged.push(pkg);
                        }
                    }
                }
            }
        }
        Ok(unmanaged)
    }

    pub async fn clean_orphans(&self) -> Result<()> {
        info!("Kernel: Commencing system-wide orphan pruning cycle.");
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                let _ = upgradable.clean_orphans(backend.needs_root()).await;
            }
        }
        Ok(())
    }

    pub async fn prune_snapshots(&self, force: bool) -> Result<()> {
        let settings = &self.config.snapshots;
        let is_dry_run = if force { false } else { self.config.dry_run };
        info!("Kernel: Commencing snapshot maintenance cycle (Limit: {} days / {} count).",
              settings.max_age_days, settings.max_count);
        self.snapshot_manager.prune_stale_snapshots(
            settings.max_age_days,
            settings.max_count,
            is_dry_run
        ).await
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let searcher = UniversalSearch::new(&self.registry, &self.config);
        searcher.search(query).await
    }

    pub async fn create_shim(&self, binary_name: &str, _source_spec: &str) -> Result<()> {
        let manager = self.shim_manager().await?;
        manager.create_shim(binary_name).await
    }
}