// src/app/profile.rs

use crate::core::{Result, Error, StateRegistry, Journal, SnapshotManager, CommandExecutor};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::app::sync::{SyncEngine, StateResolver, ChangePlanner, ScopedFilter};
use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::{LuaHooks, MetricsCollector};
use crate::utils::progress::ProgressReporter;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, debug, error, instrument};

/// Manages system "Identities" or Profiles.
pub struct ProfileManager {
    registry: Arc<BackendRegistry>,
    executor: CommandExecutor,
    metrics: MetricsCollector,
    progress: Arc<dyn ProgressReporter>,
    hooks: Arc<LuaHooks>,
    snapshot_manager: Arc<SnapshotManager>,
    journal: Arc<Mutex<Journal>>,
    state: Arc<Mutex<StateRegistry>>,
    config: Arc<Config>,
    diagnostics: Arc<FailureDiagnosticEngine>,
    profiles_dir: PathBuf,
}

impl ProfileManager {
    pub fn new(
        registry: Arc<BackendRegistry>,
        executor: CommandExecutor,
        metrics: MetricsCollector,
        progress: Arc<dyn ProgressReporter>,
        hooks: Arc<LuaHooks>,
        snapshot_manager: Arc<SnapshotManager>,
        journal: Arc<Mutex<Journal>>,
        state: Arc<Mutex<StateRegistry>>,
        config: Arc<Config>,
        diagnostics: Arc<FailureDiagnosticEngine>,
    ) -> Self {
        let profiles_dir = config.groups_dir.parent()
            .unwrap_or_else(|| Path::new("."))
            .join("profiles");
        
        Self {
            registry,
            executor,
            metrics,
            progress,
            hooks,
            snapshot_manager,
            journal,
            state,
            config,
            diagnostics,
            profiles_dir,
        }
    }

    #[instrument(skip(self))]
    pub async fn switch(&self, profile_name: &str) -> Result<()> {
        let target_profile_path = self.profiles_dir.join(profile_name);
        
        if !tokio::fs::try_exists(&target_profile_path).await.unwrap_or(false) {
            error!("ProfileManager: Requested identity '{}' does not exist.", profile_name);
            return Err(Error::Config(format!("Profile '{}' not found in {:?}", profile_name, self.profiles_dir)));
        }

        info!("ProfileManager: Transitioning system identity to '{}'...", profile_name);

        self.clear_active_groups().await?;

        let mut entries = tokio::fs::read_dir(&target_profile_path).await.map_err(Error::from)?;
        let mut provisioned_count = 0;

        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "txt") {
                let file_name = entry.file_name();
                let dest = self.config.groups_dir.join(file_name);
                
                debug!("ProfileManager: Staging manifest {:?} -> {:?}", path, dest);
                
                #[cfg(unix)]
                {
                    if tokio::fs::try_exists(&dest).await.unwrap_or(false) || dest.is_symlink() {
                        tokio::fs::remove_file(&dest).await?;
                    }
                    // FIX: Use tokio::os::unix::fs::symlink (correct path)
                    tokio::os::unix::fs::symlink(&path, &dest).await.map_err(Error::from)?;
                }
                
                #[cfg(windows)]
                {
                    tokio::fs::copy(&path, &dest).await.map_err(Error::from)?;
                }
                provisioned_count += 1;
            }
        }

        debug!("ProfileManager: Identity '{}' staged with {} manifests.", profile_name, provisioned_count);

        info!("ProfileManager: Triggering system synchronization for new identity...");

        let engine = SyncEngine::new(
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
        ).await;

        let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;
        let desired = resolver.resolve_desired_state().await?;
        
        let changes = {
            let state_guard = self.state.lock().await;
            let planner = ChangePlanner::new(self.registry.clone(), &state_guard, &self.config);
            planner.plan(&desired, ScopedFilter::None).await?
        };

        engine.sync(changes).await?;

        info!("ProfileManager: Successfully transitioned system to identity '{}'.", profile_name);
        Ok(())
    }

    async fn clear_active_groups(&self) -> Result<()> {
        debug!("ProfileManager: Cleaning active manifest directory.");
        
        if !tokio::fs::try_exists(&self.config.groups_dir).await.unwrap_or(false) {
            return Ok(());
        }

        let mut entries = tokio::fs::read_dir(&self.config.groups_dir).await.map_err(Error::from)?;
        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            let fname = entry.file_name().to_string_lossy().into_owned();
            
            if path.is_file() && fname.ends_with(".txt") && fname != "local.txt" {
                debug!("ProfileManager: Purging old identity manifest: {:?}", path);
                tokio::fs::remove_file(path).await.map_err(Error::from)?;
            }
        }
        Ok(())
    }

    pub async fn save_current_as(&self, profile_name: &str) -> Result<()> {
        let target_path = self.profiles_dir.join(profile_name);
        info!("ProfileManager: Saving active state as profile '{}' in {:?}", profile_name, target_path);
        
        if !tokio::fs::try_exists(&target_path).await.unwrap_or(false) {
            tokio::fs::create_dir_all(&target_path).await.map_err(Error::from)?;
        }

        let mut entries = tokio::fs::read_dir(&self.config.groups_dir).await.map_err(Error::from)?;
        let mut saved_count = 0;

        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "txt") {
                let dest = target_path.join(entry.file_name());
                debug!("ProfileManager: Archiving {:?} -> {:?}", path, dest);
                tokio::fs::copy(&path, &dest).await.map_err(Error::from)?;
                saved_count += 1;
            }
        }

        info!("ProfileManager: Profile '{}' created with {} manifests.", profile_name, saved_count);
        Ok(())
    }

    pub async fn list_profiles(&self) -> Result<Vec<String>> {
        if !tokio::fs::try_exists(&self.profiles_dir).await.unwrap_or(false) {
            return Ok(vec![]);
        }

        let mut profiles = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.profiles_dir).await.map_err(Error::from)?;
        
        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            if entry.file_type().await.map_or(false, |t| t.is_dir()) {
                profiles.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        
        profiles.sort();
        Ok(profiles)
    }
}