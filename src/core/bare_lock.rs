//! `locks/bare.toml` — which package manager a bare name resolved to (II.6).
//!
//! A line with no prefix (`ripgrep`) is answered by asking each backend in `priority` order
//! whether it has that name and taking the first yes (II.7 step 4). Unrecorded, that answer
//! is re-derived every run against whatever is installed *now* — so adding a package manager
//! that sits higher in `priority` and happens to publish the same name silently changes what
//! an unedited line means. The record is the fix: asked once, then the same answer until you
//! say otherwise.
//!
//! **The file is the switch, and deleting is how you unfreeze** (II.15's rule, applied here):
//! an entry means frozen, no entry means ask. Removing a line re-asks and records the new
//! answer. There is no command for it, because the file is yours and a text editor is the
//! command.
//!
//! One file rather than one per backend, unlike the rest of `locks/`: the fact recorded is
//! about a *name*, and a name that moves backends would otherwise be two writes — a delete
//! from one file and an insert into another — for one fact changing.

use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `name -> backend`. A `BTreeMap` so the file is ordered and diffs cleanly in git.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct BareLock {
    #[serde(default)]
    resolved: BTreeMap<String, String>,
}

impl BareLock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn path_in(locks_dir: &Path) -> PathBuf {
        locks_dir.join("bare.toml")
    }

    /// A missing file means nothing has been resolved yet — the correct starting state on a
    /// fresh repo, and never an error.
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
            .map_err(|e| Error::Toml(format!("serializing the bare-name lock: {}", e)))?;
        std::fs::write(path, body)
            .map_err(|e| Error::Io(format!("writing {}: {}", path.display(), e)))
    }

    /// The backend this name is frozen to, if any.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.resolved.get(name).map(String::as_str)
    }

    /// Record an answer. Returns whether the file needs writing — an answer that is already
    /// recorded is not a change, and rewriting an unchanged lock on every sync would make
    /// every run a git commit.
    pub fn record(&mut self, name: &str, backend: &str) -> bool {
        match self.resolved.get(name) {
            Some(existing) if existing == backend => false,
            _ => {
                self.resolved.insert(name.to_string(), backend.to_string());
                true
            }
        }
    }

    /// Forget every name that is no longer declared anywhere.
    ///
    /// Without this the file only grows, and a stale entry is worse than a missing one: it
    /// freezes an answer for a line that no longer exists, and would silently apply again if
    /// the name came back.
    pub fn retain_declared(&mut self, declared: &[String]) -> bool {
        let before = self.resolved.len();
        self.resolved.retain(|n, _| declared.iter().any(|d| d == n));
        self.resolved.len() != before
    }

    pub fn is_empty(&self) -> bool {
        self.resolved.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn a_recorded_name_keeps_its_backend() {
        let mut lock = BareLock::new();
        assert!(lock.record("ripgrep", "cargo"));
        assert_eq!(lock.get("ripgrep"), Some("cargo"));
        // Recording the same answer is not a change: an unchanged lock must not be rewritten,
        // or every sync becomes a commit.
        assert!(!lock.record("ripgrep", "cargo"));
        assert!(lock.record("ripgrep", "apt"));
    }

    #[test]
    fn deleting_the_entry_is_how_you_unfreeze() {
        // II.15's rule: the file is the switch. Nothing here does the unfreezing — the user's
        // editor does — so what this asserts is that a lock with no entry has no opinion.
        let lock = BareLock::new();
        assert_eq!(lock.get("ripgrep"), None);
    }

    #[test]
    fn a_name_nobody_declares_any_more_is_forgotten() {
        let mut lock = BareLock::new();
        lock.record("ripgrep", "cargo");
        lock.record("gone", "apt");
        assert!(lock.retain_declared(&["ripgrep".to_string()]));
        assert_eq!(lock.get("gone"), None);
        assert!(!lock.retain_declared(&["ripgrep".to_string()]), "no change");
    }

    #[test]
    fn a_missing_file_is_an_empty_lock_and_not_an_error() {
        let tmp = TempDir::new().unwrap();
        let path = BareLock::path_in(&tmp.path().join("locks"));
        assert!(BareLock::load(&path).unwrap().is_empty());
    }

    #[test]
    fn it_round_trips_through_the_file() {
        let tmp = TempDir::new().unwrap();
        let path = BareLock::path_in(&tmp.path().join("locks"));
        let mut lock = BareLock::new();
        lock.record("ripgrep", "cargo");
        lock.save(&path).unwrap();
        assert_eq!(BareLock::load(&path).unwrap().get("ripgrep"), Some("cargo"));
    }
}
