use crate::core::{Result, Error, StateRegistry, Journal, SnapshotManager, CommandExecutor};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::app::sync::{SyncEngine, StateResolver, ChangePlanner};
use crate::app::{LuaHooks, MetricsCollector};
use crate::utils::progress::ProgressReporter;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, debug};

/// Manages system "Identities" or Profiles (Roadmap Point 18).
/// Allows swapping between different sets of declarative configurations.
/// 
/// Hardened for Phase 4.1: Decoupled from the global App object. Now receives 
/// specific dependencies, enabling synchronization and profile logic to be 
/// tested in isolation.
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
    profiles_dir: PathBuf,
}

impl ProfileManager {
    /// Initializes the ProfileManager with explicit dependency injection.
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
            profiles_dir,
        }
    }

    /// Swaps the current active configuration to the specified profile.
    pub async fn switch(&self, profile_name: &str) -> Result<()> {
        let target_profile_path = self.profiles_dir.join(profile_name);
        
        if !tokio::fs::try_exists(&target_profile_path).await.unwrap_or(false) {
            return Err(Error::Config(format!("Profile '{}' not found in {:?}", profile_name, self.profiles_dir)));
        }

        info!("ProfileManager: Switching identity to '{}'...", profile_name);

        // 1. Clear current active groups
        self.clear_active_groups().await?;

        // 2. Provision profile groups to the active groups directory
        let mut entries = tokio::fs::read_dir(&target_profile_path).await.map_err(Error::from)?;
        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "txt") {
                let file_name = entry.file_name();
                let dest = self.config.groups_dir.join(file_name);
                
                debug!("ProfileManager: Provisioning {:?} -> {:?}", path, dest);
                
                #[cfg(unix)]
                tokio::fs::os::unix::symlink(&path, &dest).await.map_err(Error::from)?;
                
                #[cfg(windows)]
                tokio::fs::copy(&path, &dest).await.map_err(Error::from).map(|_| ())?;
            }
        }

        info!("ProfileManager: Identity '{}' staged. Triggering parallel system sync...", profile_name);

        // 3. Phase 2.2 Integration: Realize the new identity via SyncEngine
        // FIX: Passed self.state.clone() to satisfy the new 9-argument signature
        let engine = SyncEngine::new(
            &self.config,
            self.registry.clone(),
            self.executor.clone(),
            self.metrics.clone(),
            self.progress.clone(),
            self.hooks.clone(),
            self.snapshot_manager.clone(),
            self.journal.clone(),
            self.state.clone(),
        ).await;

        // Calculate the delta for the new profile
        let resolver = StateResolver::new(&self.config, self.registry.clone());
        let desired = resolver.resolve_desired_state().await?;
        
        let changes = {
            let state_guard = self.state.lock().await;
            let planner = ChangePlanner::new(self.registry.clone(), &state_guard, &self.config);
            planner.plan(&desired).await?
        };

        engine.sync(changes).await?;

        info!("ProfileManager: Successfully transitioned system to '{}'.", profile_name);
        Ok(())
    }

    /// Removes all existing manifest files (except local.txt) from the active groups directory.
    async fn clear_active_groups(&self) -> Result<()> {
        let mut entries = tokio::fs::read_dir(&self.config.groups_dir).await.map_err(Error::from)?;
        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            let fname = entry.file_name().to_string_lossy().into_owned();
            
            if path.is_file() && fname.ends_with(".txt") && fname != "local.txt" {
                debug!("ProfileManager: Removing old identity group: {:?}", path);
                tokio::fs::remove_file(path).await.map_err(Error::from)?;
            }
        }
        Ok(())
    }

    /// Creates a new profile by capturing the current set of active group files.
    pub async fn save_current_as(&self, profile_name: &str) -> Result<()> {
        let target_path = self.profiles_dir.join(profile_name);
        if !tokio::fs::try_exists(&target_path).await.unwrap_or(false) {
            tokio::fs::create_dir_all(&target_path).await.map_err(Error::from)?;
        }

        let mut entries = tokio::fs::read_dir(&self.config.groups_dir).await.map_err(Error::from)?;
        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "txt") {
                let dest = target_path.join(entry.file_name());
                tokio::fs::copy(&path, &dest).await.map_err(Error::from)?;
            }
        }

        info!("ProfileManager: Saved current state as profile '{}'", profile_name);
        Ok(())
    }

    /// Returns a list of all available profile names.
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
        Ok(profiles)
    }
}