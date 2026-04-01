// src/utils/file.rs
use crate::core::{Result, Error};
use std::fs;
use std::path::Path;
use tempfile::NamedTempFile;
use std::io::Write;

/// Atomically writes content to a file.
/// It writes to a temp file first, then renames it, ensuring the target 
/// is never left in a partially-written state.
pub fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let dir = path.parent().ok_or_else(|| {
        Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Path has no parent directory"))
    })?;

    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }

    let mut temp_file = NamedTempFile::new_in(dir)?;
    temp_file.write_all(content.as_bytes())?;
    
    // persist() performs the atomic rename operation
    temp_file.persist(path).map_err(|e| Error::Persist(e.to_string()))?;

    Ok(())
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

pub fn read_lines_filtered(path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(path)?;
    Ok(content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect())
}