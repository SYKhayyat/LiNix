// src/app/context.rs
use crate::backends::{create_default_registry, BackendRegistry};
use crate::config::Config;
use crate::core::{CommandExecutor, PackageCache, Result, Validator};
use crate::utils::progress::{create_progress_reporter, ProgressReporter};
use tracing::info;

// Pull siblings from the parent module (app/mod.rs)
use super::{LuaHooks, MetricsCollector, SyncEngine, UniversalSearch};

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
    pub async fn new(config: Config) -> Result<Self> {
        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let registry = create_default_registry(executor.clone(), &config).await;
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

    pub async fn sync(&self) -> Result<()> {
        info!("Starting sync operation");
        let engine = SyncEngine::new(
            &self.config,
            &self.registry,
            &self.executor,
            &self.cache,
            &self.metrics,
            self.progress.as_ref(),
            &self.hooks,
        );
        engine.sync().await
    }

    pub async fn unmanaged(&self) -> Result<Vec<(String, Vec<String>)>> {
        let engine = SyncEngine::new(
            &self.config,
            &self.registry,
            &self.executor,
            &self.cache,
            &self.metrics,
            self.progress.as_ref(),
            &self.hooks,
        );
        engine.find_unmanaged().await
    }

    pub async fn clean(&self) -> Result<()> {
        let engine = SyncEngine::new(
            &self.config,
            &self.registry,
            &self.executor,
            &self.cache,
            &self.metrics,
            self.progress.as_ref(),
            &self.hooks,
        );
        engine.clean().await
    }

    pub async fn orphans(&mut self) -> Result<()> {
        let managers = self.registry.available();
        for manager in managers {
            if manager.supports_orphan_cleanup() {
                let _ = manager.clean_orphans(true).await;
            }
        }
        Ok(())
    }

    pub async fn search(&self, query: &str) -> Result<Vec<crate::core::Package>> {
        let searcher = UniversalSearch::new(&self.registry, &self.config);
        searcher.search(query).await
    }

    pub fn get_metrics_report(&self) -> serde_json::Value {
        self.metrics.to_json()
    }

    pub fn available_backends(&self) -> Vec<String> {
        self.registry.available_names()
    }
}