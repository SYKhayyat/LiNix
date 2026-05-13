use crate::core::{Result, Error};
use crate::utils::file::atomic_write;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Represents a package that is actively managed by LiNix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPackage {
    pub name: String,
    pub backend: String,
    pub version: Option<String>,
    pub installed_at: u64,
}

/// The local source of truth for the system's managed state.
/// Tracks which packages were installed via LiNix to distinguish them 
/// from packages installed manually by the user or the OS.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateRegistry {
    /// List of packages successfully managed by LiNix.
    pub packages: Vec<ManagedPackage>,
}

impl StateRegistry {
    /// Loads the state registry from the standard data directory.
    /// Returns a default empty registry if the file does not exist.
    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(&path)?;
        serde_json::from_str(&data).map_err(|e| {
            Error::Other(format!("State Registry is corrupted: {}", e))
        })
    }

    /// Persists the registry to disk using an atomic write to prevent corruption.
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self).map_err(|e| Error::Other(e.to_string()))?;
        atomic_write(&path, &data)
    }

    /// Adds a package to the managed list. Idempotent.
    pub fn add(&mut self, backend: &str, name: &str, version: Option<String>) {
        if !self.is_managed(backend, name) {
            self.packages.push(ManagedPackage {
                name: name.to_string(),
                backend: backend.to_string(),
                version,
                installed_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });
        }
    }

    /// Removes a package from the managed list.
    pub fn remove(&mut self, backend: &str, name: &str) {
        self.packages.retain(|p| !(p.backend == backend && p.name == name));
    }

    /// Checks if a package is currently registered as managed.
    pub fn is_managed(&self, backend: &str, name: &str) -> bool {
        self.packages.iter().any(|p| p.backend == backend && p.name == name)
    }

    /// Determines the platform-specific path for the registry file.
    fn path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("registry.json")
    }
}