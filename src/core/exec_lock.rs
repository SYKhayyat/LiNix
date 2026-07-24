//! `locks/exec.toml` — how many times a given script *content* has run here (XIII.3).
//!
//! `when` decides whether the machine wants a script. It cannot decide whether the script
//! already happened: variables are resolved once and frozen into the plan (W4/W13), so a
//! condition the script itself would falsify is still true within the run that executes it.
//! That is what this ledger is for.
//!
//! **The hash is the identity, not the path.** Editing a script makes it a different script,
//! so it runs again; renaming it does not. Same reasoning II.12 uses for artifacts: what you
//! declared is the content.
//!
//! **A row is never dropped for a `when` that went false.** Dropping it would make a condition
//! that flaps — a laptop on battery, a host that comes and goes — re-run the script every time
//! the condition swung back true, because the count would have been forgotten. The count means
//! *"this content has run n times on this machine"*, full stop.

use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What the ledger remembers about one script content.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExecRecord {
    /// How many times this content has run on this machine.
    pub count: u32,
    /// When it last ran, RFC 3339. Recorded for a human reading the file; nothing decides on
    /// it, because a decision that reads the clock is a decision that changes without an edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<String>,
}

/// `locks/exec.toml`. A `BTreeMap` so the file is ordered and diffs cleanly in git.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExecLedger {
    #[serde(default)]
    runs: BTreeMap<String, ExecRecord>,
}

/// How many times a script content may run. The default is once per distinct content;
/// `@runs=always` is the explicit opt-out, and being explicit is the point — nothing becomes a
/// per-sync command by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ceiling {
    Times(u32),
    Always,
}

impl Ceiling {
    /// Read `@runs=`. `None` (the key absent) is the default ceiling of one.
    ///
    /// An unparseable value is refused by the grammar (`validate_exec`), so this cannot invent
    /// a ceiling from a typo: anything that is not `always` and not a positive number never
    /// reaches here.
    pub fn read(value: Option<&str>) -> Ceiling {
        match value.map(str::trim) {
            None => Ceiling::Times(1),
            Some("always") => Ceiling::Always,
            Some(n) => n.parse::<u32>().map(Ceiling::Times).unwrap_or(Ceiling::Times(1)),
        }
    }

    /// Whether a content that has already run `count` times may run again.
    pub fn permits(self, count: u32) -> bool {
        match self {
            Ceiling::Always => true,
            Ceiling::Times(max) => count < max,
        }
    }
}

impl std::fmt::Display for Ceiling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ceiling::Always => f.write_str("always"),
            Ceiling::Times(n) => write!(f, "{}", n),
        }
    }
}

impl ExecLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn path_in(locks_dir: &Path) -> PathBuf {
        locks_dir.join("exec.toml")
    }

    /// A missing file means nothing has run yet — the correct starting state, never an error.
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
            .map_err(|e| Error::Toml(format!("serializing the exec ledger: {}", e)))?;
        std::fs::write(path, body)
            .map_err(|e| Error::Io(format!("writing {}: {}", path.display(), e)))
    }

    /// How many times this content has run here. An unknown hash has run zero times, which is
    /// what makes a newly-edited script run again.
    pub fn count(&self, hash: &str) -> u32 {
        self.runs.get(hash).map(|r| r.count).unwrap_or(0)
    }

    /// Record one completed run. `at` is passed in rather than read from the clock here so the
    /// ledger stays pure and a test can pin the stamp.
    pub fn record_run(&mut self, hash: &str, at: String) {
        let entry = self.runs.entry(hash.to_string()).or_default();
        entry.count += 1;
        entry.last_run = Some(at);
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_content_has_never_run() {
        assert_eq!(ExecLedger::new().count("deadbeef"), 0);
    }

    #[test]
    fn a_run_is_counted_and_stamped() {
        let mut l = ExecLedger::new();
        l.record_run("abc", "2026-07-24T00:00:00Z".into());
        assert_eq!(l.count("abc"), 1);
        l.record_run("abc", "2026-07-24T01:00:00Z".into());
        assert_eq!(l.count("abc"), 2);
        // A second content is its own count — the hash is the identity.
        assert_eq!(l.count("xyz"), 0);
    }

    #[test]
    fn the_default_ceiling_is_once_per_content() {
        let c = Ceiling::read(None);
        assert!(c.permits(0), "a script that has never run must run");
        assert!(!c.permits(1), "run-once means not twice");
    }

    #[test]
    fn always_never_stops() {
        let c = Ceiling::read(Some("always"));
        assert!(c.permits(0));
        assert!(c.permits(9_999));
    }

    #[test]
    fn a_numeric_ceiling_counts() {
        let c = Ceiling::read(Some("3"));
        assert!(c.permits(2));
        assert!(!c.permits(3));
    }

    /// Editing the script changes its hash, so the new content's count is zero and it runs —
    /// this is the whole idempotency model in one assertion.
    #[test]
    fn an_edited_script_is_a_different_content_and_runs_again() {
        let mut l = ExecLedger::new();
        l.record_run("hash-of-v1", "t".into());
        assert!(!Ceiling::read(None).permits(l.count("hash-of-v1")));
        assert!(Ceiling::read(None).permits(l.count("hash-of-v2")));
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_ledger() {
        assert!(ExecLedger::load(Path::new("does/not/exist/exec.toml"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn the_ledger_round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = ExecLedger::path_in(dir.path());
        let mut l = ExecLedger::new();
        l.record_run("abc", "2026-07-24T00:00:00Z".into());
        l.save(&path).unwrap();
        let back = ExecLedger::load(&path).unwrap();
        assert_eq!(back, l);
        assert_eq!(back.count("abc"), 1);
    }
}
