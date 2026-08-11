//! The file rules every `locks/` record obeys, written once.
//!
//! Six ledgers live under `locks/` — the regex expansions, the bare-name resolutions, the exec
//! run counts, the hook approvals, the artifact selections, the applied extras. They carry six
//! different records and they had six identical carriers: the same `load` that reads a missing
//! file as empty, the same `save` that goes through `persist` and skips the directory create
//! during a dry run, down to the same two-line comment about `--dry-run lock` copy-pasted into
//! all six files.
//!
//! **The duplication was not carelessness, which is why collapsing it is worth doing.** All six
//! honoured the dry-run rule — not four of six. A copy-and-edit process leaves that bug in half
//! the family; this was one rule found once and correctly written out six times. The cost was
//! never the lines. The cost is that the *seventh* ledger inherits the rules only if whoever
//! writes it remembers them, and `locks/versions.json` and `locks/hooks.toml` being left behind
//! by `shall --dry-run lock` is what remembering them looks like when it fails.
//!
//! So the rules are a trait with provided methods, and a new ledger gets them by existing.
//!
//! **What is deliberately NOT here:** where each file lives. Four ledgers are
//! `locks/<fixed>.toml`, `bare` is per-host (so a sync on one machine cannot overwrite the
//! answer another depends on) and `artifact` is per-backend. That is a table of six different
//! answers, not one rule with six copies, and folding it in would mean an abstraction that has
//! to be argued with every time it is used.

use crate::core::{Error, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;

/// A record that is one TOML file under `locks/`.
///
/// Implementors supply the record and its name; the reading, the writing, the missing-file
/// rule and the dry-run rule come from here.
pub trait LockFile: Default + Serialize + DeserializeOwned + Sized {
    /// What this file is called when an error has to name it — "the regex lock", "the exec
    /// ledger". A sentence fragment for a human deciding which file to go and open, which is
    /// why it is not the filename.
    const WHAT: &'static str;

    fn new() -> Self {
        Self::default()
    }

    /// Read the file. **A missing file is the correct starting state, never an error**: every
    /// one of these ledgers begins life absent, on a fresh repo and on a machine that has not
    /// yet used the feature. A ledger that errored on absence would make a first run fail.
    fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s)
                .map_err(|e| Error::Toml(format!("reading {}: {}", path.display(), e))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(Error::Io(format!("reading {}: {}", path.display(), e))),
        }
    }

    /// Write the file.
    ///
    /// Through `persist`, and the directory is not created during a dry run — **a preview must
    /// not leave an approval or a pin behind.** `shall --dry-run lock` used to write
    /// `locks/versions.json` and `locks/hooks.toml` for real, which is a preview that changes
    /// what the next real run will do.
    fn save(&self, path: &Path) -> Result<()> {
        if !crate::core::dry_run::active() {
            if let Some(dir) = path.parent() {
                crate::utils::file::ensure_dir(dir)?;
            }
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| Error::Toml(format!("serializing {}: {}", Self::WHAT, e)))?;
        crate::utils::file::persist(path, &body).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    #[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Probe {
        #[serde(default)]
        entries: BTreeMap<String, String>,
    }

    impl LockFile for Probe {
        const WHAT: &'static str = "the probe ledger";
    }

    #[test]
    fn a_missing_file_loads_empty_rather_than_failing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("locks").join("nothing-here.toml");
        assert_eq!(Probe::load(&path).unwrap(), Probe::new());
    }

    #[test]
    fn it_round_trips_through_the_file_and_creates_the_directory() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("locks").join("probe.toml");
        let mut p = Probe::new();
        p.entries.insert("a".to_string(), "b".into());
        p.save(&path).unwrap();
        assert!(path.exists(), "save did not create locks/");
        assert_eq!(Probe::load(&path).unwrap(), p);
    }

    // The dry-run rule is asserted in `tests/ledger_file_rules_tests.rs`, not here.
    // `dry_run` is a process-wide atomic set once from `main`, so a unit test that flips it
    // would flip it for every other test sharing this binary — and the tests it would break
    // are the ones that write files, which is most of them. Its own process, its own answer.

    /// An unreadable file is an error, not silently empty. The missing-file rule is about
    /// *absence*; a file that exists and cannot be parsed is a fact the user needs told, and
    /// swallowing it would silently discard every recorded expansion, approval and pin.
    #[test]
    fn a_corrupt_file_is_an_error_and_not_an_empty_ledger() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("probe.toml");
        std::fs::write(&path, "this is not toml { [ ").unwrap();
        let err = Probe::load(&path).unwrap_err();
        assert!(
            matches!(err, Error::Toml(_)),
            "a corrupt ledger must not read as empty; got {err:?}"
        );
    }
}
