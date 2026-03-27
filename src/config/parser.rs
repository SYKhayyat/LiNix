use crate::core::Result;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

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

/// Load all packages from a groups directory
pub fn load_all_packages(groups_dir: &Path) -> Result<HashSet<String>> {
    let mut all_packages = HashSet::new();

    if !groups_dir.exists() {
        return Ok(all_packages);
    }

    for entry in walkdir::WalkDir::new(groups_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("txt") {
            let packages = parse_group_file(path)?;
            all_packages.extend(packages);
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

    #[test]
    fn test_load_all_packages() {
        let temp_dir = TempDir::new().unwrap();

        fs::write(temp_dir.path().join("group1.txt"), "pkg1\npkg2").unwrap();
        fs::write(temp_dir.path().join("group2.txt"), "pkg3\npkg1").unwrap();

        let packages = load_all_packages(temp_dir.path()).unwrap();
        assert_eq!(packages.len(), 3);
        assert!(packages.contains("pkg1"));
        assert!(packages.contains("pkg2"));
        assert!(packages.contains("pkg3"));
    }
}
