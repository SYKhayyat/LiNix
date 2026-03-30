use std::fs;
use std::path::Path;
use crate::core::{Result, Error};

pub fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    if !dest_dir.exists() { fs::create_dir_all(dest_dir)?; }
    let file = fs::File::open(archive_path)?;
    let name = archive_path.to_string_lossy().to_lowercase();

    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let tar = flate2::read::GzDecoder::new(file);
        tar::Archive::new(tar).unpack(dest_dir).map_err(Error::Io)?;
    } else if name.ends_with(".tar.xz") {
        let tar = xz2::read::XzDecoder::new(file);
        tar::Archive::new(tar).unpack(dest_dir).map_err(Error::Io)?;
    } else if name.ends_with(".tar.bz2") {
        let tar = bzip2::read::BzDecoder::new(file);
        tar::Archive::new(tar).unpack(dest_dir).map_err(Error::Io)?;
    } else if name.ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(file).map_err(|e| Error::Other(e.to_string()))?;
        archive.extract(dest_dir).map_err(|e| Error::Other(e.to_string()))?;
    } else {
        // Assume direct binary
        let target = dest_dir.join(archive_path.file_name().ok_or(Error::Parse("No filename".into()))?);
        fs::copy(archive_path, target)?;
    }
    Ok(())
}