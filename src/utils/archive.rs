use crate::core::{Error, Result};
use std::fs;
use std::path::Path;
use tracing::debug;

/// A robust, cross-platform archive extraction utility.
/// Supports .zip, .tar.gz, .tar.xz, and .tar.bz2 formats.
pub fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    // Ensure destination exists
    if !dest_dir.exists() {
        fs::create_dir_all(dest_dir).map_err(Error::from)?;
    }

    let file = fs::File::open(archive_path).map_err(Error::from)?;
    let name_lower = archive_path.to_string_lossy().to_lowercase();

    debug!("Extracting archive: {:?} into {:?}", archive_path, dest_dir);

    if name_lower.ends_with(".tar.gz") || name_lower.ends_with(".tgz") {
        let tar = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(tar);
        archive.unpack(dest_dir).map_err(Error::from)?;
    } else if name_lower.ends_with(".tar.xz") {
        let tar = xz2::read::XzDecoder::new(file);
        let mut archive = tar::Archive::new(tar);
        archive.unpack(dest_dir).map_err(Error::from)?;
    } else if name_lower.ends_with(".tar.bz2") {
        let tar = bzip2::read::BzDecoder::new(file);
        let mut archive = tar::Archive::new(tar);
        archive.unpack(dest_dir).map_err(Error::from)?;
    } else if name_lower.ends_with(".zip") {
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| Error::Other(format!("Zip error: {}", e)))?;
        archive
            .extract(dest_dir)
            .map_err(|e| Error::Other(format!("Zip extraction failed: {}", e)))?;
    } else {
        // Fallback for direct binary downloads that aren't actually archives
        let filename = archive_path
            .file_name()
            .ok_or_else(|| Error::Other("Invalid archive filename".into()))?;
        let target = dest_dir.join(filename);
        fs::copy(archive_path, target).map_err(Error::from)?;
    }

    Ok(())
}

/// Helper to check if a file is a known archive format.
pub fn is_archive(path: &Path) -> bool {
    let name = path.to_string_lossy().to_lowercase();
    [".zip", ".tar.gz", ".tgz", ".tar.xz", ".tar.bz2"]
        .iter()
        .any(|ext| name.ends_with(ext))
}
