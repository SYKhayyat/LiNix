use crate::core::{Error, PackageSpec, Result};
use crate::utils::file::persist;
use chrono::{Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    /// An `InProgress` entry `cleanup` aged out at 4h — the process that started it is
    /// gone. Still healable: the mutation may have half-run.
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JournalAction {
    Install(PackageSpec),
    Remove { name: String, backend: String },
}

/// The source of truth for the 'linix heal' command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub action: JournalAction,
    pub status: ActionStatus,
    pub started_at_unix: i64,
    /// Set only on reaching a terminal state; `None` while Pending or InProgress.
    pub finished_at_unix: Option<i64>,
    pub error: Option<String>,
}

/// Recovery from power failure, OS crash, or a kill mid-transaction depends on an
/// 'InProgress' entry being flushed to disk before any backend is invoked. A backend
/// called ahead of that flush is a modification `heal` cannot see or undo.
///
/// **Append-only, one line per state change.** It used to serialise the entire map, pretty
/// printed, through a temp file and a rename, on every transition — so installing 50 packages
/// wrote the whole growing journal ~100 times and the bytes written were O(n²) in the number of
/// actions. Worse, it did that synchronously while holding the one mutex every concurrent DAG
/// worker has to take, which put a hard throttle directly under the transaction's concurrency:
/// the more parallel the graph became, the more this cost. A log is the canonical append-only
/// structure, and appending makes each transition a constant-size write.
///
/// Reading is forward, last-writer-wins per id — the same rule `heal` already applies.
pub struct Journal {
    path: PathBuf,
    pub entries: HashMap<String, JournalEntry>,
}

impl Journal {
    pub fn new() -> Result<Self> {
        Self::at(crate::utils::safe_data_dir().join(Self::FILE_NAME))
    }

    /// `.jsonl`, because it is one JSON value per line and not one JSON document.
    pub const FILE_NAME: &'static str = "journal.jsonl";

    /// The WAL at an explicit path. Injected rather than always derived, so a test kernel
    /// gets its own: `TestKernel` isolated the registry and the groups dir but not this,
    /// so every `cargo test` run appended to the developer's real `journal.json` — 733KB
    /// of test noise in real user data, and a format change to `PackageSpec` then made
    /// that file unparseable and bricked every test at bootstrap.
    pub fn at(path: PathBuf) -> Result<Self> {
        debug!("Initializing WAL at {:?}", path);

        let mut journal = Self {
            path,
            entries: HashMap::new(),
        };

        if journal.path.exists() {
            journal.load_sync()?;
        } else {
            trace!("No existing WAL found, starting fresh.");
        }

        Ok(journal)
    }

    fn load_sync(&mut self) -> Result<()> {
        let data = std::fs::read_to_string(&self.path).map_err(|e| {
            Error::Io(format!(
                "Failed to read WAL Journal at {:?}: {}",
                self.path, e
            ))
        })?;

        if data.trim().is_empty() {
            return Ok(());
        }

        // Forward, last-writer-wins: a later line for the same id supersedes an earlier one,
        // which is how a transition is recorded without rewriting what came before.
        let mut unreadable = 0usize;
        for line in data.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<JournalEntry>(line) {
                Ok(entry) => {
                    self.entries.insert(entry.id.clone(), entry);
                }
                Err(_) => unreadable += 1,
            }
        }

        if unreadable > 0 && self.entries.is_empty() {
            // Nothing at all was readable, so this is not a torn tail — it is a corrupt file.
            // S10: a corrupt WAL must NOT brick every command. It used to return `Err`, which
            // failed `App::new`, which failed everything, with no message saying which file to
            // delete. So: move it aside (preserved for inspection, and so it stops
            // re-triggering), say so loudly (P3 — fail loud), and start fresh.
            let backup = {
                let mut s = self.path.clone().into_os_string();
                s.push(".corrupt");
                std::path::PathBuf::from(s)
            };
            // A preview moves nothing. Setting the file aside is a filesystem change like any
            // other, and `--dry-run heal` on a machine with a damaged WAL was making one.
            let previewing = crate::core::dry_run::active();
            let moved = !previewing && std::fs::rename(&self.path, &backup).is_ok();
            warn!(
                "the WAL at {:?} is corrupt — none of its {} line(s) could be read. {} \
                 Starting a fresh journal so commands still run; an operation interrupted \
                 before this cannot be auto-recovered — re-run `linix sync` to reconcile.",
                self.path,
                unreadable,
                match (previewing, moved) {
                    (true, _) => format!(
                        "A real run would move it to {:?} for inspection; this preview left it \
                         alone.",
                        backup
                    ),
                    (false, true) => format!("It has been moved to {:?} for inspection.", backup),
                    (false, false) => {
                        "It could not be moved aside; it will be overwritten on the next write."
                            .to_string()
                    }
                },
            );
        } else if unreadable > 0 {
            // A crash partway through an append leaves one truncated line, and every complete
            // line before it is still a true record. Those stand; the damage is named.
            warn!(
                "{} line(s) of the WAL at {:?} could not be read and were skipped; {} entr(ies) \
                 were recovered. If an operation was interrupted it may not be auto-healable — \
                 run `linix sync` to reconcile.",
                unreadable,
                self.path,
                self.entries.len()
            );
        } else {
            debug!(
                "Successfully loaded {} historical log entries.",
                self.entries.len()
            );
        }
        Ok(())
    }

    /// Record one entry's current state, durably, before the backend it describes is invoked.
    ///
    /// One line appended and synced — not a re-serialisation of every entry ever recorded.
    fn append(&self, entry: &JournalEntry) -> Result<()> {
        trace!("appending to WAL");
        let line = serde_json::to_string(entry)
            .map_err(|e| Error::Other(format!("Failed to serialize Journal entry: {}", e)))?;
        // A preview records no WAL entry: `append_line` answers that, and a run that performed
        // nothing has nothing to roll back.
        crate::utils::file::append_line(&self.path, &line)
            .map(|_| ())
            .map_err(|e| Error::Persist(format!("Write of WAL Journal failed: {}", e)))
    }

    /// Rewrite the log from the in-memory entries, dropping everything they no longer contain.
    ///
    /// Only `cleanup` needs this — removal is the one thing an append cannot express — and it
    /// runs once per invocation, not once per package.
    fn compact(&self) -> Result<()> {
        let mut data = String::new();
        for entry in self.entries.values() {
            data.push_str(
                &serde_json::to_string(entry)
                    .map_err(|e| Error::Other(format!("Failed to serialize Journal: {}", e)))?,
            );
            data.push('\n');
        }
        persist(&self.path, &data)
            .map(|_| ())
            .map_err(|e| Error::Persist(format!("Atomic rewrite of WAL Journal failed: {}", e)))
    }

    fn generate_id(backend: &str, package: &str) -> String {
        format!("{}:{}:{}", backend, package, Uuid::new_v4().simple())
    }

    /// MUST be called and flushed before invoking any backend command.
    pub fn record_start(&mut self, action: JournalAction) -> Result<String> {
        let (b_name, p_name) = match &action {
            JournalAction::Install(s) => (&s.backend, &s.name),
            JournalAction::Remove { name, backend } => (backend, name),
        };

        let id = Self::generate_id(b_name, p_name);

        let entry = JournalEntry {
            id: id.clone(),
            action,
            status: ActionStatus::InProgress,
            started_at_unix: Utc::now().timestamp(),
            finished_at_unix: None,
            error: None,
        };

        self.append(&entry)?;
        self.entries.insert(id.clone(), entry);

        debug!("Operation {} marked as InProgress in WAL.", id);
        Ok(id)
    }

    pub fn record_success(&mut self, id: &str) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Completed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            let entry = entry.clone();
            self.append(&entry)?;
            trace!("Operation {} marked as Completed.", id);
        } else {
            warn!("Attempted to mark unknown operation {} as successful.", id);
        }
        Ok(())
    }

    pub fn record_failure(&mut self, id: &str, err: &str) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Failed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            entry.error = Some(err.to_string());
            let entry = entry.clone();
            self.append(&entry)?;
            // `debug!`, not `warn!`: the user is about to be told this failure once, in their
            // own words, by whoever is returning the error. Saying it again here — with a
            // 32-hex operation id and the word WAL in it — is the same sentence a third time
            // in vocabulary that belongs to the journal, not to the person who typed a typo.
            debug!("Operation {} recorded as Failed in WAL: {}", id, err);
        } else {
            warn!("Attempted to record failure for unknown operation {}.", id);
        }
        Ok(())
    }

    /// Everything that touched the system and did not finish — what `heal` can act on.
    ///
    /// `Pending` is excluded because it never reached a backend. `Abandoned` is included:
    /// it is an `InProgress` entry that `cleanup` aged out at 4h, and aging out is a
    /// statement about how long ago the process died, not about whether the package it was
    /// mutating is still half-removed. Excluding it meant a crash left unattended over
    /// lunch stopped being healable at all — the case where the machine is least likely to
    /// have been put right by hand in the meantime.
    pub fn get_incomplete_actions(&self) -> Vec<JournalEntry> {
        self.entries
            .values()
            .filter(|e| {
                matches!(
                    e.status,
                    ActionStatus::InProgress | ActionStatus::Failed | ActionStatus::Abandoned
                )
            })
            .cloned()
            .collect()
    }

    /// True makes `sync` run `heal` on its own, without asking.
    pub fn needs_recovery(&self) -> bool {
        self.entries
            .values()
            .any(|e| matches!(e.status, ActionStatus::InProgress | ActionStatus::Abandoned))
    }

    /// InProgress and Failed entries are NEVER purged until they are resolved. Returns whether
    /// anything was dropped, which is also whether the log on disk was rewritten.
    pub fn cleanup_expired_logs(&mut self, days_threshold: i64) -> Result<bool> {
        let cutoff = Utc::now() - ChronoDuration::days(days_threshold);
        let cutoff_ts = cutoff.timestamp();

        let initial_count = self.entries.len();

        self.entries.retain(|id, entry| {
            let is_terminal =
                entry.status == ActionStatus::Completed || entry.status == ActionStatus::Abandoned;

            if is_terminal {
                let terminal_time = entry.finished_at_unix.unwrap_or(entry.started_at_unix);
                if terminal_time < cutoff_ts {
                    trace!("Pruning expired log record: {}", id);
                    return false;
                }
            }
            true
        });

        let purged = initial_count - self.entries.len();
        if purged > 0 {
            info!(
                "Maintenance complete. Purged {} historical records older than {} days.",
                purged, days_threshold
            );
            // A removal is the one transition an append cannot express.
            self.compact()?;
        }

        Ok(purged > 0)
    }

    pub fn cleanup(&mut self) -> Result<()> {
        debug!("journal maintenance");

        // An InProgress entry older than this is read as a crashed process, not a slow one:
        // the wrong call either abandons a live install or waits forever on a dead one.
        let stale_limit = Utc::now() - ChronoDuration::hours(4);
        let stale_ts = stale_limit.timestamp();

        let mut aged_out = false;
        for entry in self.entries.values_mut() {
            if entry.status == ActionStatus::InProgress && entry.started_at_unix < stale_ts {
                debug!("Marking stale task {} as Abandoned.", entry.id);
                entry.status = ActionStatus::Abandoned;
                entry.finished_at_unix = Some(Utc::now().timestamp());
                aged_out = true;
            }
        }

        let purged = self.cleanup_expired_logs(7)?;
        // `cleanup_expired_logs` rewrites when it drops something; aging an entry out without
        // dropping anything still has to reach the disk, or the next process re-ages it.
        if aged_out && !purged {
            self.compact()?;
        }

        if self.entries.is_empty() && self.path.exists() {
            trace!("WAL is empty. Removing journal file.");
            let _ = std::fs::remove_file(&self.path);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn an_aged_out_crash_is_still_healable() {
        // R23: `cleanup` flips an InProgress entry to Abandoned after 4h. That must not take
        // it out of heal's reach -- a node aborted mid-removal never entered the rollback
        // history, so the WAL is the only thing that knows the package is half-removed.
        let tmp = tempdir().unwrap();
        let mut journal = Journal::at(tmp.path().join("journal.json")).unwrap();

        let id = journal
            .record_start(JournalAction::Remove {
                name: "python3".into(),
                backend: "apt".into(),
            })
            .unwrap();

        // Backdate past the 4h staleness limit, then age it out.
        journal.entries.get_mut(&id).unwrap().started_at_unix =
            (Utc::now() - ChronoDuration::hours(5)).timestamp();
        journal.cleanup().unwrap();

        assert_eq!(
            journal.entries[&id].status,
            ActionStatus::Abandoned,
            "cleanup should still age the entry out"
        );
        assert!(
            journal.needs_recovery(),
            "an abandoned mutation must still trigger a heal"
        );
        assert!(
            journal.get_incomplete_actions().iter().any(|e| e.id == id),
            "an abandoned mutation must still be offered to heal"
        );
    }

    #[test]
    fn a_corrupt_wal_does_not_brick_every_command() {
        // S10: a bad parse used to fail App::new and therefore every command. It must
        // instead recover: move the bad file aside, start fresh, and still construct.
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("journal.json");
        std::fs::write(&path, b"{ this is not valid json ]]").unwrap();

        let journal = Journal::at(path.clone()).expect("a corrupt WAL must not fail construction");

        // Started fresh...
        assert!(!journal.needs_recovery());
        // ...the bad file was set aside for inspection...
        let backup = {
            let mut s = path.clone().into_os_string();
            s.push(".corrupt");
            std::path::PathBuf::from(s)
        };
        assert!(
            backup.exists(),
            "the corrupt WAL should be preserved at {:?}",
            backup
        );
        // ...and it is no longer at the live path (so it won't re-trigger).
        assert!(
            !path.exists(),
            "the corrupt WAL should have been moved off the live path"
        );
    }

    #[test]
    fn a_missing_wal_starts_fresh_without_error() {
        let tmp = tempdir().unwrap();
        let journal = Journal::at(tmp.path().join("nope.json")).unwrap();
        assert!(!journal.needs_recovery());
    }
}
