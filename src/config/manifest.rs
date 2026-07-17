use crate::core::{Error, Result};
use crate::utils::file::atomic_write;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;
use walkdir::WalkDir;

/// Represents the physical location of a package declaration within a text manifest.
#[derive(Debug, Clone)]
pub struct PackageLocation {
    pub file_path: PathBuf,
    pub line_index: usize,
    pub raw_line: String,
}

/// Machine-generated lock data. Separates user intent (.txt) from machine-verified
/// state (locks.json).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManifestLocks {
    /// Maps "backend:package" to a map of verified options (like sha256).
    pub locks: HashMap<String, HashMap<String, String>>,
}

/// Reads and writes the declarative `.txt` manifests and the lock-staging area.
pub struct ManifestEngine {
    groups_dir: PathBuf,
    locks_path: PathBuf,
}

impl ManifestEngine {
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self::new(&config.groups_dir)
    }

    pub fn new(groups_dir: impl Into<PathBuf>) -> Self {
        let dir = groups_dir.into();
        let locks_path = dir.join("locks.json");
        Self {
            groups_dir: dir,
            locks_path,
        }
    }

    /// Sorted, not `WalkDir` order: filesystem order differs between machines holding
    /// identical files, and later lines override earlier ones.
    async fn manifest_files(&self) -> Result<Vec<PathBuf>> {
        let dirs = vec![self.groups_dir.clone()];
        let mut entries: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
            let mut out: Vec<PathBuf> = Vec::new();
            for dir in dirs {
                if !dir.exists() {
                    continue;
                }
                out.extend(
                    WalkDir::new(&dir)
                        .into_iter()
                        .filter_map(|e| e.ok())
                        .filter(|e| {
                            e.path().is_file()
                                && e.path().extension().is_some_and(|ext| ext == "txt")
                        })
                        .map(|e| e.path().to_path_buf()),
                );
            }
            out
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))?;
        entries.sort();
        entries.dedup();
        Ok(entries)
    }

    /// Scans every wish-list folder for all declarations of a package.
    pub async fn find_all_packages(&self, package_name: &str) -> Result<Vec<PackageLocation>> {
        let mut locations = Vec::new();
        let entries = self.manifest_files().await?;

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
        if !tokio::fs::try_exists(&self.locks_path)
            .await
            .unwrap_or(false)
        {
            return Ok(ManifestLocks::default());
        }
        let data = fs::read_to_string(&self.locks_path).await?;
        if data.trim().is_empty() {
            return Ok(ManifestLocks::default());
        }
        serde_json::from_str(&data).map_err(Error::from)
    }

    /// Phase 3.1: Atomically updates a lock in the staging area.
    pub async fn update_lock(
        &self,
        backend: &str,
        package: &str,
        options: HashMap<String, String>,
    ) -> Result<()> {
        let mut locks = self.load_locks().await?;
        let key = format!("{}:{}", backend, package);
        locks.locks.insert(key, options);

        let data = serde_json::to_string_pretty(&locks).map_err(Error::from)?;
        let path = self.locks_path.clone();

        tokio::task::spawn_blocking(move || atomic_write(&path, &data))
            .await
            .map_err(|e| Error::Other(e.to_string()))??;

        Ok(())
    }

    /// Updates a package declaration with a new specification string.
    pub async fn update_package(&self, package_name: &str, new_spec: &str) -> Result<()> {
        let locations = self.find_all_packages(package_name).await?;
        let loc = locations.first().ok_or_else(|| {
            Error::Config(format!("Package '{}' not found in manifests", package_name))
        })?;
        self.update_package_at_location(loc, new_spec).await
    }

    async fn update_package_at_location(
        &self,
        location: &PackageLocation,
        new_spec: &str,
    ) -> Result<()> {
        let content = fs::read_to_string(&location.file_path).await?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

        if lines.len() > location.line_index {
            let leading_ws: String = location
                .raw_line
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect();
            lines[location.line_index] = format!("{}{}", leading_ws, new_spec);

            let output = lines.join("\n") + "\n";
            let path = location.file_path.clone();
            tokio::task::spawn_blocking(move || atomic_write(&path, &output))
                .await
                .map_err(|e| Error::Other(e.to_string()))??;
        }
        Ok(())
    }

}
