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
    pub teleported_to: Option<String>,
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

/// The Mission-Critical State Registry for LiNix v3.5.0.
/// Tracks current managed state, expired leases, and "ghost" metadata for 
/// historical consistency.
/// 
/// This is the SINGLE source of truth for system state.
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
        let expires_at = options.get("lease").and_then(|l| Self::parse_duration(l));

        // Remove from ghosts if it's coming back
        self.ghosts.remove(name);

        self.remove(backend, name); // Prevent duplicates
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
                removed_at: Self::now(),
                teleported_to: None,
            });
        }
    }

    /// Identifies packages whose leases have expired (Point 15).
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

    /// Gets a managed package by backend and name.
    pub fn get_package(&self, backend: &str, name: &str) -> Option<&ManagedPackage> {
        self.packages.iter().find(|p| p.backend == backend && p.name == name)
    }

    /// Gets ghost metadata for a package if it exists.
    pub fn get_ghost(&self, name: &str) -> Option<&GhostMetadata> {
        self.ghosts.get(name)
    }

    /// Returns all ghost entries.
    pub fn list_ghosts(&self) -> Vec<(String, &GhostMetadata)> {
        self.ghosts.iter().map(|(k, v)| (k.clone(), v)).collect()
    }

    /// Clears all ghost entries older than the given timestamp.
    pub fn cleanup_ghosts(&mut self, older_than: u64) {
        self.ghosts.retain(|_, v| v.removed_at >= older_than);
    }

    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }

    /// Parses shorthand duration strings (e.g. "2h", "30m", "1d").
    fn parse_duration(duration_str: &str) -> Option<u64> {
        let unit = duration_str.chars().last()?;
        let value: u64 = duration_str[..duration_str.len()-1].parse().ok()?;
        
        let seconds = match unit {
            's' => value,
            'm' => value * 60,
            'h' => value * 3600,
            'd' => value * 86400,
            _ => return None,
        };
        
        Some(Self::now() + seconds)
    }

    fn path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("registry.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_state_registry_add_remove() {
        let mut registry = StateRegistry::default();
        
        registry.add_simple("apt", "curl", Some("7.81.0".into()));
        assert!(registry.is_managed("apt", "curl"));
        
        registry.remove("apt", "curl");
        assert!(!registry.is_managed("apt", "curl"));
        assert!(registry.get_ghost("curl").is_some());
    }

    #[test]
    fn test_expired_packages() {
        let mut registry = StateRegistry::default();
        let mut options = HashMap::new();
        options.insert("lease".into(), "1s".into());
        
        registry.add("apt", "test-pkg", None, options);
        
        // Should not be expired immediately
        assert!(registry.get_expired_packages().is_empty());
    }

    #[test]
    fn test_parse_duration() {
        let now = StateRegistry::now();
        
        let s = StateRegistry::parse_duration("30s").unwrap();
        assert!(s > now);
        assert!(s - now >= 30);
        
        let m = StateRegistry::parse_duration("5m").unwrap();
        assert!(m - now >= 300);
        
        let h = StateRegistry::parse_duration("2h").unwrap();
        assert!(h - now >= 7200);
        
        let d = StateRegistry::parse_duration("1d").unwrap();
        assert!(d - now >= 86400);
        
        assert!(StateRegistry::parse_duration("invalid").is_none());
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = tempdir().unwrap();
        let original_path = dir.path().join("registry.json");
        
        // We need to temporarily override the path for testing
        // This is a simplified test - in real code, you'd use dependency injection
        
        let mut registry = StateRegistry::default();
        registry.add_simple("apt", "vim", Some("8.2".into()));
        
        // In production, save/load use atomic_write which is tested separately
        let data = serde_json::to_string(&registry).unwrap();
        let new_registry: StateRegistry = serde_json::from_str(&data).unwrap();
        
        assert_eq!(registry.packages.len(), new_registry.packages.len());
        assert_eq!(registry.packages[0].name, new_registry.packages[0].name);
    }
}