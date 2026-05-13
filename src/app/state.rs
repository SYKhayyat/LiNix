use crate::core::{Result, Error};
use crate::utils::file::atomic_write;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Represents preserved metadata for a package that is no longer present (Point 14).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostMetadata {
    pub backend: String,
    pub options: HashMap<String, String>,
    pub properties: HashMap<String, String>,
    pub requires: Vec<String>,
    pub removed_at: u64,
}

/// Represents a package that is actively managed by LiNix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPackage {
    pub name: String,
    pub backend: String,
    pub version: Option<String>,
    pub installed_at: u64,
    /// Roadmap Point 15: Timestamp after which the package is considered expired.
    pub expires_at: Option<u64>,
    /// Stores custom user options applied during installation.
    pub options: HashMap<String, String>,
}

/// The Mission-Critical State Registry for LiNix v3.3.0.
/// Tracks current managed state, expired leases, and "ghost" metadata for 
/// historical consistency.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateRegistry {
    /// Packages currently active on the host.
    pub packages: Vec<ManagedPackage>,
    /// Archived metadata for removed packages (Point 14).
    pub ghosts: HashMap<String, GhostMetadata>,
}

impl StateRegistry {
    /// Loads the state registry from the standard data directory.
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

    /// Persists the registry to disk using an atomic write.
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self).map_err(|e| Error::Other(e.to_string()))?;
        atomic_write(&path, &data)
    }

    /// Adds a package to the managed list.
    /// Handles Roadmap Point 15: If a TTL is provided, sets an expiration timestamp.
    pub fn add(&mut self, backend: &str, name: &str, version: Option<String>, options: HashMap<String, String>) {
        // Calculate expiration if TTL is present in options (e.g. "lease=2h")
        let expires_at = options.get("lease").and_then(|l| self.parse_duration(l));

        // Remove from ghosts if it's coming back
        self.ghosts.remove(name);

        self.remove(backend, name); // Prevent duplicates
        self.packages.push(ManagedPackage {
            name: name.to_string(),
            backend: backend.to_string(),
            version,
            installed_at: self.now(),
            expires_at,
            options,
        });
    }

    /// Removes a package and archives it as a "Ghost" (Point 14).
    pub fn remove(&mut self, backend: &str, name: &str) {
        if let Some(pos) = self.packages.iter().position(|p| p.backend == backend && p.name == name) {
            let pkg = self.packages.remove(pos);
            
            // Archive to ghosts
            self.ghosts.insert(name.to_string(), GhostMetadata {
                backend: backend.to_string(),
                options: pkg.options,
                properties: HashMap::new(), // Populated by backends during execution
                requires: Vec::new(),
                removed_at: self.now(),
            });
        }
    }

    /// Identifies packages whose leases have expired (Point 15).
    pub fn get_expired_packages(&self) -> Vec<(String, String)> {
        let now = self.now();
        self.packages.iter()
            .filter(|p| p.expires_at.map_or(false, |expiry| now > expiry))
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect()
    }

    pub fn is_managed(&self, backend: &str, name: &str) -> bool {
        self.packages.iter().any(|p| p.backend == backend && p.name == name)
    }

    fn now(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }

    /// Parses shorthand duration strings (e.g. "2h", "30m", "1d").
    fn parse_duration(&self, duration_str: &str) -> Option<u64> {
        let unit = duration_str.chars().last()?;
        let value: u64 = duration_str[..duration_str.len()-1].parse().ok()?;
        
        let seconds = match unit {
            's' => value,
            'm' => value * 60,
            'h' => value * 3600,
            'd' => value * 86400,
            _ => return None,
        };
        
        Some(self.now() + seconds)
    }

    fn path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("registry.json")
    }
}