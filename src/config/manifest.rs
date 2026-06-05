use crate::core::{Error, Result};
use crate::utils::file::atomic_write;
use std::fs;
use std::path::{PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct PackageLocation {
    pub file_path: PathBuf,
    pub line_index: usize,
    pub raw_line: String,
}

pub struct ManifestEngine {
    groups_dir: PathBuf,
}

impl ManifestEngine {
    pub fn new(groups_dir: impl Into<PathBuf>) -> Self {
        Self {
            groups_dir: groups_dir.into(),
        }
    }

    pub fn find_all_packages(&self, package_name: &str) -> Result<Vec<PackageLocation>> {
        let mut locations = Vec::new();
        if !self.groups_dir.exists() {
            return Ok(locations);
        }

        for entry in WalkDir::new(&self.groups_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file() && e.path().extension().map_or(false, |ext| ext == "txt"))
        {
            let content = fs::read_to_string(entry.path())?;
            for (idx, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                let spec_part = trimmed.split('@').next().unwrap_or(trimmed);
                let is_match = if spec_part.contains(':') {
                    let (_backend, name) = spec_part.split_once(':').unwrap();
                    name == package_name || spec_part == package_name
                } else {
                    spec_part == package_name
                };
                if is_match {
                    locations.push(PackageLocation {
                        file_path: entry.path().to_path_buf(),
                        line_index: idx,
                        raw_line: line.to_string(),
                    });
                }
            }
        }
        Ok(locations)
    }

    pub fn find_package(&self, package_name: &str) -> Result<Option<PackageLocation>> {
        Ok(self.find_all_packages(package_name)?.into_iter().next())
    }

    pub fn update_package(&self, package_name: &str, new_spec: &str) -> Result<()> {
        let locations = self.find_all_packages(package_name)?;
        let loc = locations.first().ok_or_else(|| Error::Config(format!("Package '{}' not found", package_name)))?;
        self.update_package_at_location(loc, new_spec)
    }

    fn update_package_at_location(&self, location: &PackageLocation, new_spec: &str) -> Result<()> {
        let content = fs::read_to_string(&location.file_path)?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        if lines.len() > location.line_index {
            let leading_ws: String = location.raw_line.chars().take_while(|c| c.is_whitespace()).collect();
            lines[location.line_index] = format!("{}{}", leading_ws, new_spec);
            // Fix E0369: Removed '&' to allow concatenation on owned String
            let output = lines.join("\n") + "\n";
            atomic_write(&location.file_path, &output)?;
        }
        Ok(())
    }

    pub fn delete_package(&self, package_name: &str) -> Result<usize> {
        let locations = self.find_all_packages(package_name)?;
        let count = locations.len();
        for loc in &locations {
            self.delete_package_at_location(loc)?;
        }
        Ok(count)
    }

    fn delete_package_at_location(&self, location: &PackageLocation) -> Result<()> {
        let content = fs::read_to_string(&location.file_path)?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        if lines.len() > location.line_index {
            lines.remove(location.line_index);
            // Fix E0369: Removed '&' to allow concatenation on owned String
            let output = lines.join("\n").trim_end().to_string() + "\n";
            atomic_write(&location.file_path, &output)?;
        }
        Ok(())
    }

    pub fn add_to_local(&self, spec_str: &str) -> Result<()> {
        let local_path = self.groups_dir.join("local.txt");
        let name_part = spec_str.split('@').next().unwrap_or(spec_str);
        let clean_name = name_part.split_once(':').map(|(_, n)| n).unwrap_or(name_part);
        
        if !self.find_all_packages(clean_name)?.is_empty() {
            return Ok(());
        }
        if !self.groups_dir.exists() {
            fs::create_dir_all(&self.groups_dir)?;
        }
        let mut lines = if local_path.exists() {
            fs::read_to_string(&local_path)?.lines().map(|s| s.to_string()).collect()
        } else {
            vec![
                "# LiNix Local Manifest".to_string(),
                "# Automatically managed imperative installations".to_string(),
                "".to_string(),
            ]
        };
        lines.push(spec_str.to_string());
        // Fix E0369: Removed '&' to allow concatenation on owned String
        let output = lines.join("\n") + "\n";
        atomic_write(&local_path, &output)?;
        Ok(())
    }

    pub fn list_all_specs(&self) -> Result<Vec<String>> {
        let mut specs = Vec::new();
        if !self.groups_dir.exists() {
            return Ok(specs);
        }
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