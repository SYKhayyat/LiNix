use crate::App;
use crate::core::{Result, Error};
use crate::app::sync::{SyncEngine, StateResolver, ChangePlanner};
use std::path::{Path, PathBuf};
use tracing::{info, debug};

/// Manages system "Identities" or Profiles (Roadmap Point 18).
/// Allows swapping between different sets of declarative configurations.
pub struct ProfileManager<'a> {
    app: &'a App,
    profiles_dir: PathBuf,
}

impl<'a> ProfileManager<'a> {
    /// Initializes the ProfileManager.
    pub fn new(app: &'a App) -> Self {
        let profiles_dir = app.config.groups_dir.parent()
            .unwrap_or_else(|| Path::new("."))
            .join("profiles");
        
        Self {
            app,
            profiles_dir,
        }
    }

    /// Swaps the current active configuration to the specified profile.
    pub async fn switch(&self, profile_name: &str) -> Result<()> {
        let target_profile_path = self.profiles_dir.join(profile_name);
        
        if !target_profile_path.exists() {
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
                let dest = self.app.config.groups_dir.join(file_name);
                
                debug!("ProfileManager: Provisioning {:?} -> {:?}", path, dest);
                
                #[cfg(unix)]
                tokio::fs::symlink(&path, &dest).await.map_err(Error::from)?;
                
                #[cfg(windows)]
                tokio::fs::copy(&path, &dest).await.map_err(Error::from).map(|_| ())?;
            }
        }

        info!("ProfileManager: Identity '{}' staged. Triggering parallel system sync...", profile_name);

        // 3. Realize the new identity via SyncEngine
        // Fix E0599: Await the constructor
        let engine = SyncEngine::new(
            &self.app.config,
            self.app.registry.clone(),
            self.app.executor.clone(),
            self.app.metrics.clone(),
            self.app.progress.clone(),
            self.app.hooks.clone(),
            self.app.snapshot_manager.clone(),
            self.app.journal.clone(),
        ).await;

        // Calculate the delta for the new profile
        let resolver = StateResolver::new(&self.app.config, self.app.registry.clone());
        let desired = resolver.resolve_desired_state().await?;
        
        let changes = {
            let state = self.app.state.lock().await;
            let planner = ChangePlanner::new(self.app.registry.clone(), &state, &self.app.config);
            planner.plan(&desired).await?
        };

        // Fix E0061: Pass the calculated changes to sync()
        engine.sync(changes).await?;

        info!("ProfileManager: Successfully transitioned system to '{}'.", profile_name);
        Ok(())
    }

    /// Removes all existing manifest files (except local.txt) from the active groups directory.
    async fn clear_active_groups(&self) -> Result<()> {
        let mut entries = tokio::fs::read_dir(&self.app.config.groups_dir).await.map_err(Error::from)?;
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
        if !target_path.exists() {
            tokio::fs::create_dir_all(&target_path).await.map_err(Error::from)?;
        }

        let mut entries = tokio::fs::read_dir(&self.app.config.groups_dir).await.map_err(Error::from)?;
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
        if !self.profiles_dir.exists() {
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