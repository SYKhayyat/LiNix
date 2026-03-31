// src/app/context.rs
use crate::backends::{create_default_registry, BackendRegistry};
use crate::config::Config;
use crate::core::{CommandExecutor, PackageCache, Result, Validator, manager::PackageManager};
use crate::utils::progress::{create_progress_reporter, ProgressReporter};
use std::sync::Arc;
use tracing::info;

use super::{LuaHooks, MetricsCollector, SyncEngine, UniversalSearch};

pub struct App {
    pub config: Config,
    pub cache: Arc<PackageCache>,
    pub registry: Arc<BackendRegistry>,
    pub executor: CommandExecutor,
    pub validator: Validator,
    pub metrics: MetricsCollector,
    pub progress: Arc<dyn ProgressReporter>,
    pub hooks: Arc<LuaHooks>,
}

impl App {
    pub async fn new(config: Config) -> Result<Self> {
        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        let hooks = Arc::new(LuaHooks::new(&config)?);
        let registry = Arc::new(create_default_registry(executor.clone(), &config, hooks.clone()).await);
        let progress = create_progress_reporter(config.show_progress);

        Ok(Self {
            config,
            cache: Arc::new(PackageCache::new()),
            registry,
            executor,
            validator: Validator,
            metrics: MetricsCollector::new(),
            progress,
            hooks,
        })
    }

    pub async fn sync(&self) -> Result<()> {
        let engine = SyncEngine::new(&self.config, self.registry.clone(), self.executor.clone(), self.cache.clone(), self.metrics.clone(), self.progress.clone(), self.hooks.clone());
        engine.sync().await
    }

    pub async fn teleport(&self, package: &str, target_backend: &str) -> Result<()> {
        info!("Teleporting {} to {}...", package, target_backend);
        let mut source_backend = None;
        for manager in self.registry.available() {
            if let Ok(inst) = manager.list_installed().await {
                if inst.iter().any(|p| p.name == package) {
                    source_backend = Some(manager.name().to_string());
                    break;
                }
            }
        }
        let src = source_backend.ok_or_else(|| crate::core::Error::Other(format!("Package {} not found", package)))?;
        let target_mgr = self.registry.get(target_backend).ok_or_else(|| crate::core::Error::BackendNotFound(target_backend.into()))?;
        self.registry.get(&src).unwrap().remove(&[package.to_string()], true).await?;
        target_mgr.install(&[package.to_string()], true).await?;
        Ok(())
    }

    pub async fn create_shim(&self, binary: &str, source_spec: &str) -> Result<()> {
        let bin_dir = dirs::home_dir().unwrap_or_default().join(".local").join("bin");
        let shim_path = bin_dir.join(binary);
        let content = format!("#!/bin/sh\nlinix run --packages \"{}\" --command \"{} $@\"", source_spec, binary);
        tokio::fs::create_dir_all(&bin_dir).await?;
        tokio::fs::write(&shim_path, content).await?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&shim_path, std::fs::Permissions::from_mode(0o755)).await?;
        }
        Ok(())
    }

    pub async fn run_ephemeral(&self, package_urls: Vec<String>, command: &str) -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let bin_dir = temp_dir.path().join("bin");
        tokio::fs::create_dir_all(&bin_dir).await?;
        let web_manager = crate::backends::web::WebManager::new(self.executor.clone(), None);
        let specs: Vec<crate::core::PackageSpec> = package_urls.into_iter().map(|u| {
            let mut options = std::collections::HashMap::new();
            options.insert("type".to_string(), "program".to_string());
            crate::core::PackageSpec { name: u, backend: "web".into(), options }
        }).collect();
        web_manager.install_with_options(&specs, false).await?;

        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        let arg = if cfg!(windows) { "/C" } else { "-c" };
        let mut child = tokio::process::Command::new(shell).arg(arg).arg(command)
            .env("PATH", format!("{}:{}", bin_dir.to_string_lossy(), std::env::var("PATH").unwrap_or_default()))
            .spawn().map_err(|e| crate::core::Error::Other(e.to_string()))?;
        child.wait().await?;
        Ok(())
    }

    pub async fn unmanaged(&self) -> Result<Vec<(String, Vec<String>)>> {
        let engine = SyncEngine::new(&self.config, self.registry.clone(), self.executor.clone(), self.cache.clone(), self.metrics.clone(), self.progress.clone(), self.hooks.clone());
        engine.find_unmanaged().await
    }

    pub async fn clean(&self) -> Result<()> {
        let engine = SyncEngine::new(&self.config, self.registry.clone(), self.executor.clone(), self.cache.clone(), self.metrics.clone(), self.progress.clone(), self.hooks.clone());
        engine.clean().await
    }

    pub async fn orphans(&mut self) -> Result<()> {
        for manager in self.registry.available() {
            if manager.supports_orphan_cleanup() { let _ = manager.clean_orphans(true).await; }
        }
        Ok(())
    }

    pub async fn search(&self, query: &str) -> Result<Vec<crate::core::Package>> {
        let searcher = UniversalSearch::new(&self.registry, &self.config);
        searcher.search(query).await
    }
}