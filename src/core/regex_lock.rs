//! `locks/regex.toml` — what each `re:` pattern expanded to (II.15).
//!
//! **The lock file IS the switch** (II.15). An entry means the pattern is frozen to that list;
//! no entry means it is asked again. There is no command either way: the first expansion
//! records itself, and deleting the entry is how you re-find (owner ruling, 2026-07-21).
//!
//! Freezing is the point, not a cache. `apt:re:^lib` matched 30,207 packages when it was
//! measured; a pattern re-expanded on every sync means the machine grows a package the day
//! someone else's upload happens to match, with nothing in your files changing and nothing to
//! review. The recorded list is in git, so what the pattern means is a diff.
//!
//! **Residual hole, accepted (II.15):** a package renamed out of the pattern silently drops
//! one package. One package, recoverable, and the snapshot has your back.

use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `backend:pattern -> the names it matched`. Keyed by both because the same pattern against
/// two managers is two different questions with two different answers.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RegexLock {
    #[serde(default)]
    expanded: BTreeMap<String, Vec<String>>,
}

pub fn key(backend: &str, pattern: &str) -> String {
    format!("{}:{}", backend, pattern)
}

impl RegexLock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn path_in(locks_dir: &Path) -> PathBuf {
        locks_dir.join("regex.toml")
    }

    /// A missing file means nothing is frozen yet — the correct starting state, never an error.
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
            .map_err(|e| Error::Toml(format!("serializing the regex lock: {}", e)))?;
        std::fs::write(path, body)
            .map_err(|e| Error::Io(format!("writing {}: {}", path.display(), e)))
    }

    pub fn get(&self, backend: &str, pattern: &str) -> Option<&[String]> {
        self.expanded.get(&key(backend, pattern)).map(Vec::as_slice)
    }

    /// Record an expansion. Returns whether the file needs writing — an unchanged lock
    /// rewritten every run would make every sync a commit.
    pub fn record(&mut self, backend: &str, pattern: &str, mut names: Vec<String>) -> bool {
        names.sort();
        names.dedup();
        match self.expanded.get(&key(backend, pattern)) {
            Some(existing) if *existing == names => false,
            _ => {
                self.expanded.insert(key(backend, pattern), names);
                true
            }
        }
    }

    /// Forget every pattern no line declares any more, so the file cannot freeze an answer
    /// for a pattern that is gone and apply it again if the line comes back.
    pub fn retain_declared(&mut self, declared: &[String]) -> bool {
        let before = self.expanded.len();
        self.expanded.retain(|k, _| declared.iter().any(|d| d == k));
        self.expanded.len() != before
    }

    pub fn is_empty(&self) -> bool {
        self.expanded.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn one_pattern_against_two_backends_is_two_answers() {
        let mut lock = RegexLock::new();
        lock.record("apt", "^fonts-", vec!["fonts-a".into()]);
        lock.record("pacman", "^fonts-", vec!["fonts-z".into()]);
        assert_eq!(lock.get("apt", "^fonts-"), Some(&["fonts-a".to_string()][..]));
        assert_eq!(
            lock.get("pacman", "^fonts-"),
            Some(&["fonts-z".to_string()][..])
        );
    }

    #[test]
    fn recording_the_same_expansion_is_not_a_change() {
        let mut lock = RegexLock::new();
        assert!(lock.record("apt", "^fonts-", vec!["b".into(), "a".into()]));
        // Sorted on the way in, so the order the manager happened to print in is not a diff.
        assert!(!lock.record("apt", "^fonts-", vec!["a".into(), "b".into()]));
        assert!(lock.record("apt", "^fonts-", vec!["a".into()]));
    }

    #[test]
    fn a_pattern_nobody_declares_any_more_is_forgotten() {
        let mut lock = RegexLock::new();
        lock.record("apt", "^fonts-", vec!["fonts-a".into()]);
        lock.record("apt", "^gone-", vec!["gone-a".into()]);
        assert!(lock.retain_declared(&[key("apt", "^fonts-")]));
        assert_eq!(lock.get("apt", "^gone-"), None);
    }

    #[test]
    fn it_round_trips_through_the_file() {
        let tmp = TempDir::new().unwrap();
        let path = RegexLock::path_in(&tmp.path().join("locks"));
        assert!(RegexLock::load(&path).unwrap().is_empty());
        let mut lock = RegexLock::new();
        lock.record("apt", "^fonts-", vec!["fonts-a".into(), "fonts-b".into()]);
        lock.save(&path).unwrap();
        assert_eq!(
            RegexLock::load(&path).unwrap().get("apt", "^fonts-"),
            Some(&["fonts-a".to_string(), "fonts-b".to_string()][..])
        );
    }
}
