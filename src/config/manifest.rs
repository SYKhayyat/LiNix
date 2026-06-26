use crate::core::{Error, Result};
use crate::utils::file::atomic_write;
use std::path::{PathBuf};
use walkdir::WalkDir;
use tokio::fs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents the physical location of a package declaration within a text manifest.
#[derive(Debug, Clone)]
pub struct PackageLocation {
    pub file_path: PathBuf,
    pub line_index: usize,
    pub raw_line: String,
}

/// Phase 3.1: Machine-generated lock data to avoid race conditions and manifest corruption.
/// This separates user-intent (.txt) from machine-verified state (locks.json).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManifestLocks {
    /// Maps "backend:package" to a map of verified options (like sha256).
    pub locks: HashMap<String, HashMap<String, String>>,
}

/// The ManifestEngine coordinates reads and writes to declarative .txt files and the lock-staging area.
/// Hardened for Phase 3.1: Implements non-blocking I/O and staging-aware updates.
pub struct ManifestEngine {
    groups_dir: PathBuf,
    locks_path: PathBuf,
}

impl ManifestEngine {
    pub fn new(groups_dir: impl Into<PathBuf>) -> Self {
        let dir = groups_dir.into();
        let locks_path = dir.join("locks.json");
        Self {
            groups_dir: dir,
            locks_path,
        }
    }

    /// Recursively scans the groups directory for all declarations of a package.
    pub async fn find_all_packages(&self, package_name: &str) -> Result<Vec<PackageLocation>> {
        let mut locations = Vec::new();
        if !self.groups_dir.exists() {
            return Ok(locations);
        }

        let groups_dir = self.groups_dir.clone();
        let entries: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
            WalkDir::new(&groups_dir)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "txt"))
                .map(|e| e.path().to_path_buf())
                .collect()
        }).await.map_err(|e| Error::Other(e.to_string()))?;

        for path in entries {
            let content = fs::read_to_string(&path).await?;
            for (idx, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                
                let spec_part = trimmed.split('@').next().unwrap_or(trimmed);
                
                let is_match = if let Some((_backend, name)) = spec_part.split_once(':') {
                    name.trim() == package_name || spec_part.trim() == package_name
                } else {
                    spec_part.trim() == package_name
                };

                if is_match {
                    locations.push(PackageLocation {
                        file_path: path.clone(),
                        line_index: idx,
                        raw_line: line.to_string(),
                    });
                }
            }
        }
        Ok(locations)
    }

    /// Phase 3.1: Loads machine-generated locks from the staging area.
    pub async fn load_locks(&self) -> Result<ManifestLocks> {
        if !tokio::fs::try_exists(&self.locks_path).await.unwrap_or(false) {
            return Ok(ManifestLocks::default());
        }
        let data = fs::read_to_string(&self.locks_path).await?;
        if data.trim().is_empty() {
            return Ok(ManifestLocks::default());
        }
        serde_json::from_str(&data).map_err(Error::from)
    }

    /// Phase 3.1: Atomically updates a lock in the staging area.
    pub async fn update_lock(&self, backend: &str, package: &str, options: HashMap<String, String>) -> Result<()> {
        let mut locks = self.load_locks().await?;
        let key = format!("{}:{}", backend, package);
        locks.locks.insert(key, options);
        
        let data = serde_json::to_string_pretty(&locks).map_err(Error::from)?;
        let path = self.locks_path.clone();
        
        tokio::task::spawn_blocking(move || {
            atomic_write(&path, &data)
        }).await.map_err(|e| Error::Other(e.to_string()))??;
        
        Ok(())
    }

    /// Updates a package declaration with a new specification string.
    pub async fn update_package(&self, package_name: &str, new_spec: &str) -> Result<()> {
        let locations = self.find_all_packages(package_name).await?;
        let loc = locations.first().ok_or_else(|| Error::Config(format!("Package '{}' not found in manifests", package_name)))?;
        self.update_package_at_location(loc, new_spec).await
    }

    async fn update_package_at_location(&self, location: &PackageLocation, new_spec: &str) -> Result<()> {
        let content = fs::read_to_string(&location.file_path).await?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        
        if lines.len() > location.line_index {
            let leading_ws: String = location.raw_line.chars().take_while(|c| c.is_whitespace()).collect();
            lines[location.line_index] = format!("{}{}", leading_ws, new_spec);
            
            let output = lines.join("\n") + "\n";
            let path = location.file_path.clone();
            tokio::task::spawn_blocking(move || {
                atomic_write(&path, &output)
            }).await.map_err(|e| Error::Other(e.to_string()))??;
        }
        Ok(())
    }

    pub async fn delete_package(&self, package_name: &str) -> Result<usize> {
        let locations = self.find_all_packages(package_name).await?;
        let count = locations.len();
        for loc in &locations {
            self.delete_package_at_location(loc).await?;
        }
        Ok(count)
    }

    async fn delete_package_at_location(&self, location: &PackageLocation) -> Result<()> {
        let content = fs::read_to_string(&location.file_path).await?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        
        if lines.len() > location.line_index {
            lines.remove(location.line_index);
            let output = lines.join("\n").trim_end().to_string() + "\n";
            
            let path = location.file_path.clone();
            tokio::task::spawn_blocking(move || {
                atomic_write(&path, &output)
            }).await.map_err(|e| Error::Other(e.to_string()))??;
        }
        Ok(())
    }

    pub async fn add_to_local(&self, spec_str: &str) -> Result<()> {
        let local_path = self.groups_dir.join("local.txt");
        let name_part = spec_str.split('@').next().unwrap_or(spec_str);
        let clean_name = name_part.split_once(':').map(|(_, n)| n).unwrap_or(name_part).trim();
        
        if !self.find_all_packages(clean_name).await?.is_empty() {
            return Ok(());
        }

        if !self.groups_dir.exists() {
            fs::create_dir_all(&self.groups_dir).await?;
        }

        let mut lines = if tokio::fs::try_exists(&local_path).await.unwrap_or(false) {
            fs::read_to_string(&local_path).await?.lines().map(|s| s.to_string()).collect()
        } else {
            vec![
                "# LiNix Local Manifest".to_string(),
                "# Automatically managed imperative installations".to_string(),
                "".to_string(),
            ]
        };

        lines.push(spec_str.to_string());
        let output = lines.join("\n") + "\n";
        
        tokio::task::spawn_blocking(move || {
            atomic_write(&local_path, &output)
        }).await.map_err(|e| Error::Other(e.to_string()))??;
        
        Ok(())
    }

    pub async fn list_all_specs(&self) -> Result<Vec<String>> {
        let mut specs = Vec::new();
        if !self.groups_dir.exists() {
            return Ok(specs);
        }

        let groups_dir = self.groups_dir.clone();
        let entries: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
            WalkDir::new(&groups_dir)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "txt"))
                .map(|e| e.path().to_path_buf())
                .collect()
        }).await.map_err(|e| Error::Other(e.to_string()))?;

        for path in entries {
            let content = fs::read_to_string(path).await?;
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    specs.push(trimmed.to_string());
                }
            }
        }
        Ok(specs)
    }
}