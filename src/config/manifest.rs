use crate::core::{Result, Error};
use crate::utils::file::atomic_write;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use walkdir::WalkDir;

/// Represents the precise location of a package definition within the declarative group files.
#[derive(Debug, Clone)]
pub struct PackageLocation {
    pub file_path: PathBuf,
    pub line_index: usize,
    pub raw_line: String,
}

/// The Manifest Engine provides a round-trip orchestration layer for group files (.txt).
/// It allows LiNix to modify the declarative source-of-truth while preserving 
/// file structure, whitespace, and user comments.
pub struct ManifestEngine {
    groups_dir: PathBuf,
}

impl ManifestEngine {
    pub fn new(groups_dir: impl Into<PathBuf>) -> Self {
        Self {
            groups_dir: groups_dir.into(),
        }
    }

    /// Locates a package by name across all manifest files in the groups directory.
    /// Matches name exactly, even if the line contains options (@) or is prefixed with a backend.
    pub fn find_package(&self, package_name: &str) -> Result<Option<PackageLocation>> {
        debug!("ManifestEngine: Searching for package '{}' in {:?}", package_name, self.groups_dir);

        for entry in WalkDir::new(&self.groups_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file() && e.path().extension().map_or(false, |ext| ext == "txt"))
        {
            let content = fs::read_to_string(entry.path())?;
            for (idx, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                
                // Skip comments and empty lines
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }

                // Strip options if present (e.g., "apt:neovim@version=1.0" -> "apt:neovim")
                let spec_part = trimmed.split('@').next().unwrap_or(trimmed);
                
                // Check for exact name match or backend:name match
                let is_match = if spec_part.contains(':') {
                    let (_backend, name) = spec_part.split_once(':').unwrap();
                    name == package_name || spec_part == package_name
                } else {
                    spec_part == package_name
                };

                if is_match {
                    return Ok(Some(PackageLocation {
                        file_path: entry.path().to_path_buf(),
                        line_index: idx,
                        raw_line: line.to_string(),
                    }));
                }
            }
        }

        Ok(None)
    }

    /// Updates an existing package entry with a new specification string.
    /// This is used for "Auto-Locking" checksums or updating versions declaratively.
    pub fn update_package(&self, package_name: &str, new_spec: &str) -> Result<()> {
        let location = self.find_package(package_name)?
            .ok_or_else(|| Error::Config(format!("Package '{}' not found in manifests", package_name)))?;

        info!("ManifestEngine: Updating '{}' in {:?}", package_name, location.file_path);

        let content = fs::read_to_string(&location.file_path)?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

        if lines.len() > location.line_index {
            // Preserve leading whitespace of the original line if any
            let leading_ws = location.raw_line.chars()
                .take_while(|c| c.is_whitespace())
                .collect::<String>();
            
            lines[location.line_index] = format!("{}{}", leading_ws, new_spec);
            
            let new_content = lines.join("\n") + "\n";
            atomic_write(&location.file_path, &new_content)?;
            return Ok(());
        }

        Err(Error::Config("Manifest file structure changed during operation".into()))
    }

    /// Surgically removes a package from whichever manifest file it is defined in.
    /// Essential for "Teleportation" where a package moves from a system group to a local group.
    pub fn delete_package(&self, package_name: &str) -> Result<()> {
        let location = self.find_package(package_name)?
            .ok_or_else(|| Error::Config(format!("Package '{}' not found in manifests", package_name)))?;

        info!("ManifestEngine: Deleting '{}' from {:?}", package_name, location.file_path);

        let content = fs::read_to_string(&location.file_path)?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

        if lines.len() > location.line_index {
            lines.remove(location.line_index);
            
            // If the removal leaves trailing empty lines, clean them up
            let new_content = lines.join("\n").trim_end().to_string() + "\n";
            atomic_write(&location.file_path, &new_content)?;
            return Ok(());
        }

        Err(Error::Config("Manifest file structure changed during operation".into()))
    }

    /// Adds a package to the designated 'local.txt' file.
    /// If the package already exists elsewhere, it does nothing to prevent duplication.
    pub fn add_to_local(&self, spec_str: &str) -> Result<()> {
        let local_path = self.groups_dir.join("local.txt");
        
        // Extract name for existence check
        let name_part = spec_str.split('@').next().unwrap_or(spec_str);
        let clean_name = if name_part.contains(':') {
            name_part.split_once(':').unwrap().1
        } else {
            name_part
        };

        if self.find_package(clean_name)?.is_some() {
            debug!("ManifestEngine: Package '{}' already exists in manifests. Skipping add.", clean_name);
            return Ok(());
        }

        if !self.groups_dir.exists() {
            fs::create_dir_all(&self.groups_dir)?;
        }

        let mut lines: Vec<String> = if local_path.exists() {
            fs::read_to_string(&local_path)?.lines().map(|s| s.to_string()).collect()
        } else {
            vec![
                "# LiNix Local Manifest".to_string(),
                "# Automatically managed imperative installations".to_string(),
                "".to_string(),
            ]
        };

        lines.push(spec_str.to_string());
        let new_content = lines.join("\n") + "\n";
        atomic_write(&local_path, &new_content)
    }

    /// Returns all package specification strings across all manifests.
    pub fn list_all_specs(&self) -> Result<Vec<String>> {
        let mut specs = Vec::new();

        for entry in WalkDir::new(&self.groups_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file() && e.path().extension().map_or(false, |ext| ext == "txt"))
        {
            let content = fs::read_to_string(entry.path())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_manifest_surgical_update() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("base.txt");
        let content = "# Header\n  apt:curl\n  apt:vim@v1\n# Footer";
        fs::write(&file_path, content).unwrap();

        let engine = ManifestEngine::new(dir.path());
        engine.update_package("vim", "apt:vim@v2").unwrap();

        let new_content = fs::read_to_string(&file_path).unwrap();
        assert!(new_content.contains("  apt:vim@v2"));
        assert!(new_content.contains("# Header"));
        assert!(new_content.contains("apt:curl"));
    }

    #[test]
    fn test_manifest_surgical_delete() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("base.txt");
        let content = "apt:curl\napt:vim\napt:git";
        fs::write(&file_path, content).unwrap();

        let engine = ManifestEngine::new(dir.path());
        engine.delete_package("vim").unwrap();

        let new_content = fs::read_to_string(&file_path).unwrap();
        assert!(!new_content.contains("apt:vim"));
        assert!(new_content.contains("apt:curl"));
        assert!(new_content.contains("apt:git"));
    }
}