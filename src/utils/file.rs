use crate::core::{Error, Result};
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;

pub fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let dir = path.parent().ok_or_else(|| {
        let err = std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Target path has no parent directory",
        );
        Error::Io(err.to_string())
    })?;

    if !dir.exists() {
        fs::create_dir_all(dir).map_err(Error::from)?;
    }

    let mut temp_file = NamedTempFile::new_in(dir).map_err(Error::from)?;
    temp_file
        .write_all(content.as_bytes())
        .map_err(Error::from)?;
    temp_file.flush().map_err(Error::from)?;
    temp_file.as_file().sync_all().map_err(Error::from)?;
    temp_file.persist(path).map_err(Error::from)?;

    Ok(())
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path).map_err(Error::from)?;
    }
    Ok(())
}

pub fn read_lines_filtered(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(path).map_err(Error::from)?;
    Ok(content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect())
}

pub fn force_remove(path: &Path) -> Result<()> {
    if path.exists() {
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(Error::from)?;
        } else {
            fs::remove_file(path).map_err(Error::from)?;
        }
    }
    Ok(())
}
