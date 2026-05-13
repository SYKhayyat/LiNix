use crate::core::{Result, Error};
use std::fs;
use std::path::Path;
use tempfile::NamedTempFile;
use std::io::Write;

/// Atomically writes content to a file.
/// This implementation writes to a temporary file in the target directory and then renames it.
/// On modern filesystems (BTRFS, ZFS, Ext4, NTFS), renaming is an atomic operation.
/// This ensures the target file is never left in a partially-written state if the 
/// process crashes or power is lost.
/// Implementation for Roadmap Phase 3: Mission-Critical Safety.
pub fn atomic_write(path: &Path, content: &str) -> Result<()> {
    // 1. Identify the parent directory to ensure the temp file is on the same mount point
    let dir = path.parent().ok_or_else(|| {
        Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Target path has no parent directory"))
    })?;

    // 2. Ensure the parent directory structure exists
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }

    // 3. Create a temporary file within the same directory
    // This is vital because atomic renames usually do not work across different filesystems/mounts.
    let mut temp_file = NamedTempFile::new_in(dir)?;
    
    // 4. Write and flush the data to the temporary file
    temp_file.write_all(content.as_bytes())?;
    temp_file.flush()?;
    
    // 5. Explicitly sync to ensure data is physically written to the storage medium
    temp_file.as_file().sync_all()?;

    // 6. Persist performs the atomic rename operation to the final path
    temp_file.persist(path).map_err(|e| Error::Persist(e.to_string()))?;

    Ok(())
}

/// Utility to ensure a directory exists, creating all parents if necessary.
pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Reads a file and returns a list of non-comment, non-empty lines.
/// Used for parsing simple package list files (.txt).
pub fn read_lines_filtered(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    
    let content = fs::read_to_string(path)?;
    Ok(content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect())
}

/// Securely removes a file or directory if it exists.
pub fn force_remove(path: &Path) -> Result<()> {
    if path.exists() {
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}