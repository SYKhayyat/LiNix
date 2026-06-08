use crate::core::{Result, Error};
use crate::App;
use tracing::{info, warn, debug};

/// Handles system-wide cleanup operations for the LiNix engine.
/// Coordinates between backends to prune orphans and clear persistent caches.
pub struct Cleaner<'a> {
    app: &'a App,
}

impl<'a> Cleaner<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Entry point for deep system cleaning.
    /// 
    /// 1. Prunes unused dependencies (orphans) across all available backends.
    /// 2. Clears LiNix's internal metadata caches.
    /// 3. Cleans backend-specific temporary download directories defined in Config.
    pub async fn clean(&self) -> Result<()> {
        info!("Cleaner: Initiating deep system cleanup...");

        // 1. Backend-specific orphan removal
        for backend in self.app.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                debug!("Cleaner: Requesting orphan pruning for {}...", backend.name());
                // Use the backend's root requirement setting
                let sudo = backend.needs_root();
                if let Err(e) = upgradable.clean_orphans(sudo).await {
                    warn!("Cleaner: Failed to clean orphans for {}: {}", backend.name(), e);
                }
            }
        }

        // 2. Clear LiNix internal PackageCache
        debug!("Cleaner: Clearing LiNix metadata cache...");
        self.app.cache.clear_all().await;

        // 3. Clean temporary storage (Phase 1.4: Use configurable path)
        self.clean_temp_dirs().await?;

        info!("Cleaner: System cleanup completed successfully.");
        Ok(())
    }

    /// Internal logic to purge temporary build and download artifacts.
    /// Fulfills Phase 1.4: Uses the tmp_dir specified in the LiNix configuration.
    async fn clean_temp_dirs(&self) -> Result<()> {
        let base_temp = &self.app.config.tmp_dir;

        if tokio::fs::try_exists(base_temp).await.unwrap_or(false) {
            debug!("Cleaner: Purging temporary directory: {:?}", base_temp);
            // We remove and recreate to ensure a completely clean state
            tokio::fs::remove_dir_all(base_temp).await.map_err(|e| Error::Io(e.to_string()))?;
            tokio::fs::create_dir_all(base_temp).await.map_err(|e| Error::Io(e.to_string()))?;
        } else {
            debug!("Cleaner: Temporary directory {:?} does not exist. Skipping.", base_temp);
        }
        Ok(())
    }
}