use crate::core::Result;
use crate::utils::file::atomic_write;
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashSet;

/// Parses a package group file (.txt).
/// Returns a list of package strings, stripping comments and empty lines.
pub fn parse_group_file(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path)?;

    let packages: Vec<String> = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect();

    Ok(packages)
}

/// Point 2: Adds a package string to the local declarative configuration.
/// This ensures that 'linix install pkg' becomes a permanent part of the system state.
pub fn add_package_to_local(groups_dir: &Path, package_str: &str) -> Result<()> {
    let local_file = groups_dir.join("local.txt");
    
    // 1. Ensure directory exists
    if !groups_dir.exists() {
        fs::create_dir_all(groups_dir)?;
    }

    // 2. Read existing content
    let mut lines: Vec<String> = if local_file.exists() {
        fs::read_to_string(&local_file)?
            .lines()
            .map(|s| s.to_string())
            .collect()
    } else {
        vec![
            "# LiNix Local Manifest".to_string(),
            "# This file is automatically updated by imperative commands.".to_string(),
            "".to_string(),
        ]
    };

    // 3. Check for duplicates to maintain idempotency
    let is_duplicate = lines.iter().any(|l| {
        let clean = l.trim();
        clean == package_str || 
        (clean.contains(':') && clean == package_str) // Exact match check
    });

    if !is_duplicate {
        lines.push(package_str.to_string());
        let new_content = lines.join("\n") + "\n";
        
        // 4. Perform atomic write to prevent manifest corruption
        atomic_write(&local_file, &new_content)?;
    }

    Ok(())
}

/// Point 2: Removes a package string from the local declarative configuration.
/// Used when 'linix remove pkg' is called to update the source-of-truth.
pub fn remove_package_from_local(groups_dir: &Path, package_name: &str) -> Result<()> {
    let local_file = groups_dir.join("local.txt");
    if !local_file.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&local_file)?;
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;

    for line in content.lines() {
        let trimmed = line.trim();
        
        // Match logic:
        // If line is "apt:neovim" and package_name is "neovim" or "apt:neovim"
        let is_match = if trimmed.contains(':') {
            let (backend, name) = trimmed.split_once(':').unwrap();
            name == package_name || trimmed == package_name
        } else {
            trimmed == package_name
        };

        if is_match {
            found = true;
            continue; // Skip this line (remove it)
        }
        lines.push(line.to_string());
    }

    if found {
        let new_content = lines.join("\n") + "\n";
        atomic_write(&local_file, &new_content)?;
    }

    Ok(())
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
    atomic_write(path, &content)?;
    Ok(())
}

/// Load all packages from a groups directory, applying hostname matching
pub fn load_all_packages(groups_dir: &Path) -> Result<HashSet<String>> {
    let mut all_packages = HashSet::new();

    if !groups_dir.exists() {
        return Ok(all_packages);
    }
    
    let current_hostname = crate::config::Config::get_hostname();
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