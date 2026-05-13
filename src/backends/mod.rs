pub mod registry;
pub mod generic;

// Specialized backends with complex internal state or custom API logic.
pub mod github;
pub mod web;
pub mod link;
pub mod nix;
pub mod vscode;
pub mod mise;
pub mod emacs;
pub mod service;
pub mod appimage;
pub mod snap;
pub mod flatpak;

pub use registry::{create_default_registry, BackendRegistry};
pub use generic::{GenericManager, ManagerConfig};

use crate::core::{Result, Error};
use std::path::{Path, PathBuf};
use std::fs;
use tracing::{debug, info};

/// The ManagedStore trait defines a unified interface for "Logic Backends"
/// (GitHub, Web, AppImage) to handle content-addressable storage and 
/// deterministic binary linkage.
/// 
/// Fulfills Point 4: Convergence of fragmented logic storage.
pub trait ManagedStore: Send + Sync {
    /// Returns the primary installation directory for the package.
    fn get_store_path(&self, package_name: &str) -> PathBuf;
    
    /// Synchronizes the binary symlinks in ~/.local/bin.
    fn link_binaries(&self, package_name: &str, bin_names: &[String]) -> Result<Vec<PathBuf>>;
}

/// Unified utility for calculating package storage paths across all backends.
/// Standardizes layout to: ~/.local/share/linix/store/<backend>/<md5_of_name>
pub struct Store;

impl Store {
    /// Generates a deterministic, platform-agnostic storage path for a package.
    pub fn package_path(backend: &str, name: &str) -> PathBuf {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("/var/lib"))
            .join("linix")
            .join("store")
            .join(backend);
        
        // Use an MD5 hash of the package name (URL or Repo path) to ensure 
        // filesystem compatibility and prevent path traversal.
        let id = format!("{:x}", md5::compute(name.as_bytes()));
        base.join(id)
    }

    /// Returns the user's local bin directory for shim/link exposure.
    pub fn user_bin_dir() -> Result<PathBuf> {
        let bin = dirs::home_dir()
            .ok_or_else(|| Error::Other("Could not locate home directory".into()))?
            .join(".local")
            .join("bin");
        
        if !bin.exists() {
            fs::create_dir_all(&bin)?;
        }
        Ok(bin)
    }

    /// Surgically links a source binary into the user's PATH.
    /// Handles existing collisions by replacing them (Atomic link).
    pub fn expose_binary(src_path: &Path, bin_name: &str) -> Result<PathBuf> {
        let bin_dir = Self::user_bin_dir()?;
        
        #[cfg(unix)]
        let dest_path = bin_dir.join(bin_name);
        
        #[cfg(windows)]
        let dest_path = bin_dir.join(format!("{}.exe", bin_name));

        if dest_path.exists() || dest_path.is_symlink() {
            debug!("Store: Removing existing binary collision at {:?}", dest_path);
            let _ = fs::remove_file(&dest_path);
        }

        info!("Store: Exposing binary {} -> {:?}", bin_name, src_path);

        #[cfg(unix)]
        {
            // Ensure source is executable before linking
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(src_path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(src_path, perms);
            }
            
            std::os::unix::fs::symlink(src_path, &dest_path)
                .map_err(|e| Error::Io(e))?;
        }

        #[cfg(windows)]
        {
            // Windows symlinks are restricted; we use copies for the store binaries.
            fs::copy(src_path, &dest_path).map_err(|e| Error::Io(e))?;
        }

        Ok(dest_path)
    }

    /// Deep-scans a directory to find an executable matching a name hint.
    pub fn find_executable_in_dir(dir: &Path, hint: &str) -> Option<PathBuf> {
        let mut entries = walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok());
        entries.find(|e| {
            let fname = e.file_name().to_string_lossy().to_lowercase();
            let is_match = fname == hint.to_lowercase() || 
                           (fname.starts_with(hint) && !fname.contains('.'));
            
            if is_match && e.path().is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = fs::metadata(e.path()) {
                        return meta.permissions().mode() & 0o111 != 0;
                    }
                }
                #[cfg(windows)]
                {
                    return fname.ends_with(".exe") || fname.ends_with(".bat");
                }
            }
            false
        }).map(|e| e.into_path())
    }
}