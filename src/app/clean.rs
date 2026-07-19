use crate::core::{Result, Error};
use crate::App;
use tracing::{info, warn, debug};

pub struct Cleaner<'a> {
    app: &'a App,
}

impl<'a> Cleaner<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    pub async fn clean(&self) -> Result<()> {
        debug!("starting cleanup");

        // A backend with no orphan concept returns Error::Unsupported — a benign skip that
        // must not be counted or reported like a real failure.
        let (mut cleaned, mut skipped, mut failed) = (0u32, 0u32, 0u32);
        for backend in self.app.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                debug!("Requesting orphan pruning for {}...", backend.name());
                let sudo = backend.sudo_for_write();
                match upgradable.clean_orphans(sudo).await {
                    Ok(()) => { cleaned += 1; }
                    Err(Error::Unsupported(_)) => {
                        skipped += 1;
                        debug!("{} has no orphan-cleanup concept — skipping.", backend.name());
                    }
                    Err(e) => {
                        failed += 1;
                        warn!("Failed to clean orphans for {}: {}", backend.name(), e);
                    }
                }
            }
        }
        info!("orphan pass complete — {} cleaned, {} not applicable, {} failed.",
              cleaned, skipped, failed);

        debug!("Clearing LiNix metadata cache...");
        self.app.cache.clear_all().await;

        self.clean_temp_dirs().await?;

        debug!("cleanup complete");
        Ok(())
    }

    async fn clean_temp_dirs(&self) -> Result<()> {
        let base_temp = &self.app.config.tmp_dir;

        if tokio::fs::try_exists(base_temp).await.unwrap_or(false) {
            debug!("Purging temporary directory: {:?}", base_temp);
            tokio::fs::remove_dir_all(base_temp).await.map_err(|e| Error::Io(e.to_string()))?;
            tokio::fs::create_dir_all(base_temp).await.map_err(|e| Error::Io(e.to_string()))?;
        } else {
            debug!("Temporary directory {:?} does not exist. Skipping.", base_temp);
        }
        Ok(())
    }
}