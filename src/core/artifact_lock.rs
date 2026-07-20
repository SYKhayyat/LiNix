//! What a download-shaped backend actually resolved to: `locks/<backend>.toml` (VIII.2, D6).
//!
//! A version is not the identity of a downloaded artifact. `github:sharkdp/fd@version=10.2.0`
//! names one release and that release ships a `.deb`, an `.rpm`, a `.tar.gz` and a bare
//! binary — so a lock recording only the version leaves the artifact free to change under a
//! pinned declaration, which is the bug Part VIII exists to close.
//!
//! The hash is here rather than in the declaration because one hash cannot cover an asset that
//! varies by machine (D6): a shared module says `github:x/y` and the Ubuntu box downloads the
//! `.deb` while the Fedora box downloads the `.rpm`. A per-machine record can describe both; a
//! hand-written `@sha256=` cannot describe either without pinning the format first.
//!
//! **A recorded hash is a record, not a policy.** It says what was downloaded, so a change is
//! visible in `linix diff` and a re-download that differs is an error. It does not demand that
//! the user pre-declare anything.

use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// One resolved artifact. Every field is generated — nothing here is typed by a user.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ArtifactLock {
    /// The release this came from, as the backend names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The asset filename that was chosen, so a re-resolve that picks differently is visible.
    pub asset: String,
    /// Where it came from. Recorded because the asset name alone does not identify a file.
    pub url: String,
    /// The `formats` entry that matched, as VIII.2 spells it.
    pub format: String,
    /// The hash of the bytes that were installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// `locks/<backend>.toml`, keyed by the package name the declaration used. A `BTreeMap` so the
/// file is ordered and diffs cleanly in git.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ArtifactLedger {
    #[serde(default, flatten)]
    entries: BTreeMap<String, ArtifactLock>,
}

impl ArtifactLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// A missing file means nothing has been locked yet — the correct starting state, and
    /// never an error.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s)
                .map_err(|e| Error::Toml(format!("reading {}: {}", path.display(), e))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(Error::Io(format!("reading {}: {}", path.display(), e))),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| Error::Io(format!("creating {}: {}", dir.display(), e)))?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| Error::Toml(format!("serializing artifact ledger: {}", e)))?;
        std::fs::write(path, body)
            .map_err(|e| Error::Io(format!("writing {}: {}", path.display(), e)))
    }

    pub fn get(&self, name: &str) -> Option<&ArtifactLock> {
        self.entries.get(name)
    }

    pub fn record(&mut self, name: impl Into<String>, lock: ArtifactLock) {
        self.entries.insert(name.into(), lock);
    }

    /// Drop a package's entry. The lock describes what is installed, so a removal that left
    /// the entry behind would pin a future install to an artifact chosen for a different
    /// declaration.
    pub fn forget(&mut self, name: &str) -> bool {
        self.entries.remove(name).is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ArtifactLock)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }
}

/// What a re-download must satisfy to be the artifact the lock describes.
///
/// Returns the objection, or `None` when the download is what was locked. A mismatch is an
/// error rather than a re-selection: selecting a different asset because the pinned one failed
/// its hash would turn a supply-chain alarm into a silent substitution (VIII.2).
pub fn verify_against(lock: &ArtifactLock, asset: &str, sha256: Option<&str>) -> Option<String> {
    if lock.asset != asset {
        return Some(format!(
            "the lock records `{}` and this resolved to `{}`. Run `linix lock` if the change \
             is intended.",
            lock.asset, asset
        ));
    }
    match (&lock.sha256, sha256) {
        (Some(locked), Some(got)) if !locked.eq_ignore_ascii_case(got) => Some(format!(
            "`{}` does not match the hash in the lock.\n  locked: {}\n  got:    {}",
            asset, locked, got
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn lock(asset: &str, sha: Option<&str>) -> ArtifactLock {
        ArtifactLock {
            version: Some("10.2.0".into()),
            asset: asset.into(),
            url: format!("https://example.invalid/{}", asset),
            format: "tarball".into(),
            sha256: sha.map(str::to_string),
        }
    }

    #[test]
    fn a_missing_file_is_an_empty_ledger_not_an_error() {
        let dir = TempDir::new().unwrap();
        let led = ArtifactLedger::load(&dir.path().join("github.toml")).unwrap();
        assert!(led.is_empty());
    }

    #[test]
    fn an_entry_survives_a_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("locks").join("github.toml");
        let mut led = ArtifactLedger::new();
        led.record("sharkdp/fd", lock("fd.tar.gz", Some("abc123")));
        led.save(&path).unwrap();

        let back = ArtifactLedger::load(&path).unwrap();
        assert_eq!(back.get("sharkdp/fd").unwrap().asset, "fd.tar.gz");
        assert_eq!(back.get("sharkdp/fd").unwrap().url, "https://example.invalid/fd.tar.gz");
        assert_eq!(back.get("sharkdp/fd").unwrap().format, "tarball");
        assert_eq!(back.get("sharkdp/fd").unwrap().sha256.as_deref(), Some("abc123"));
    }

    #[test]
    fn forgetting_a_package_drops_its_entry() {
        // A lock left behind after a removal would pin the next install to an artifact
        // chosen for a declaration that no longer exists.
        let mut led = ArtifactLedger::new();
        led.record("sharkdp/fd", lock("fd.tar.gz", None));
        assert!(led.forget("sharkdp/fd"));
        assert!(!led.forget("sharkdp/fd"));
        assert!(led.is_empty());
    }

    #[test]
    fn a_different_asset_is_an_objection_that_says_how_to_accept_it() {
        let l = lock("fd-gnu.tar.gz", None);
        let why = verify_against(&l, "fd-musl.tar.gz", None).unwrap();
        assert!(why.contains("fd-gnu.tar.gz"), "{}", why);
        assert!(why.contains("fd-musl.tar.gz"), "{}", why);
        assert!(why.contains("linix lock"), "{}", why);
    }

    #[test]
    fn a_changed_hash_on_the_same_asset_is_an_objection() {
        let l = lock("fd.tar.gz", Some("abc123"));
        let why = verify_against(&l, "fd.tar.gz", Some("def456")).unwrap();
        assert!(why.contains("abc123"), "{}", why);
        assert!(why.contains("def456"), "{}", why);
    }

    #[test]
    fn the_same_asset_and_hash_is_no_objection() {
        let l = lock("fd.tar.gz", Some("abc123"));
        assert!(verify_against(&l, "fd.tar.gz", Some("ABC123")).is_none());
    }

    #[test]
    fn an_unhashed_lock_still_checks_the_asset() {
        // Older entries and download-only artifacts may have no hash; the asset name is
        // still an identity worth holding to.
        let l = lock("fd.tar.gz", None);
        assert!(verify_against(&l, "fd.tar.gz", Some("abc123")).is_none());
        assert!(verify_against(&l, "other.tar.gz", None).is_some());
    }
}
