// src/core/state.rs
use crate::core::{Result, Error};
use crate::utils::file::atomic_write;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, info};

/// Represents a package that LiNix has explicitly installed and currently "owns."
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManagedPackage {
    pub name: String,
    pub backend: String,
    pub version: Option<String>,
    pub installed_at: u64,
}

/// The main database for LiNix state. 
/// Tracks ownership and in-progress actions (Journaling).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateRegistry {
    /// Packages successfully installed and managed by LiNix.
    pub packages: Vec<ManagedPackage>,
    
    /// The Journal: Actions currently in progress. 
    /// Format: (BackendName, PackageName, IsInstalling)
    /// If this is not empty on startup, it means the last run crashed.
    pub pending_actions: Vec<(String, String, bool)>, 
}

impl StateRegistry {
    /// Loads the registry from the standard system data directory.
    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            debug!("No state registry found at {:?}, starting fresh.", path);
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(&path)
            .map_err(|e| Error::Io(e))?;
        
        serde_json::from_str(&data)
            .map_err(|e| Error::Other(format!("State Registry JSON is corrupted: {}", e)))
    }

    /// Persists the current state to disk atomically.
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        
        // Ensure the directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| Error::Io(e))?;
            }
        }

        let data = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Other(e.to_string()))?;

        // Uses the atomic_write utility to prevent partial file corruption
        atomic_write(&path, &data)
    }

    /// Mark the BEGINNING of a package operation in the journal.
    /// This ensures we can recover if the process is killed mid-install.
    pub fn journal_start(&mut self, backend: &str, name: &str, is_install: bool) -> Result<()> {
        debug!("Journaling start: {} via {} (install={})", name, backend, is_install);
        self.pending_actions.push((backend.to_string(), name.to_string(), is_install));
        self.save()
    }

    /// Mark the SUCCESSFUL END of an operation by removing it from the journal.
    pub fn journal_commit(&mut self, backend: &str, name: &str) -> Result<()> {
        debug!("Journaling commit: {} via {}", name, backend);
        self.pending_actions.retain(|(b, n, _)| !(b == backend && n == name));
        self.save()
    }

    /// Checks if a package is currently in the "Managed" list.
    pub fn is_managed(&self, backend: &str, name: &str) -> bool {
        self.packages.iter().any(|p| p.backend == backend && p.name == name)
    }

    /// Adds a package to the ownership list.
    pub fn add(&mut self, backend: &str, name: &str, version: Option<String>) {
        if !self.is_managed(backend, name) {
            self.packages.push(ManagedPackage {
                name: name.to_string(),
                backend: backend.to_string(),
                version,
                installed_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });
        }
    }

    /// Removes a package from the ownership list.
    pub fn remove(&mut self, backend: &str, name: &str) {
        self.packages.retain(|p| !(p.backend == backend && p.name == name));
    }

    /// Returns the standard path for the registry file.
    fn path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("registry.json")
    }
}