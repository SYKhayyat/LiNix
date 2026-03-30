use std::fs;
use std::path::Path;
use crate::core::{Result, Error};

pub fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    if !dest_dir.exists() {
        fs::create_dir_all(dest_dir)?;
    }
    
    let file = fs::File::open(archive_path)?;
    let file_name = archive_path.to_string_lossy().to_lowercase();

    if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") {
        let tar = flate2::read::GzDecoder::new(file);
        let mut arch = tar::Archive::new(tar);
        arch.unpack(dest_dir).map_err(|e| Error::Io(e))?;
    } else if file_name.ends_with(".zip") {
        let mut arch = zip::ZipArchive::new(file)
            .map_err(|e| Error::Other(format!("Zip error: {}", e)))?;
        arch.extract(dest_dir)
            .map_err(|e| Error::Other(format!("Extraction error: {}", e)))?;
    } else {
        // If not an archive, treat as a direct binary
        let target = dest_dir.join(archive_path.file_name().unwrap());
        fs::copy(archive_path, target)?;
    }
    Ok(())
}