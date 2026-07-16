use crate::core::{Error, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io;
use std::path::Path;
use tracing::debug;

/// The only thing standing between a backend that downloads unauthenticated binaries
/// (Web, GitHub) and executing whatever the network handed it.
pub fn verify_checksum(path: &Path, expected_hex: &str) -> Result<()> {
    if !path.exists() {
        let err = io::Error::new(
            io::ErrorKind::NotFound,
            format!("File not found for checksum verification: {:?}", path),
        );
        return Err(Error::from(err));
    }

    let mut file = File::open(path).map_err(Error::from)?;
    let mut hasher = Sha256::new();

    // Streamed, not read to a Vec: these files are arbitrarily large binaries.
    io::copy(&mut file, &mut hasher).map_err(Error::from)?;

    let hash = hasher.finalize();
    let actual_hex = hex::encode(hash);

    if actual_hex.to_lowercase() == expected_hex.to_lowercase() {
        debug!("Security: Checksum verified for {:?}", path);
        Ok(())
    } else {
        Err(Error::Validation(format!(
            "SECURITY ALERT: Checksum mismatch detected!\nPath: {:?}\nExpected: {}\nActual:   {}",
            path, expected_hex, actual_hex
        )))
    }
}

pub fn generate_checksum(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(Error::from)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(Error::from)?;
    Ok(hex::encode(hasher.finalize()))
}
