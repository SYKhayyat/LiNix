// C:\Users\Administrator\Videos\Nexus\linix\src\config\parser.rs
use crate::core::Result;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Parse a package group file
pub fn parse_group_file(path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(path)?;

    let packages: Vec<String> = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect();

    Ok(packages)
}

/// Parse a bloatware file
pub fn parse_bloatware_file(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    parse_group_file(path)
}

/// Gets the primary file path for user imperative installations (local.txt)
pub fn get_user_group_file(groups_dir: &Path) -> PathBuf {
    if !groups_dir.exists() {
        let _ = fs::create_dir_all(groups_dir);
    }
    groups_dir.join("local.txt")
}

/// Writes package declarations back to the file
pub fn write_group_file(path: &Path, packages: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    let content = packages.join("\n");
    fs::write(path, content)?;
    Ok(())
}

/// Load all packages from a groups directory, applying hostname matching
pub fn load_all_packages(groups_dir: &Path) -> Result<HashSet<String>> {
    let mut all_packages = HashSet::new();

    if !groups_dir.exists() {
        return Ok(all_packages);
    }
    
    let current_hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let host_file_name = format!("host-{}.txt", current_hostname);

    for entry in walkdir::WalkDir::new(groups_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        if path.is_file() {
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if file_name.ends_with(".txt") {
                // If it starts with "host-", only load if it matches our hostname
                if file_name.starts_with("host-") {
                    if file_name == host_file_name {
                        let packages = parse_group_file(path)?;
                        all_packages.extend(packages);
                    }
                } else {
                    let packages = parse_group_file(path)?;
                    all_packages.extend(packages);
                }
            }
        }
    }

    Ok(all_packages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_group_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("packages.txt");

        let content = "package1\n# comment\npackage2\n\npackage3";
        fs::write(&file_path, content).unwrap();

        let packages = parse_group_file(&file_path).unwrap();
        assert_eq!(packages, vec!["package1", "package2", "package3"]);
    }
}