use crate::core::{Result, Error};
use std::path::{Path, PathBuf};
use std::fs;
use tracing::{info, debug, warn};

/// Manages the deployment of high-performance Rust shims (Point 4 & 6).
/// 
/// Shims are small binaries placed in the user's PATH (usually ~/.local/bin) 
/// that act as proxies. When executed, they invoke 'linix run', which 
/// handles environment provisioning and Bubblewrap sandboxing.
pub struct ShimManager {
    bin_dir: PathBuf,
}

impl ShimManager {
    /// Initializes the manager and ensures the local bin directory exists.
    pub fn new() -> Result<Self> {
        let bin_dir = dirs::home_dir()
            .ok_or_else(|| Error::Other("Could not locate home directory".into()))?
            .join(".local")
            .join("bin");

        if !bin_dir.exists() {
            debug!("ShimManager: Creating shim directory at {:?}", bin_dir);
            fs::create_dir_all(&bin_dir)?;
        }

        Ok(Self { bin_dir })
    }

    /// Deploys a high-performance binary shim.
    /// 
    /// Strategy:
    /// 1. Identifies the path of the current LiNix executable.
    /// 2. Creates a hard link (or copy if cross-device) in ~/.local/bin.
    /// 3. The shim detects its own name via argv[0] and calls 'linix run'.
    pub async fn create_shim(&self, binary_name: &str) -> Result<()> {
        let target_path = self.bin_dir.join(binary_name);

        // Ensure we don't accidentally overwrite the main 'linix' binary if it lives here
        if binary_name == "linix" {
            return Ok(());
        }

        let current_exe = std::env::current_exe()
            .map_err(|e| Error::Io(std::io::Error::new(e.kind(), format!("Failed to locate linix binary: {}", e))))?;
        
        // Clean up existing file/link
        if target_path.exists() || target_path.is_symlink() {
            fs::remove_file(&target_path)?;
        }

        info!("ShimManager: Deploying shim for '{}' -> {:?}", binary_name, target_path);

        #[cfg(unix)]
        {
            // A hard link is the highest performance option. 
            // It allows the kernel to execute the same inode but LiNix sees the new name in args.
            if let Err(e) = fs::hard_link(&current_exe, &target_path) {
                debug!("ShimManager: Hard link failed ({}), falling back to copy...", e);
                // Fallback to copy if the bin_dir is on a different partition
                fs::copy(&current_exe, &target_path)?;
            }
        }

        #[cfg(windows)]
        {
            // Windows usually requires copies for reliable execution of linked binaries.
            let mut win_target = target_path.clone();
            if win_target.extension().map_or(true, |ext| ext != "exe") {
                win_target.set_extension("exe");
            }
            fs::copy(&current_exe, &win_target)?;
        }

        Ok(())
    }

    /// Synchronizes shims based on the declarative state.
    /// If a package is managed and has 'sandbox=true' or is a specific binary type,
    /// this ensures the shim exists.
    pub async fn reconcile_shims(&self, package_name: &str, should_exist: bool) -> Result<()> {
        if should_exist {
            self.create_shim(package_name).await
        } else {
            self.remove_shim(package_name).await
        }
    }

    /// Removes a deployed shim.
    pub async fn remove_shim(&self, binary_name: &str) -> Result<()> {
        let target_path = self.bin_dir.join(binary_name);
        
        #[cfg(windows)]
        let target_path = if target_path.extension().is_none() {
            target_path.with_extension("exe")
        } else {
            target_path
        };

        if target_path.exists() {
            debug!("ShimManager: Removing shim {:?}", target_path);
            fs::remove_file(&target_path)?;
            info!("ShimManager: Successfully removed shim '{}'", binary_name);
        }
        Ok(())
    }

    /// Returns a list of all shims currently managed in the local bin directory.
    pub fn list_shims(&self) -> Result<Vec<String>> {
        let mut shims = Vec::new();
        if !self.bin_dir.exists() {
            return Ok(shims);
        }

        for entry in fs::read_dir(&self.bin_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    // We only count it as a managed shim if it's not the main app
                    if name != "linix" && name != "linix.exe" {
                        shims.push(name.to_string());
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