use crate::backends::{create_default_registry, BackendRegistry};
use crate::config::Config;
use crate::core::{
    CommandExecutor, PackageCache, Result, Error, 
    Package, StateRegistry, PackageSpec, Validator, SnapshotManager, Journal
};
use crate::app::bridge::DependencyBridge;
use crate::app::migrate::Migrator;
use crate::app::teleport::Teleporter;
use crate::app::shell::GhostShell;
use crate::app::profile::ProfileManager;
use crate::app::shim_manager::ShimManager;
use crate::app::undo::UndoManager;
use crate::utils::progress::{create_progress_reporter, ProgressReporter};

use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::{VecDeque, HashSet};
use tracing::info;
use super::{LuaHooks, MetricsCollector, UniversalSearch};

/// The unified Application Context for LiNix v3.5.0.
/// Coordinates state, configuration, and all high-level orchestrators.
/// 
/// Hardened for Phase 4.1: Functions as a Service Provider.
pub struct App {
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

impl App {
    /// Initializes the application kernel.
    pub async fn new(config: Config) -> Result<Self> {
        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let hooks = Arc::new(LuaHooks::new(&config)?);
        
        let registry = Arc::new(create_default_registry(executor.duplicate(), &config, hooks.clone()).await);
        let progress = create_progress_reporter(config.show_progress);
        
        // StateRegistry::load() involves disk I/O; wrapped in spawn_blocking for async safety
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

    // High-level service accessors
    pub fn migrator(&self) -> Migrator<'_> { Migrator::new(self) }
    pub fn teleporter(&self) -> Teleporter<'_> { Teleporter::new(self) }
    pub fn shell(&self) -> GhostShell<'_> { GhostShell::new(self) }
    pub fn profile_manager(&self) -> ProfileManager<'_> { ProfileManager::new(self) }
    pub fn undo_manager(&self) -> UndoManager<'_> { UndoManager::new(self) }
    
    /// Phase 3.2: ShimManager instantiation is now asynchronous.
    pub async fn shim_manager(&self) -> Result<ShimManager> { 
        ShimManager::new().await 
    }

    /// Priority-Based Probing and recursive meta-dependency resolution.
    /// Note: Recursive Native dependencies are handled by the Planner.
    pub async fn resolve_spec(&self, spec_str: &str) -> Result<Vec<PackageSpec>> {
        let mut resolved = Vec::new();
        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();

        let resolver = crate::app::sync::resolver::StateResolver::new(&self.config, self.registry.clone());
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

    /// Performs a drift audit to identify unmanaged manual packages.
    pub async fn get_unmanaged_packages(&self) -> Result<Vec<Package>> {
        let mut unmanaged = Vec::new();
        let state = self.state.lock().await;
        
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                let installed = queryable.list_installed().await?;
                for pkg in installed {
                    if !state.is_managed(&pkg.backend, &pkg.name) {
                        unmanaged.push(pkg);
                    }
                }
            }
        }
        Ok(unmanaged)
    }

    /// Refreshes metadata across all active backends.
    pub async fn update(&self) -> Result<()> {
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                info!("Updating {} metadata...", backend.name());
                let sudo = backend.needs_root();
                upgradable.update(sudo).await?;
            }
        }
        Ok(())
    }

    /// Upgrades all managed packages.
    pub async fn upgrade(&self) -> Result<()> {
        let _snapshot = self.snapshot_manager.auto_snapshot("pre_upgrade").await?;
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                info!("Upgrading {} packages...", backend.name());
                let sudo = backend.needs_root();
                upgradable.upgrade(sudo).await?;
            }
        }
        self.metrics.print_summary();
        Ok(())
    }

    /// Lists installed packages, optionally filtered by backend.
    pub async fn list(&self, backend_filter: Option<&str>) -> Result<Vec<Package>> {
        let mut all = Vec::new();
        if let Some(name) = backend_filter {
            let b = self.registry.get(name).ok_or_else(|| Error::BackendNotFound(name.into()))?;
            if let Some(queryable) = b.as_queryable() {
                all.extend(queryable.list_installed().await?);
            }
        } else {
            for b in self.registry.available() {
                if let Some(queryable) = b.as_queryable() {
                    all.extend(queryable.list_installed().await?);
                }
            }
        }
        Ok(all)
    }

    /// Prunes unused dependencies from the system.
    pub async fn clean_orphans(&self) -> Result<()> {
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                info!("Cleaning orphans for {}...", backend.name());
                let sudo = backend.needs_root();
                upgradable.clean_orphans(sudo).await?;
            }
        }
        Ok(())
    }

    /// Searches for packages across all searchable backends in parallel.
    pub async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let searcher = UniversalSearch::new(&self.registry, &self.config);
        searcher.search(query).await
    }

    /// Retrieves detailed info for a package by name.
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

    /// Explicitly creates a shim for a binary.
    pub async fn create_shim(&self, binary_name: &str, _source_spec: &str) -> Result<()> {
        let manager = self.shim_manager().await?;
        manager.create_shim(binary_name).await
    }
}