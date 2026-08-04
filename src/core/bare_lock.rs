//! `locks/bare.HOST.toml` — which package manager an unpinned name resolved to (II.6).
//!
//! A line that does not pin one manager (`ripgrep`, or `apt,dnf:ripgrep`) is answered by
//! asking its candidates in order whether they have that name and taking the first yes (II.7
//! step 4). Unrecorded, that answer is re-derived every run against whatever is installed
//! *now* — so adding a package manager that sits higher in `priority` and happens to publish
//! the same name silently changes what an unedited line means. The record is the fix: asked
//! once, then the same answer until you say otherwise.
//!
//! **One file per host.** Which manager has `ripgrep` is a fact about a machine, and
//! `locks/` travels with the config to every machine that shares it. A single file would
//! hold whichever answer synced last: the Ubuntu box writes `apt`, the Fedora box overwrites
//! with `dnf`, and the two rewrite each other on every sync and collide in git forever. A
//! file per host means each machine writes only its own, every file commits cleanly, and
//! each machine still reproduces exactly what it had.
//!
//! **Deleting is how you unfreeze** (II.15's rule, applied here): an entry means frozen, no
//! entry means ask. `linix unlock backends` removes entries; so does an editor, because the file is
//! yours.
//!
//! One file per host rather than one per backend, unlike the rest of `locks/`: the fact
//! recorded is about a *name*, and a name that moves backends would otherwise be two writes —
//! a delete from one file and an insert into another — for one fact changing.

use crate::core::ledger::LockFile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `name -> backend`. A `BTreeMap` so the file is ordered and diffs cleanly in git.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct BareLock {
    #[serde(default)]
    resolved: BTreeMap<String, String>,
}

impl LockFile for BareLock {
    const WHAT: &'static str = "the bare-name lock";
}

impl BareLock {
    /// This machine's file. Every other machine sharing the config has its own, so a sync
    /// here can never overwrite the answer another host depends on.
    pub fn path_in(locks_dir: &Path) -> PathBuf {
        Self::path_for(locks_dir, &crate::config::Config::get_hostname())
    }

    /// Anything a hostname may legally contain but a filename should not becomes `_`, so a
    /// host called `../etc` writes inside `locks/` like every other host.
    pub fn path_for(locks_dir: &Path, host: &str) -> PathBuf {
        let safe: String = host
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let safe = if safe.is_empty() {
            "unknown".to_string()
        } else {
            safe
        };
        locks_dir.join(format!("bare.{}.toml", safe.to_lowercase()))
    }

    /// Forget one name, so it is asked again. Reports whether there was anything to forget.
    pub fn forget(&mut self, name: &str) -> bool {
        self.resolved.remove(name).is_some()
    }

    /// Forget everything.
    pub fn clear(&mut self) -> bool {
        let had = !self.resolved.is_empty();
        self.resolved.clear();
        had
    }

    /// Every frozen name and the manager it is frozen to, for `linix lock backends --list`.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.resolved.iter().map(|(n, b)| (n.as_str(), b.as_str()))
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
    fn each_host_keeps_its_own_answers() {
        // The reason the file is per-host: `locks/` is shared, but which manager has a name
        // is not. Two machines must not be able to overwrite each other.
        let locks = Path::new("locks");
        assert_ne!(
            BareLock::path_for(locks, "ubuntu-box"),
            BareLock::path_for(locks, "fedora-box")
        );
        assert_eq!(
            BareLock::path_for(locks, "Ubuntu-Box"),
            BareLock::path_for(locks, "ubuntu-box"),
            "a host that shouts its name is the same host"
        );
    }

    #[test]
    fn a_hostname_cannot_write_outside_the_locks_directory() {
        let path = BareLock::path_for(Path::new("locks"), "../../etc/passwd");
        assert_eq!(path.parent(), Some(Path::new("locks")));
    }

    #[test]
    fn unlocking_forgets_and_says_whether_there_was_anything_to_forget() {
        let mut lock = BareLock::new();
        lock.record("ripgrep", "cargo");
        assert!(lock.forget("ripgrep"));
        assert_eq!(lock.get("ripgrep"), None);
        assert!(!lock.forget("ripgrep"), "forgetting twice is not a change");

        lock.record("jq", "apt");
        assert!(lock.clear());
        assert!(lock.is_empty());
        assert!(!lock.clear());
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
