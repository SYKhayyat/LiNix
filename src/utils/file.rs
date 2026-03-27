use crate::core::Result;
use std::fs;
use std::path::Path;
use tempfile::NamedTempFile;
use std::io::Write;

/// Atomically write to a file
pub fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let dir = path.parent().ok_or_else(|| {
        crate::core::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Path has no parent directory",
        ))
    })?;

    // Ensure directory exists
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }

    let mut temp_file = NamedTempFile::new_in(dir)?;
    temp_file.write_all(content.as_bytes())?;
    temp_file.persist(path)?;

    Ok(())
}

/// Ensure a directory exists
pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Read lines from a file, filtering empty and comment lines
pub fn read_lines_filtered(path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(path)?;

    Ok(content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        atomic_write(&file_path, "test content").unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "test content");
    }

    #[test]
    fn test_ensure_dir() {
        let temp_dir = TempDir::new().unwrap();
        let nested_dir = temp_dir.path().join("a").join("b").join("c");

        ensure_dir(&nested_dir).unwrap();
        assert!(nested_dir.exists());
    }
}
