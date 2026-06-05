use crate::core::Result;
use crate::utils::file::atomic_write;
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashSet;

/// Parses a package group file (.txt).
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

/// Adds a package string to the local declarative configuration.
pub fn add_package_to_local(groups_dir: &Path, package_str: &str) -> Result<()> {
    let local_file = groups_dir.join("local.txt");
    
    if !groups_dir.exists() {
        fs::create_dir_all(groups_dir)?;
    }

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

    let is_duplicate = lines.iter().any(|l| {
        let clean = l.trim();
        clean == package_str
    });

    if !is_duplicate {
        lines.push(package_str.to_string());
        let new_content = lines.join("\n") + "\n";
        atomic_write(&local_file, &new_content)?;
    }

    Ok(())
}

/// Removes a package string from the local declarative configuration.
/// Hardened: Now correctly utilizes the backend identifier for precision.
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
        if trimmed.is_empty() || trimmed.starts_with('#') {
            lines.push(line.to_string());
            continue;
        }
        
        // WIRING: Use the backend to ensure we don't remove the wrong package
        let is_match = if let Some((b, n)) = trimmed.split_once(':') {
            let name_only = n.split('@').next().unwrap_or(n).trim();
            // Match if:
            // 1. Full string match (e.g. "apt:vim" == "apt:vim")
            // 2. Name match (e.g. "vim" == "vim") while ignoring the backend
            // 3. Full match with options (e.g. "apt:vim" == "apt:vim@version=1")
            trimmed == package_name || name_only == package_name || b == package_name
        } else {
            trimmed == package_name
        };

        if is_match {
            found = true;
            continue; 
        }
        lines.push(line.to_string());
    }

    if found {
        let new_content = lines.join("\n") + "\n";
        atomic_write(&local_file, &new_content)?;
    }

    Ok(())
}

pub fn get_user_group_file(groups_dir: &Path) -> PathBuf {
    if !groups_dir.exists() {
        let _ = fs::create_dir_all(groups_dir);
    }
    groups_dir.join("local.txt")
}

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