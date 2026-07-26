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
    /// Which rule chose this format — the line's `@asset=`/`@formats=`, or the built-in default
    /// (D14). Kept so `why` can answer "why this file and not the `.deb` I expected" without
    /// re-running a network selection. `serde(default)` so a lock written before D14 still
    /// parses; it simply has no reason to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_by: Option<String>,
    /// The hash of the bytes that were installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The system installer that owns this artifact once it is placed (D5) — `dpkg` for a
    /// `.deb`, `rpm` for an `.rpm`. `None` is the ordinary case: LiNix unpacked it and put it on
    /// PATH itself. When set, removal, upgrade and dedup route through this manager rather than
    /// through a file delete, and `system_package` is the name it was recorded under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_by: Option<String>,
    /// The package name the system manager knows this artifact as (read from the file at install
    /// time). Only meaningful alongside `installed_by`; it is what `dpkg -r`/`rpm -e` and the
    /// unmanaged-dedup key on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_package: Option<String>,
}

/// `locks/<backend>.toml`, keyed by the package name the declaration used. A `BTreeMap` so the
/// file is ordered and diffs cleanly in git.
///
/// A declaration locks a *list* because `@asset=all` installs every match (VIII.2), and one
/// entry per declaration could only describe the first of them.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ArtifactLedger {
    #[serde(default, flatten)]
    entries: BTreeMap<String, Vec<ArtifactLock>>,
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

    /// What a declaration resolved to, in selection order. Empty when nothing is locked.
    pub fn locked(&self, name: &str) -> &[ArtifactLock] {
        self.entries.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Record everything one declaration installed. The whole set is replaced, because a
    /// declaration that now resolves to fewer artifacts must not keep the ones it dropped.
    pub fn record(&mut self, name: impl Into<String>, locks: Vec<ArtifactLock>) {
        self.entries.insert(name.into(), locks);
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

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[ArtifactLock])> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    /// Every system package these declarations installed through a system manager (D5), as
    /// `(installer, package_name)` pairs. The unmanaged crawl and `check` subtract these so a
    /// `.deb` a `github:` line handed to `dpkg` is not also reported as apt-visible drift, and
    /// `purge-unmanaged` defers to the recorded installer rather than deleting it.
    pub fn system_packages(&self) -> Vec<(String, String)> {
        self.entries
            .values()
            .flatten()
            .filter_map(|l| match (&l.installed_by, &l.system_package) {
                (Some(installer), Some(pkg)) => Some((installer.clone(), pkg.clone())),
                _ => None,
            })
            .collect()
    }
}

/// What a re-download of a whole declaration must satisfy, when `@asset=all` locked several.
///
/// The set is compared by name, not by position: a release that reorders its assets is not a
/// change to what was installed. Anything else — a name that was not locked, or one that is
/// locked and no longer resolved — is the same objection [`verify_against`] raises for one.
pub fn verify_set(
    locked: &[ArtifactLock],
    resolved: &[(String, Option<String>)],
) -> Option<String> {
    for (asset, sha) in resolved {
        let Some(entry) = locked.iter().find(|l| &l.asset == asset) else {
            return Some(format!(
                "the lock records {} and this resolved to `{}` as well. Run `linix lock` if \
                 the change is intended.",
                named(locked),
                asset
            ));
        };
        if let Some(objection) = verify_against(entry, asset, sha.as_deref()) {
            return Some(objection);
        }
    }
    if let Some(dropped) = locked
        .iter()
        .find(|l| !resolved.iter().any(|(a, _)| a == &l.asset))
    {
        return Some(format!(
            "the lock records `{}` and this release no longer offers it. Run `linix lock` if \
             the change is intended.",
            dropped.asset
        ));
    }
    None
}

fn named(locks: &[ArtifactLock]) -> String {
    locks
        .iter()
        .map(|l| format!("`{}`", l.asset))
        .collect::<Vec<_>>()
        .join(", ")
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
            selected_by: None,
            sha256: sha.map(str::to_string),
            ..Default::default()
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
        led.record("sharkdp/fd", vec![lock("fd.tar.gz", Some("abc123"))]);
        led.save(&path).unwrap();

        let back = ArtifactLedger::load(&path).unwrap();
        let [entry] = back.locked("sharkdp/fd") else {
            panic!("expected one locked artifact");
        };
        assert_eq!(entry.asset, "fd.tar.gz");
        assert_eq!(entry.url, "https://example.invalid/fd.tar.gz");
        assert_eq!(entry.format, "tarball");
        assert_eq!(entry.sha256.as_deref(), Some("abc123"));
    }

    #[test]
    fn a_declaration_that_installed_several_locks_all_of_them() {
        // `@asset=all` (VIII.2). One entry per declaration could describe only the first.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("locks").join("github.toml");
        let mut led = ArtifactLedger::new();
        led.record(
            "foo/bar",
            vec![
                lock("bar.tar.gz", Some("a1")),
                lock("bar-server.tar.gz", Some("b2")),
            ],
        );
        led.save(&path).unwrap();

        let back = ArtifactLedger::load(&path).unwrap();
        let names: Vec<&str> = back
            .locked("foo/bar")
            .iter()
            .map(|l| l.asset.as_str())
            .collect();
        assert_eq!(names, vec!["bar.tar.gz", "bar-server.tar.gz"]);
    }

    #[test]
    fn recording_replaces_the_whole_set() {
        // A declaration that now resolves to fewer artifacts must not keep the dropped ones.
        let mut led = ArtifactLedger::new();
        led.record(
            "foo/bar",
            vec![lock("a.tar.gz", None), lock("b.tar.gz", None)],
        );
        led.record("foo/bar", vec![lock("a.tar.gz", None)]);
        assert_eq!(led.locked("foo/bar").len(), 1);
    }

    #[test]
    fn forgetting_a_package_drops_its_entry() {
        // A lock left behind after a removal would pin the next install to an artifact
        // chosen for a declaration that no longer exists.
        let mut led = ArtifactLedger::new();
        led.record("sharkdp/fd", vec![lock("fd.tar.gz", None)]);
        assert!(led.forget("sharkdp/fd"));
        assert!(!led.forget("sharkdp/fd"));
        assert!(led.is_empty());
    }

    #[test]
    fn nothing_locked_is_an_empty_slice_not_a_missing_answer() {
        assert!(ArtifactLedger::new().locked("foo/bar").is_empty());
    }

    fn system_lock(asset: &str, installer: &str, pkg: &str) -> ArtifactLock {
        ArtifactLock {
            version: Some("10.2.0".into()),
            asset: asset.into(),
            url: format!("https://example.invalid/{}", asset),
            format: "deb".into(),
            installed_by: Some(installer.into()),
            system_package: Some(pkg.into()),
            ..Default::default()
        }
    }

    #[test]
    fn system_packages_reports_only_handoff_locks() {
        // D5: a `.deb` handed to dpkg records the installer and the name it listed the package
        // under; a plain PATH-deployed artifact contributes nothing to the dedup set.
        let mut led = ArtifactLedger::new();
        led.record(
            "sharkdp/fd",
            vec![system_lock("fd_10.2.0_amd64.deb", "dpkg", "fd")],
        );
        led.record("owner/plain", vec![lock("plain.tar.gz", Some("z9"))]);
        let owned = led.system_packages();
        assert_eq!(owned, vec![("dpkg".to_string(), "fd".to_string())]);
    }

    #[test]
    fn a_handoff_lock_survives_a_toml_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("github.toml");
        let mut led = ArtifactLedger::new();
        led.record(
            "sharkdp/fd",
            vec![system_lock("fd_10.2.0_amd64.deb", "rpm", "fd")],
        );
        led.save(&path).unwrap();
        let back = ArtifactLedger::load(&path).unwrap();
        let l = &back.locked("sharkdp/fd")[0];
        assert_eq!(l.installed_by.as_deref(), Some("rpm"));
        assert_eq!(l.system_package.as_deref(), Some("fd"));
    }

    #[test]
    fn a_set_that_gained_a_file_is_an_objection_naming_what_was_locked() {
        let locked = [lock("a.tar.gz", None)];
        let why = verify_set(
            &locked,
            &[("a.tar.gz".into(), None), ("b.tar.gz".into(), None)],
        )
        .unwrap();
        assert!(why.contains("a.tar.gz"), "{}", why);
        assert!(why.contains("b.tar.gz"), "{}", why);
    }

    #[test]
    fn a_set_that_lost_a_file_is_an_objection_naming_it() {
        let locked = [lock("a.tar.gz", None), lock("b.tar.gz", None)];
        let why = verify_set(&locked, &[("a.tar.gz".into(), None)]).unwrap();
        assert!(why.contains("b.tar.gz"), "{}", why);
    }

    #[test]
    fn the_same_set_in_another_order_is_no_objection() {
        // A release that reorders its assets did not change what is installed.
        let locked = [lock("a.tar.gz", Some("a1")), lock("b.tar.gz", Some("b2"))];
        assert!(verify_set(
            &locked,
            &[
                ("b.tar.gz".into(), Some("b2".into())),
                ("a.tar.gz".into(), Some("a1".into())),
            ]
        )
        .is_none());
    }

    #[test]
    fn a_changed_hash_inside_a_set_is_still_an_objection() {
        let locked = [lock("a.tar.gz", Some("a1")), lock("b.tar.gz", Some("b2"))];
        let why = verify_set(
            &locked,
            &[
                ("a.tar.gz".into(), Some("a1".into())),
                ("b.tar.gz".into(), Some("ZZZ".into())),
            ],
        )
        .unwrap();
        assert!(why.contains("b.tar.gz"), "{}", why);
        assert!(why.contains("ZZZ"), "{}", why);
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
