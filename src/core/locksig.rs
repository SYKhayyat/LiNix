// src/core/locksig.rs
//
// Tamper-evident lockfiles. `linix lock` writes a keyed digest ("sig") alongside the locks; a
// `sync --locked` verifies it, so an edited locks.json is caught rather than silently trusted.
//
// The key is a machine-local secret stored next to the lockfile (0600 on Unix). This is not a
// public-key signature — it detects tampering by anyone who can't read the key file, which is
// the realistic threat for a reproducibility lockfile. Attesting across machines (a shared or
// asymmetric key) is a future extension; the on-disk format ("sig" field) already allows it.

use sha2::{Digest, Sha256};
use std::path::Path;

/// Deterministic text form of the locks object for signing. `serde_json::Map` is a sorted
/// `BTreeMap` (no `preserve_order` feature), so this is canonical regardless of insert order.
pub fn canonical(locks: &serde_json::Map<String, serde_json::Value>) -> String {
    serde_json::Value::Object(locks.clone()).to_string()
}

/// Keyed digest over the canonical locks text: `hex(sha256(key || "|" || text))`.
pub fn sign(key: &[u8], canonical_text: &str) -> String {
    let mut h = Sha256::new();
    h.update(key);
    h.update(b"|");
    h.update(canonical_text.as_bytes());
    hex::encode(h.finalize())
}

/// True if `sig` matches the expected digest for `locks` under `key`.
pub fn verify(key: &[u8], locks: &serde_json::Map<String, serde_json::Value>, sig: &str) -> bool {
    sign(key, &canonical(locks)) == sig
}

/// Read the machine-local lock key WITHOUT creating one. `None` means no key exists here —
/// e.g. a fresh machine restoring a bundle — in which case a signed lockfile simply can't be
/// verified locally (the caller allows it rather than refusing, to keep reproducibility).
pub fn read_key(dir: &Path) -> Option<Vec<u8>> {
    match std::fs::read(dir.join(".linix-lock.key")) {
        Ok(k) if !k.is_empty() => Some(k),
        _ => None,
    }
}

/// Read (creating if absent) the machine-local lock key stored beside the lockfile. A 256-bit
/// key derived from two v4 UUIDs; written 0600 on Unix.
pub fn machine_key(dir: &Path) -> std::io::Result<Vec<u8>> {
    let path = dir.join(".linix-lock.key");
    if let Ok(existing) = std::fs::read(&path) {
        if !existing.is_empty() {
            return Ok(existing);
        }
    }
    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    key.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    std::fs::create_dir_all(dir).ok();
    std::fs::write(&path, &key)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn locks() -> serde_json::Map<String, serde_json::Value> {
        json!({ "apt:curl": "8.4.0", "cargo:ripgrep": "14.1" })
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn sign_is_stable_and_key_sensitive() {
        let m = locks();
        let a = sign(b"key-one", &canonical(&m));
        let b = sign(b"key-one", &canonical(&m));
        let c = sign(b"key-two", &canonical(&m));
        assert_eq!(a, b, "same key + content → same sig");
        assert_ne!(a, c, "different key → different sig");
    }

    #[test]
    fn verify_detects_tampering() {
        let key = b"secret";
        let m = locks();
        let sig = sign(key, &canonical(&m));
        assert!(verify(key, &m, &sig));

        // Flip a version → verification fails.
        let mut tampered = m.clone();
        tampered.insert("apt:curl".into(), json!("9.9.9"));
        assert!(!verify(key, &tampered, &sig));
    }

    #[test]
    fn machine_key_is_persistent() {
        let tmp = tempfile::tempdir().unwrap();
        let k1 = machine_key(tmp.path()).unwrap();
        let k2 = machine_key(tmp.path()).unwrap();
        assert_eq!(k1, k2, "key persists across reads");
        assert_eq!(k1.len(), 32);
    }
}
