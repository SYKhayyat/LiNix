use crate::core::{Error, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use tracing::debug;

/// The only thing standing between a backend that downloads unauthenticated binaries
/// (Web, GitHub) and executing whatever the network handed it.
///
/// `async` because hashing a 150 MB release tarball is seconds of CPU-bound work reading a file
/// off disk, and every caller is an async download path. Run inline it holds a runtime worker
/// for the length of the file; the work itself goes to the blocking pool.
pub async fn verify_checksum(path: &Path, expected_hex: &str) -> Result<()> {
    let (path, expected) = (path.to_path_buf(), expected_hex.to_string());
    hashing(move || verify_checksum_blocking(&path, &expected)).await
}

pub async fn generate_checksum(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    hashing(move || generate_checksum_blocking(&path)).await
}

/// Hash two files at once, for the callers that compare a source against a target.
///
/// The planner asks this per template spec from inside its fan-out; done one after the other it
/// is two full file reads and two SHA-256 passes taken in series for no reason — they have
/// nothing to say to each other.
pub async fn checksum_pair(a: &Path, b: &Path) -> (Result<String>, Result<String>) {
    tokio::join!(generate_checksum(a), generate_checksum(b))
}

async fn hashing<T, F>(work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| Error::Io(format!("hashing did not finish: {}", e)))?
}

fn verify_checksum_blocking(path: &PathBuf, expected_hex: &str) -> Result<()> {
    if !path.exists() {
        let err = io::Error::new(
            io::ErrorKind::NotFound,
            format!("File not found for checksum verification: {:?}", path),
        );
        return Err(Error::from(err));
    }

    let actual_hex = generate_checksum_blocking(path)?;

    if actual_hex.eq_ignore_ascii_case(expected_hex) {
        debug!("Security: Checksum verified for {:?}", path);
        Ok(())
    } else {
        Err(Error::Validation(format!(
            "SECURITY ALERT: Checksum mismatch detected!\nPath: {:?}\nExpected: {}\nActual:   {}",
            path, expected_hex, actual_hex
        )))
    }
}

fn generate_checksum_blocking(path: &PathBuf) -> Result<String> {
    let mut file = File::open(path).map_err(Error::from)?;
    let mut hasher = Sha256::new();
    // Streamed, not read to a Vec: these files are arbitrarily large binaries.
    //
    // Fed by hand rather than by `io::copy`, which needs the hasher to be an `io::Write` — an
    // impl that comes from a crate feature `sha2` has since removed, so the copy is the line
    // that stops compiling on the next major. `update` is the interface the hash actually has.
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = io::Read::read(&mut file, &mut buf).map_err(Error::from)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}
