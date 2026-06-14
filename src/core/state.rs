// src/core/state.rs

use crate::core::{Error, Result};
use crate::utils::file::atomic_write;
use crate::utils::safe_data_dir;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, trace, error};  // removed unused `warn`

// ============================================================================
// GhostMetadata (unchanged)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostMetadata {
    pub backend: String,
    pub options: HashMap<String, String>,
    pub properties: HashMap<String, String>,
    pub requires: Vec<String>,
    pub removed_at: u64,
    pub teleported_to: Option<String>,
}

// ============================================================================
// ManagedPackage (unchanged)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPackage {
    pub name: String,
    pub backend: String,
    pub version: Option<String>,
    pub installed_at: u64,
    pub expires_at: Option<u64>,
    pub options: HashMap<String, String>,
    pub source: Option<String>,
    pub is_transient: bool,
    pub session_id: Option<String>,
}

// ============================================================================
// StateRegistry (now path‑aware, no static test path)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateRegistry {
    /// Path to the registry file on disk.
    #[serde(skip)]
    pub path: PathBuf,
    /// List of all packages under LiNix management.
    pub packages: Vec<ManagedPackage>,
    /// Historical archive of removed packages.
    pub ghosts: HashMap<String, GhostMetadata>,
    /// ID of the currently active ephemeral shell session.
    pub active_session_id: Option<String>,
}

impl StateRegistry {
    /// Creates a new empty registry associated with a specific file path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            packages: Vec::new(),
            ghosts: HashMap::new(),
            active_session_id: None,
        }
    }

    /// Loads a registry from the given path.
    pub fn load_from(path: &Path) -> Result<Self> {
        debug!("StateRegistry: Loading mission-critical state from {:?}", path);

        if !path.exists() {
            info!("StateRegistry: No state file found at {:?}. Initializing empty registry.", path);
            return Ok(Self::new(path.to_path_buf()));
        }

        let data = std::fs::read_to_string(path).map_err(|e| {
            Error::Io(format!("Registry Read Error at {:?}: {}", path, e))
        })?;

        if data.trim().is_empty() {
            trace!("StateRegistry: State file is empty, returning default.");
            return Ok(Self::new(path.to_path_buf()));
        }

        let mut registry: Self = serde_json::from_str(&data).map_err(|e| {
            Error::Other(format!("Registry Corruption at {:?}: {}", path, e))
        })?;

        // Ensure the loaded registry has the correct path.
        registry.path = path.to_path_buf();

        debug!("StateRegistry: Successfully loaded {} managed packages and {} ghosts.",
               registry.packages.len(), registry.ghosts.len());
        Ok(registry)
    }

    /// Loads the registry from the default data directory.
    /// This replaces the old static `load()` method.
    pub fn load_default() -> Result<Self> {
        let default_path = safe_data_dir().join("registry.json");
        Self::load_from(&default_path)
    }

    /// Saves the registry to its associated file path.
    pub fn save(&self) -> Result<()> {
        trace!("StateRegistry: Initiating atomic save to {:?}", self.path);

        if let Some(parent) = self.path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Error::Io(format!("Failed to create registry directory: {}", e))
                })?;
            }
        }

        let data = serde_json::to_string_pretty(self).map_err(|e| {
            Error::Other(format!("State Serialization Error: {}", e))
        })?;

        atomic_write(&self.path, &data).map_err(|e| {
            Error::Persist(format!("Atomic write failed for state registry: {}", e))
        })
    }

    /// Adds a package to management with full metadata.
    pub fn add(
        &mut self,
        backend: &str,
        name: &str,
        version: Option<String>,
        options: HashMap<String, String>,
        source: Option<String>,
        is_transient: bool,
    ) {
        let expires_at = options.get("lease")
            .or_else(|| options.get("duration"))
            .and_then(|l| Self::parse_duration(l));

        let session_id = if is_transient { self.active_session_id.clone() } else { None };

        self.packages.retain(|p| !(p.backend == backend && p.name == name));
        let ghost_key = format!("{}:{}", backend, name);
        self.ghosts.remove(&ghost_key);

        let new_pkg = ManagedPackage {
            name: name.to_string(),
            backend: backend.to_string(),
            version,
            installed_at: Self::now(),
            expires_at,
            options,
            source,
            is_transient,
            session_id,
        };

        trace!("StateRegistry: Finalizing addition of {}:{} (Source: {:?}, Transient: {})",
               backend, name, new_pkg.source, is_transient);

        self.packages.push(new_pkg);
        debug!("StateRegistry: Package {}:{} is now under management.", backend, name);
    }

    /// Convenience wrapper for simple imperative installs.
    pub fn add_simple(&mut self, backend: &str, name: &str, version: Option<String>) {
        self.add(backend, name, version, HashMap::new(), None, false);
    }

    /// Updates an existing lease.
    pub fn update_lease(&mut self, backend: &str, name: &str, duration_str: &str) -> Result<()> {
        let expiry = Self::parse_duration(duration_str)
            .ok_or_else(|| Error::Validation(format!("Invalid duration format: '{}'. Use 30d, 2h, etc.", duration_str)))?;

        if let Some(pkg) = self.packages.iter_mut().find(|p| p.backend == backend && p.name == name) {
            pkg.expires_at = Some(expiry);
            pkg.options.insert("lease".to_string(), duration_str.to_string());
            info!("StateRegistry: Updated lease for {}:{} -> Expires at Unix {}", backend, name, expiry);
            Ok(())
        } else {
            error!("StateRegistry: Attempted to set lease for unmanaged package {}:{}", backend, name);
            Err(Error::PackageNotFound(format!("{}:{}", backend, name)))
        }
    }

    /// Removes a package and archives it as a ghost.
    pub fn remove(&mut self, backend: &str, name: &str) {
        if let Some(pos) = self.packages.iter().position(|p| p.backend == backend && p.name == name) {
            let pkg = self.packages.remove(pos);

            let ghost_key = format!("{}:{}", backend, name);
            self.ghosts.insert(ghost_key, GhostMetadata {
                backend: backend.to_string(),
                options: pkg.options,
                properties: HashMap::new(),
                requires: Vec::new(),
                removed_at: Self::now(),
                teleported_to: None,
            });
            debug!("StateRegistry: Package {}:{} archived as Ghost.", backend, name);
        } else {
            trace!("StateRegistry: Requested removal of {}:{} but it was not managed.", backend, name);
        }
    }

    /// Returns packages whose leases have expired.
    pub fn get_expired_packages(&self) -> Vec<(String, String)> {
        let now = Self::now();
        self.packages.iter()
            .filter(|p| p.expires_at.map_or(false, |expiry| now >= expiry))
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect()
    }

    /// Returns transient packages for a given session ID.
    pub fn get_transient_packages(&self, session_id: &str) -> Vec<(String, String)> {
        trace!("StateRegistry: Scanning for transient packages in session '{}'", session_id);
        self.packages.iter()
            .filter(|p| p.is_transient && p.session_id.as_deref() == Some(session_id))
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect()
    }

    /// Checks if a package is managed.
    pub fn is_managed(&self, backend: &str, name: &str) -> bool {
        self.packages.iter().any(|p| p.backend == backend && p.name == name)
    }

    /// Returns a reference to a managed package if present.
    pub fn get_package(&self, backend: &str, name: &str) -> Option<&ManagedPackage> {
        self.packages.iter().find(|p| p.backend == backend && p.name == name)
    }

    /// Parses a duration string (e.g., "30d", "2h") into a future Unix timestamp.
    fn parse_duration(duration_str: &str) -> Option<u64> {
        if duration_str.is_empty() { return None; }
        let unit = duration_str.chars().last()?;
        let val_part = &duration_str[..duration_str.len() - 1];
        let value: u64 = val_part.parse().ok()?;
        let seconds = match unit {
            's' => value,
            'm' => value * 60,
            'h' => value * 3600,
            'd' => value * 86400,
            _ => return None,
        };
        Some(Self::now() + seconds)
    }

    /// Returns current Unix timestamp in seconds.
    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }
}

// ============================================================================
// Default implementation (uses default data directory)
// ============================================================================

impl Default for StateRegistry {
    fn default() -> Self {
        let default_path = safe_data_dir().join("registry.json");
        Self::new(default_path)
    }
}