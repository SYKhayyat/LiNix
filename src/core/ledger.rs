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

/// Read a backend's JSON record of what it deployed: absent is empty, unparseable is an error.
///
/// **This is [`LockFile::load`]'s rule, one file format over**, and it is here rather than in
/// each backend for the reason this module exists at all. The three download backends each
/// read their record with `read_to_string(..).unwrap_or_default()` followed by
/// `from_str(..).unwrap_or_default()`, so a truncated or hand-edited file arrived as an empty
/// map with no error and no log line. The read was not the damage: `commit_state` merges the
/// run's delta into that empty map and persists it, so the next run makes the loss permanent —
/// every recorded artifact forgotten, its `bin_link` no longer known, and the files still on
/// disk and on PATH with nothing owning them.
pub async fn load_json_records<T: DeserializeOwned>(
    path: &Path,
) -> Result<std::collections::HashMap<String, T>> {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => serde_json::from_str(&s)
            .map_err(|e| Error::Json(format!("reading {}: {}", path.display(), e))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(std::collections::HashMap::new()),
        Err(e) => Err(Error::Io(format!("reading {}: {}", path.display(), e))),
    }
}

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
    /// **The caller is answerable for the lock.** Every production `save` outside
    /// [`update`](Self::update) is reached from a `Writer` verb, which holds the data lock for
    /// its whole run — and `a_ledger_is_read_and_written_as_one_step_tests` is what keeps that
    /// list closed. Taking the lock here instead would be a leaf function reaching for a
    /// process-wide lock it cannot know the scope of, and would serialise every test that
    /// writes a ledger behind one file in the developer's real data directory.
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

    /// Read, change and write as one indivisible step.
    ///
    /// **This is the door, and `load` followed later by `save` is the bug it replaces.** Every
    /// one of these files is written whole, so two processes that each read it, each change
    /// their own copy and each write it back leave one of the two changes gone — and the
    /// changes are approvals, pins and resolutions, so the one that loses is a hook that has to
    /// be approved again or a version that comes unpinned. The window is the whole of the work
    /// between the read and the write, which for a `sync` is every package it installs.
    ///
    /// The caller states its change as a *delta* against whatever is on disk now, rather than
    /// handing over a copy it read minutes ago. That is what makes the merge possible at all:
    /// a whole-file copy carries the other process's entries as absences, and writing it back
    /// is how they are lost.
    /// A change that fails writes nothing: half of a set of approvals is worse than none of
    /// them, because the half that landed looks deliberate. The error type is the caller's, so
    /// a verb that speaks `anyhow` keeps speaking it.
    fn update<T, E>(
        path: &Path,
        change: impl FnOnce(&mut Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        let _guard =
            crate::core::datalock::DataLock::for_this_write(Self::WHAT).map_err(E::from)?;
        let mut current = Self::load(path).map_err(E::from)?;
        let outcome = change(&mut current)?;
        current.save(path).map_err(E::from)?;
        Ok(outcome)
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

    /// **The same rule for the JSON records, because the same bug lived in three of them.**
    ///
    /// A truncated `web`/`github`/`appimage` state file read as an empty map, and the map was
    /// then merged into and written back — so the second run made the loss permanent. Absent
    /// is still the correct starting state; unparseable is not.
    #[tokio::test]
    async fn a_corrupt_json_record_is_an_error_and_a_missing_one_is_empty() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("never-written.json");
        let empty: std::collections::HashMap<String, String> =
            load_json_records(&missing).await.unwrap();
        assert!(empty.is_empty(), "a missing record starts empty");

        let corrupt = tmp.path().join("torn.json");
        std::fs::write(&corrupt, "{\"fd\": {\"version\"").unwrap();
        let err = load_json_records::<String>(&corrupt).await.unwrap_err();
        assert!(
            matches!(err, Error::Json(_)),
            "a corrupt record must not read as empty; got {err:?}"
        );
    }
}
