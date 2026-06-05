use sha2::{Sha256, Digest};
use std::io;
use std::fs::File;
use std::path::Path;
use crate::core::{Result, Error};
use tracing::debug;

/// Verifies that the file at 'path' matches the provided SHA256 hex string.
/// This is a critical security component for backends that download 
/// unauthenticated binaries from the web (Web, GitHub, etc.).
pub fn verify_checksum(path: &Path, expected_hex: &str) -> Result<()> {
    // 1. Validate that the file exists before attempting to open
    if !path.exists() {
        let err = io::Error::new(
            io::ErrorKind::NotFound, 
            format!("File not found for checksum verification: {:?}", path)
        );
        return Err(Error::from(err));
    }
    
    // 2. Open file and initialize the SHA256 hasher
    let mut file = File::open(path).map_err(Error::from)?;
    let mut hasher = Sha256::new();
    
    // 3. Stream the file into the hasher to avoid loading large binaries into memory
    io::copy(&mut file, &mut hasher).map_err(Error::from)?;
    
    // 4. Finalize the hash and encode as hex
    let hash = hasher.finalize();
    let actual_hex = hex::encode(hash);
    
    // 5. Compare (case-insensitive)
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

/// Generates a SHA256 hash for a given file.
pub fn generate_checksum(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(Error::from)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(Error::from)?;
    Ok(hex::encode(hasher.finalize()))
}