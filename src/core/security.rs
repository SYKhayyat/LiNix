use sha2::{Sha256, Digest};
use std::io;
use std::fs::File;
use std::path::Path;
use crate::core::{Result, Error};

pub fn verify_checksum(path: &Path, expected_hex: &str) -> Result<()> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    let hash = hasher.finalize();
    let actual_hex = hex::encode(hash);
    
    if actual_hex.to_lowercase() == expected_hex.to_lowercase() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "SECURITY ALERT: Checksum mismatch for {:?}! Expected {}, got {}", 
            path, expected_hex, actual_hex
        )))
    }
}