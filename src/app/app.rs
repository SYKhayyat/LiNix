use crate::backends::{create_default_registry, BackendRegistry};
use crate::config::Config;
use crate::core::{CommandExecutor, PackageCache, Result, Validator};
use crate::app::{LuaHooks, MetricsCollector, SyncEngine, UniversalSearch};
use crate::utils::progress::{create_progress_reporter, ProgressReporter};
use tracing::info;

/// Main application struct
pub struct App {
    pub config: Config,
    pub cache: PackageCache,
    pub registry: BackendRegistry,
    pub executor: CommandExecutor,
    pub validator: Validator,
    pub metrics: MetricsCollector,
    pub progress: Box<dyn ProgressReporter>,
    pub hooks: LuaHooks,
}

impl App {
    /// Create a new App instance
    pub async fn new(config: Config) -> Result<Self> {
        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let registry = create_default_registry(executor.clone()).await;
        let progress = create_progress_reporter(config.show_progress);
        let hooks = LuaHooks::new(&config)?;

        Ok(Self {
            config,
            cache: PackageCache::new(),
            registry,
            executor,
            validator: Validator,
            metrics: MetricsCollector::new(),
            progress,
            hooks,
        })
    }

    /// Run the sync operation
    pub async fn sync(&mut self) -> Result<()> {
        info!("Starting sync operation");
        self.metrics.start_operation("sync");

        let engine = SyncEngine::new(
            &self.config,
            &self.registry,
            &self.executor,
            &self.cache,
            &self.metrics,
            self.progress.as_ref(),
            &self.hooks,
        );

        let result = engine.sync().await;

        self.metrics.end_operation("sync");
        result
    }

    /// Show unmanaged packages
    pub async fn unmanaged(&self) -> Result<Vec<(String, Vec<String>)>> {
        info!("Finding unmanaged packages");

        // Create a temporary metrics collector for this operation
        let temp_metrics = MetricsCollector::new();

        let engine = SyncEngine::new(
            &self.config,
            &self.registry,
            &self.executor,
            &self.cache,
            &temp_metrics,
            self.progress.as_ref(),
            &self.hooks,
        );

        engine.find_unmanaged().await
    }

    /// Clean unmanaged packages
    pub async fn clean(&mut self) -> Result<()> {
        info!("Starting clean operation");
        self.metrics.start_operation("clean");

        let engine = SyncEngine::new(
            &self.config,
            &self.registry,
            &self.executor,
            &self.cache,
            &self.metrics,
            self.progress.as_ref(),
            &self.hooks,
        );

        let result = engine.clean().await;

        self.metrics.end_operation("clean");
        result
    }

    /// Clean orphan packages
    pub async fn orphans(&mut self) -> Result<()> {
        info!("Starting orphan cleanup");
        self.metrics.start_operation("orphans");

        let managers = self.registry.available();

        for manager in managers {
            if manager.supports_orphan_cleanup() {
                info!("Cleaning orphans for {}", manager.name());
                if let Err(e) = manager.clean_orphans(true).await {
                    tracing::warn!("Failed to clean orphans for {}: {}", manager.name(), e);
                }
            }
        }

        self.metrics.end_operation("orphans");
        Ok(())
    }

    /// Search across all backends
    pub async fn search(&self, query: &str) -> Result<Vec<crate::core::Package>> {
        info!("Searching for: {}", query);

        let searcher = UniversalSearch::new(&self.registry, &self.config);
        searcher.search(query).await
    }

    /// Get metrics report
    pub fn get_metrics_report(&self) -> serde_json::Value {
        self.metrics.to_json()
    }

    /// Get list of available backends
    pub fn available_backends(&self) -> Vec<String> {
        self.registry.available_names()
    }
}
