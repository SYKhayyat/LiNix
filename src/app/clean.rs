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
    /// 1. Prunes unused dependencies (orphans) across all 33 backends.
    /// 2. Clears LiNix's internal metadata caches.
    /// 3. Cleans backend-specific temporary download directories.
    pub async fn clean(&self) -> Result<()> {
        info!("Cleaner: Initiating deep system cleanup...");

        // 1. Backend-specific orphan removal (ISP: Upgradable capability)
        for backend in self.app.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                debug!("Cleaner: Requesting orphan pruning for {}...", backend.name());
                // We use sudo=true for system-level managers
                if let Err(e) = upgradable.clean_orphans(true).await {
                    warn!("Cleaner: Failed to clean orphans for {}: {}", backend.name(), e);
                }
            }
        }

        // 2. Clear LiNix internal PackageCache
        debug!("Cleaner: Clearing LiNix metadata cache...");
        self.app.cache.clear_all().await;

        // 3. Clean temporary storage used by Logic backends (GitHub/Web/AppImage)
        self.clean_temp_dirs().await?;

        info!("Cleaner: System cleanup completed successfully.");
        Ok(())
    }

    /// Internal logic to purge temporary build and download artifacts.
    async fn clean_temp_dirs(&self) -> Result<()> {
        let base_temp = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("linix")
            .join("tmp");

        if base_temp.exists() {
            debug!("Cleaner: Purging temporary directory: {:?}", base_temp);
            tokio::fs::remove_dir_all(&base_temp).await.map_err(Error::Io)?;
            tokio::fs::create_dir_all(&base_temp).await.map_err(Error::Io)?;
        }
        Ok(())
    }
}