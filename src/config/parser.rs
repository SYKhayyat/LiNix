use crate::core::{Result, Error};
use crate::utils::file::atomic_write;
use std::path::{Path, PathBuf};
use std::collections::HashSet;
use tokio::fs;

/// Parses a package group file (.txt) asynchronously.
/// Hardened for Phase 3.2: Full non-blocking I/O support.
pub async fn parse_group_file(path: &Path) -> Result<Vec<String>> {
    if !tokio::fs::try_exists(path).await.unwrap_or(false) {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path).await.map_err(Error::from)?;

    let packages: Vec<String> = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect();

    Ok(packages)
}

/// Adds a package string to the local declarative configuration.
pub async fn add_package_to_local(groups_dir: &Path, package_str: &str) -> Result<()> {
    let local_file = groups_dir.join("local.txt");
    
    if !tokio::fs::try_exists(groups_dir).await.unwrap_or(false) {
        fs::create_dir_all(groups_dir).await.map_err(Error::from)?;
    }

    let mut lines: Vec<String> = if tokio::fs::try_exists(&local_file).await.unwrap_or(false) {
        fs::read_to_string(&local_file).await.map_err(Error::from)?
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
        
        let path_owned = local_file.clone();
        tokio::task::spawn_blocking(move || {
            atomic_write(&path_owned, &new_content)
        }).await.map_err(|e| Error::Other(e.to_string()))??;
    }

    Ok(())
}

/// Removes a package string from the local declarative configuration.
pub async fn remove_package_from_local(groups_dir: &Path, package_name: &str) -> Result<()> {
    let local_file = groups_dir.join("local.txt");
    if !tokio::fs::try_exists(&local_file).await.unwrap_or(false) {
        return Ok(());
    }

    let content = fs::read_to_string(&local_file).await.map_err(Error::from)?;
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            lines.push(line.to_string());
            continue;
        }
        
        let is_match = if let Some((b, n)) = trimmed.split_once(':') {
            let name_only = n.split('@').next().unwrap_or(n).trim();
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
        let path_owned = local_file.clone();
        tokio::task::spawn_blocking(move || {
            atomic_write(&path_owned, &new_content)
        }).await.map_err(|e| Error::Other(e.to_string()))??;
    }

    Ok(())
}

pub async fn get_user_group_file(groups_dir: &Path) -> PathBuf {
    if !tokio::fs::try_exists(groups_dir).await.unwrap_or(false) {
        let _ = fs::create_dir_all(groups_dir).await;
    }
    groups_dir.join("local.txt")
}

pub async fn write_group_file(path: &Path, packages: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !tokio::fs::try_exists(parent).await.unwrap_or(false) {
            fs::create_dir_all(parent).await.map_err(Error::from)?;
        }
    }
    let content = packages.join("\n");
    let path_owned = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        atomic_write(&path_owned, &content)
    }).await.map_err(|e| Error::Other(e.to_string()))??;
    Ok(())
}

pub async fn load_all_packages(groups_dir: &Path) -> Result<HashSet<String>> {
    let mut all_packages = HashSet::new();
    if !tokio::fs::try_exists(groups_dir).await.unwrap_or(false) {
        return Ok(all_packages);
    }
    
    let current_hostname = crate::config::Config::get_hostname();
    let host_file_name = format!("host-{}.txt", current_hostname);

    let groups_dir_owned = groups_dir.to_path_buf();
    let entries: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
        walkdir::WalkDir::new(groups_dir_owned)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .map(|e| e.path().to_path_buf())
            .collect()
    }).await.map_err(|e| Error::Other(e.to_string()))?;

    for path in entries {
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if file_name.ends_with(".txt") {
            if file_name.starts_with("host-") {
                if file_name == host_file_name {
                    let packages = parse_group_file(&path).await?;
                    all_packages.extend(packages);
                }
            } else {
                let packages = parse_group_file(&path).await?;
                all_packages.extend(packages);
            }
        }
    }
    Ok(all_packages)
}