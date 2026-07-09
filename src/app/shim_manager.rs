use crate::core::{Error, Result};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, info};

/// Manages the deployment of high-performance Rust shims (Point 4 & 6).
/// Shims act as proxies that invoke 'linix run' with sandboxing.
/// Hardened for Phase 3.2: Full Async I/O support.
pub struct ShimManager {
    bin_dir: PathBuf,
}

impl ShimManager {
    /// Initializes the manager and ensures the local bin directory exists.
    pub async fn new() -> Result<Self> {
        let bin_dir = dirs::home_dir()
            .ok_or_else(|| Error::Other("Could not locate home directory".into()))?
            .join(".local")
            .join("bin");

        if !tokio::fs::try_exists(&bin_dir).await.unwrap_or(false) {
            debug!("ShimManager: Creating shim directory at {:?}", bin_dir);
            fs::create_dir_all(&bin_dir).await.map_err(Error::from)?;
        }

        Ok(Self { bin_dir })
    }

    /// Deploys a high-performance binary shim.
    pub async fn create_shim(&self, binary_name: &str) -> Result<()> {
        #[allow(unused_mut)] // mutated only under cfg(windows)
        let mut target_path = self.bin_dir.join(binary_name);

        // Ensure we don't accidentally overwrite the main 'linix' binary
        if binary_name == "linix" {
            return Ok(());
        }

        #[cfg(windows)]
        {
            if target_path.extension().is_none_or(|ext| ext != "exe") {
                target_path.set_extension("exe");
            }
        }

        // std::env::current_exe() is blocking; wrap in spawn_blocking
        let current_exe = tokio::task::spawn_blocking(std::env::current_exe)
            .await
            .map_err(|e| Error::Other(e.to_string()))?
            .map_err(|e| Error::Io(format!("Failed to locate linix binary: {}", e)))?;

        // Clean up existing file/link
        if tokio::fs::try_exists(&target_path).await.unwrap_or(false) || target_path.is_symlink() {
            fs::remove_file(&target_path).await.map_err(Error::from)?;
        }

        info!(
            "ShimManager: Deploying shim for '{}' -> {:?}",
            binary_name, target_path
        );

        #[cfg(unix)]
        {
            // A hard link is the highest performance option.
            if let Err(e) = fs::hard_link(&current_exe, &target_path).await {
                debug!(
                    "ShimManager: Hard link failed ({}), falling back to copy...",
                    e
                );
                fs::copy(&current_exe, &target_path)
                    .await
                    .map_err(Error::from)?;
            }
        }

        #[cfg(windows)]
        {
            fs::copy(&current_exe, &target_path)
                .await
                .map_err(Error::from)?;
        }

        Ok(())
    }

    /// Synchronizes shims based on the declarative state.
    pub async fn reconcile_shims(&self, package_name: &str, should_exist: bool) -> Result<()> {
        if should_exist {
            self.create_shim(package_name).await
        } else {
            self.remove_shim(package_name).await
        }
    }

    /// Removes a deployed shim.
    pub async fn remove_shim(&self, binary_name: &str) -> Result<()> {
        #[allow(unused_mut)] // mutated only under cfg(windows)
        let mut target_path = self.bin_dir.join(binary_name);

        #[cfg(windows)]
        {
            if target_path.extension().is_none() {
                target_path.set_extension("exe");
            }
        }

        if tokio::fs::try_exists(&target_path).await.unwrap_or(false) || target_path.is_symlink() {
            debug!("ShimManager: Removing shim {:?}", target_path);
            fs::remove_file(&target_path).await.map_err(Error::from)?;
            info!("ShimManager: Successfully removed shim '{}'", binary_name);
        }
        Ok(())
    }

    /// Returns a list of all shims currently managed in the local bin directory.
    pub async fn list_shims(&self) -> Result<Vec<String>> {
        let mut shims = Vec::new();
        if !tokio::fs::try_exists(&self.bin_dir).await.unwrap_or(false) {
            return Ok(shims);
        }

        let mut entries = fs::read_dir(&self.bin_dir).await.map_err(Error::from)?;

        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            let metadata = entry.metadata().await.map_err(Error::from)?;

            if metadata.is_file() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    // Filter out the main binary and non-shims
                    if name != "linix" && name != "linix.exe" {
                        // On windows, remove the extension for the listing
                        #[cfg(windows)]
                        {
                            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                shims.push(stem.to_string());
                            }
                        }
                        #[cfg(unix)]
                        {
                            shims.push(name.to_string());
                        }
                    }
                }
            }
        }
        Ok(shims)
    }

    /// Path accessor for external verification.
    pub fn get_bin_dir(&self) -> &Path {
        &self.bin_dir
    }
}
