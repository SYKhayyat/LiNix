use crate::core::{Result, Error};
use crate::utils::file::atomic_write;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::OnceLock;

/// Global override for the registry path, used exclusively for integration testing
/// to prevent polluting the user's real package database during automated runs.
static TEST_REGISTRY_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Represents preserved metadata for a package that is no longer present on the system.
/// 
/// Fulfills Phase 7.2: Documentation and metadata integrity. 
/// This structure ensures that even when a package is removed, LiNix retains the 
/// knowledge of its original configuration, allowing for "Restore" or "Undo" operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostMetadata {
    /// The backend that originally managed this package (e.g., "apt", "cargo").
    pub backend: String,
    /// User-defined options applied during the last installation (e.g., version constraints).
    pub options: HashMap<String, String>,
    /// Technical properties discovered by the backend (e.g., specific install paths or IDs).
    pub properties: HashMap<String, String>,
    /// Declared dependencies (meta-requirements) at the time of removal.
    pub requires: Vec<String>,
    /// Unix timestamp (seconds) when the package was removed from active management.
    pub removed_at: u64,
    /// If the package was moved to another backend via the 'Teleport' command, 
    /// this field stores the destination backend identifier. 
    /// This is vital for debugging cross-backend state transitions.
    pub teleported_to: Option<String>,
}

/// Represents a package that is actively managed by LiNix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPackage {
    pub name: String,
    pub backend: String,
    pub version: Option<String>,
    pub installed_at: u64,
    /// Roadmap Point 15: Unix timestamp after which the package is considered expired.
    /// Used for temporary development dependencies or limited-time leases.
    pub expires_at: Option<u64>,
    /// Stores custom user options applied during installation.
    pub options: HashMap<String, String>,
}

/// The Mission-Critical State Registry for LiNix v3.5.0.
/// Tracks current managed state, expired leases, and "ghost" metadata for 
/// historical consistency.
/// 
/// This is the SINGLE source of truth for the local system state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateRegistry {
    /// Packages currently active and managed on the host.
    pub packages: Vec<ManagedPackage>,
    /// Archived metadata for removed packages, keyed by "backend:package_name".
    pub ghosts: HashMap<String, GhostMetadata>,
}

impl StateRegistry {
    /// Allows tests to redirect registry I/O to a temporary location.
    /// Fulfills Phase 3.2: Prevents collision in high-concurrency test environments.
    pub fn set_test_path(path: PathBuf) {
        let _ = TEST_REGISTRY_PATH.set(path);
    }

    /// Loads the state registry from the standard data directory or the test override path.
    /// Note: This is a synchronous operation intended to be wrapped in `tokio::task::spawn_blocking`.
    pub fn load() -> Result<Self> {
        let path = Self::get_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(&path).map_err(|e| {
            Error::Io(format!("Failed to read state registry at {:?}: {}", path, e))
        })?;
        
        if data.trim().is_empty() {
            return Ok(Self::default());
        }

        serde_json::from_str(&data).map_err(|e| {
            Error::Other(format!("State Registry at {:?} is corrupted: {}", path, e))
        })
    }

    /// Creates a registry instance from a specific path.
    /// Fulfills Phase 9.2: Enables isolated state testing.
    pub fn with_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(path).map_err(Error::from)?;
        serde_json::from_str(&data).map_err(Error::from)
    }

    /// Persists the registry to disk using an atomic write.
    /// Ensures that system crashes during the write do not corrupt the existing state.
    pub fn save(&self) -> Result<()> {
        let path = Self::get_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::from)?;
        }
        let data = serde_json::to_string_pretty(self).map_err(|e| Error::Other(e.to_string()))?;
        atomic_write(&path, &data)
    }

    /// Adds a package to the managed list.
    /// Handles Roadmap Point 15: If a TTL is provided (e.g. "@lease=2h"), sets an expiry.
    pub fn add(&mut self, backend: &str, name: &str, version: Option<String>, options: HashMap<String, String>) {
        // Calculate expiration if TTL is present in options
        let expires_at = options.get("lease").and_then(|l| Self::parse_duration(l));

        // Remove from ghosts if the package is returning to active state
        self.ghosts.remove(name);

        self.remove(backend, name); // Prevent duplicate entries
        self.packages.push(ManagedPackage {
            name: name.to_string(),
            backend: backend.to_string(),
            version,
            installed_at: Self::now(),
            expires_at,
            options,
        });
    }

    /// Adds a package with default empty options.
    pub fn add_simple(&mut self, backend: &str, name: &str, version: Option<String>) {
        self.add(backend, name, version, HashMap::new());
    }

    /// Removes a package from active management and archives it as a "Ghost".
    pub fn remove(&mut self, backend: &str, name: &str) {
        if let Some(pos) = self.packages.iter().position(|p| p.backend == backend && p.name == name) {
            let pkg = self.packages.remove(pos);
            
            // Archive to ghosts for historical tracking and teleport debugging
            self.ghosts.insert(name.to_string(), GhostMetadata {
                backend: backend.to_string(),
                options: pkg.options,
                properties: HashMap::new(), 
                requires: Vec::new(),
                removed_at: Self::now(),
                teleported_to: None,
            });
        }
    }

    /// Identifies packages whose time-limited leases have expired.
    pub fn get_expired_packages(&self) -> Vec<(String, String)> {
        let now = Self::now();
        self.packages.iter()
            .filter(|p| p.expires_at.map_or(false, |expiry| now > expiry))
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect()
    }

    /// Checks if a package is currently registered as managed.
    pub fn is_managed(&self, backend: &str, name: &str) -> bool {
        self.packages.iter().any(|p| p.backend == backend && p.name == name)
    }

    /// Returns a reference to a managed package if found.
    pub fn get_package(&self, backend: &str, name: &str) -> Option<&ManagedPackage> {
        self.packages.iter().find(|p| p.backend == backend && p.name == name)
    }

    /// Gets ghost metadata for a removed package if it exists.
    pub fn get_ghost(&self, name: &str) -> Option<&GhostMetadata> {
        self.ghosts.get(name)
    }

    /// Returns all archived ghost entries.
    pub fn list_ghosts(&self) -> Vec<(String, &GhostMetadata)> {
        self.ghosts.iter().map(|(k, v)| (k.clone(), v)).collect()
    }

    /// Clears ghost entries older than the provided timestamp.
    pub fn cleanup_ghosts(&mut self, older_than: u64) {
        self.ghosts.retain(|_, v| v.removed_at >= older_than);
    }

    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }

    /// Parses duration shorthand strings (e.g., "2h", "30m", "1d").
    fn parse_duration(duration_str: &str) -> Option<u64> {
        let unit = duration_str.chars().last()?;
        let val_str = &duration_str[..duration_str.len()-1];
        let value: u64 = val_str.parse().ok()?;
        
        let seconds = match unit {
            's' => value,
            'm' => value * 60,
            'h' => value * 3600,
            'd' => value * 86400,
            _ => return None,
        };
        
        Some(Self::now() + seconds)
    }

    /// Returns the active filesystem path for the registry, respecting test overrides.
    pub fn get_path() -> PathBuf {
        if let Some(path) = TEST_REGISTRY_PATH.get() {
            return path.clone();
        }
        crate::utils::safe_data_dir().join("registry.json")
    }
}