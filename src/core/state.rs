use crate::core::{Result, Error};
use crate::utils::file::atomic_write;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::OnceLock;
use tracing::{debug, info, warn, trace, error};

/// Global override for the registry path.
/// Primarily used to redirect state I/O into a TempDir during integration tests
/// to prevent corruption of the production environment.
static TEST_REGISTRY_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Represents preserved metadata for a package that is no longer on the system.
/// 
/// This allows LiNix to "undo" a removal or track the history of a package's
/// movement between backends (Teleportation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostMetadata {
    /// The unique identifier of the backend that last owned this package.
    pub backend: String,
    /// The set of user-defined options (e.g., '@version=1.2.3') at time of removal.
    pub options: HashMap<String, String>,
    /// Backend-specific technical properties (e.g., 'install_path').
    pub properties: HashMap<String, String>,
    /// Meta-dependency requirements declared by the user in the manifest.
    pub requires: Vec<String>,
    /// Unix timestamp recording the exact moment of removal from the OS.
    pub removed_at: u64,
    /// If the package was moved via the 'Teleport' command, this stores the destination.
    pub teleported_to: Option<String>,
}

/// Represents a package actively managed by LiNix.
/// 
/// Modernized for v3.6.0: This structure is the single source of truth for
/// scoped upgrades, timed expirations, and ephemeral session cleanup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPackage {
    /// The unique name of the package within its backend (e.g. "ripgrep").
    pub name: String,
    /// The unique identifier for the backend (e.g. "apt", "cargo").
    pub backend: String,
    /// The specific version currently installed on the host.
    pub version: Option<String>,
    /// Unix timestamp of the initial installation date.
    pub installed_at: u64,
    /// Feature 7: Unix timestamp for lease expiration. If reached, the package
    /// is considered an orphan and scheduled for removal in the next sync.
    pub expires_at: Option<u64>,
    /// Stores custom user options applied during installation.
    pub options: HashMap<String, String>,
    /// Feature 3: Tracks the origin of the package (e.g. "profile:work", "module:dev").
    /// Essential for Feature 4 targeted upgrades.
    pub source: Option<String>,
    /// Feature 6: If true, this package is considered transient and will be 
    /// purged when the associated shell session terminates.
    pub is_transient: bool,
    /// Feature 6: The ID of the ephemeral shell session that spawned this package.
    pub session_id: Option<String>,
}

/// The Mission-Critical State Registry.
/// 
/// This is the "System Brain". It maintains the difference between what 
/// is on the OS and what LiNix is responsible for. 
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateRegistry {
    /// List of all packages under LiNix management.
    pub packages: Vec<ManagedPackage>,
    /// A historical archive of removed packages, keyed by "backend:package_name".
    pub ghosts: HashMap<String, GhostMetadata>,
    /// Feature 6: The ID of the currently active ephemeral shell session.
    /// Used to tag newly installed packages as transient.
    pub active_session_id: Option<String>,
}

impl StateRegistry {
    /// Overrides the path where the registry is saved (Testing Only).
    pub fn set_test_path(path: PathBuf) {
        if let Err(existing) = TEST_REGISTRY_PATH.set(path) {
            warn!("StateRegistry: Attempted to set test path to {:?}, but it was already set to {:?}", 
                  TEST_REGISTRY_PATH.get(), existing);
        }
    }

    /// Loads the state from disk.
    /// Note: This is a synchronous operation intended to be wrapped in tokio::task::spawn_blocking.
    pub fn load() -> Result<Self> {
        let path = Self::get_path();
        debug!("StateRegistry: Loading mission-critical state from {:?}", path);

        if !path.exists() {
            info!("StateRegistry: No state file found at {:?}. Initializing empty registry.", path);
            return Ok(Self::default());
        }

        let data = std::fs::read_to_string(&path).map_err(|e| {
            Error::Io(format!("Registry Read Error at {:?}: {}", path, e))
        })?;
        
        if data.trim().is_empty() {
            trace!("StateRegistry: State file is empty, returning default.");
            return Ok(Self::default());
        }

        let registry: Self = serde_json::from_str(&data).map_err(|e| {
            Error::Other(format!("Registry Corruption at {:?}: {}", path, e))
        })?;

        debug!("StateRegistry: Successfully loaded {} managed packages and {} ghosts.", 
               registry.packages.len(), registry.ghosts.len());
        Ok(registry)
    }

    /// Persists the registry to disk using an atomic write pattern.
    /// This ensures state integrity even during sudden power failure or crash.
    pub fn save(&self) -> Result<()> {
        let path = Self::get_path();
        trace!("StateRegistry: Initiating atomic save to {:?}", path);

        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Error::Io(format!("Failed to create registry directory: {}", e))
                })?;
            }
        }

        let data = serde_json::to_string_pretty(self).map_err(|e| {
            Error::Other(format!("State Serialization Error: {}", e))
        })?;

        atomic_write(&path, &data).map_err(|e| {
            Error::Persist(format!("Atomic write failed for state registry: {}", e))
        })
    }

    /// Primary method to add a package to management with full v3.6.0 metadata.
    /// 
    /// This method automatically handles:
    /// 1. Expiration calculation (Leases).
    /// 2. Session ID tagging (Transient environments).
    /// 3. Duplicate removal (Unique Backend + Name).
    /// 4. Ghost cleanup.
    pub fn add(
        &mut self, 
        backend: &str, 
        name: &str, 
        version: Option<String>, 
        options: HashMap<String, String>,
        source: Option<String>,
        is_transient: bool,
    ) {
        // Feature 7: Auto-calculate expiration from lease/duration options
        let expires_at = options.get("lease")
            .or_else(|| options.get("duration"))
            .and_then(|l| Self::parse_duration(l));

        // Feature 6: Associate with active session if transient
        let session_id = if is_transient { self.active_session_id.clone() } else { None };

        // Maintenance: Remove any existing entry for this package (Unique key: Backend + Name)
        self.packages.retain(|p| !(p.backend == backend && p.name == name));
        
        // Maintenance: Remove from ghosts if returning from the dead
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

    /// Modernized convenience wrapper for simple imperative installs.
    /// Resolves the missing method error in bridge.rs and migrate.rs.
    pub fn add_simple(&mut self, backend: &str, name: &str, version: Option<String>) {
        self.add(backend, name, version, HashMap::new(), None, false);
    }

    /// Update an existing lease for a package (Bug Fix 8).
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

    /// Removes a package and captures its "Ghost" metadata for historical tracking.
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

    /// Feature 7: Find all packages whose timed leases have expired.
    pub fn get_expired_packages(&self) -> Vec<(String, String)> {
        let now = Self::now();
        self.packages.iter()
            .filter(|p| {
                match p.expires_at {
                    Some(expiry) => now >= expiry,
                    None => false
                }
            })
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect()
    }

    /// Feature 6: Find all transient packages for a specific session.
    pub fn get_transient_packages(&self, session_id: &str) -> Vec<(String, String)> {
        trace!("StateRegistry: Scanning for transient packages in session '{}'", session_id);
        self.packages.iter()
            .filter(|p| p.is_transient && p.session_id.as_deref() == Some(session_id))
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect()
    }

    /// Checks if a package is currently registered as managed.
    pub fn is_managed(&self, backend: &str, name: &str) -> bool {
        self.packages.iter().any(|p| p.backend == backend && p.name == name)
    }

    /// Helper to find a specific managed package.
    pub fn get_package(&self, backend: &str, name: &str) -> Option<&ManagedPackage> {
        self.packages.iter().find(|p| p.backend == backend && p.name == name)
    }

    /// Feature 7: Shorthand duration parser (e.g. 1h, 30m, 10s, 7d).
    fn parse_duration(duration_str: &str) -> Option<u64> {
        if duration_str.is_empty() { return None; }
        
        let unit = duration_str.chars().last()?;
        let val_part = &duration_str[..duration_str.len() - 1];
        let value: u64 = match val_part.parse() {
            Ok(v) => v,
            Err(_) => {
                warn!("StateRegistry: Failed to parse numeric value from duration '{}'", duration_str);
                return None;
            }
        };
        
        let seconds = match unit {
            's' => value,
            'm' => value * 60,
            'h' => value * 3600,
            'd' => value * 86400,
            _ => {
                warn!("StateRegistry: Invalid duration unit '{}' in '{}'", unit, duration_str);
                return None;
            }
        };
        
        Some(Self::now() + seconds)
    }

    /// Returns the current Unix timestamp in seconds.
    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }

    /// Resolves the absolute path for the registry file, respecting test overrides.
    pub fn get_path() -> PathBuf {
        if let Some(path) = TEST_REGISTRY_PATH.get() {
            return path.clone();
        }
        crate::utils::safe_data_dir().join("registry.json")
    }
}