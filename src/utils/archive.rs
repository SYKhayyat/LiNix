use crate::core::{Error, Result};
use std::fs;
use std::path::Path;
use tracing::debug;

pub fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<()> {
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

/// Compress a directory tree into a single `.tar.gz` file. The archive stores paths relative
/// to `src_dir` under `root_name/…`, so unpacking recreates one self-contained top folder
/// (the mirror of [`extract_archive`]). Returns the number of bytes written.
pub fn create_tar_gz(src_dir: &Path, dest_file: &Path, root_name: &str) -> Result<u64> {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    if let Some(parent) = dest_file.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(Error::from)?;
        }
    }
    let out = fs::File::create(dest_file).map_err(Error::from)?;
    let enc = GzEncoder::new(out, Compression::default());
    let mut builder = tar::Builder::new(enc);
    builder
        .append_dir_all(root_name, src_dir)
        .map_err(Error::from)?;
    // Finish the tar, then finish the gzip stream, so all bytes are flushed to disk.
    let enc = builder.into_inner().map_err(Error::from)?;
    enc.finish().map_err(Error::from)?;
    let size = fs::metadata(dest_file).map(|m| m.len()).unwrap_or(0);
    debug!("Wrote {} bytes to {:?}", size, dest_file);
    Ok(size)
}

pub fn is_archive(path: &Path) -> bool {
    let name = path.to_string_lossy().to_lowercase();
    [".zip", ".tar.gz", ".tgz", ".tar.xz", ".tar.bz2"]
        .iter()
        .any(|ext| name.ends_with(ext))
}
