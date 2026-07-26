use crate::core::{Error, PackageSpec, Result};
use crate::utils::file::atomic_write;
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
    /// Merged into the StateRegistry only once the action succeeds.
    pub staged_properties: HashMap<String, String>,
}

/// Recovery from power failure, OS crash, or a kill mid-transaction depends on an
/// 'InProgress' entry being flushed to disk before any backend is invoked. A backend
/// called ahead of that flush is a modification `heal` cannot see or undo.
pub struct Journal {
    path: PathBuf,
    pub entries: HashMap<String, JournalEntry>,
}

impl Journal {
    pub fn new() -> Result<Self> {
        Self::at(crate::utils::safe_data_dir().join("journal.json"))
    }

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

        match serde_json::from_str(&data) {
            Ok(entries) => {
                self.entries = entries;
                debug!(
                    "Successfully loaded {} historical log entries.",
                    self.entries.len()
                );
            }
            Err(e) => {
                // S10: a corrupt WAL must NOT brick every command. It used to return Err,
                // which failed `App::new`, which failed everything — with no message saying
                // which file to delete. The WAL only records in-flight actions for crash
                // recovery; a corrupt one means we cannot auto-recover an interrupted run, but
                // that is no reason to refuse `list`, `plan`, or anything else. So: move the
                // bad file aside (preserved for inspection, and so it stops re-triggering),
                // say so loudly (P3 — fail loud), and start fresh.
                let backup = {
                    let mut s = self.path.clone().into_os_string();
                    s.push(".corrupt");
                    std::path::PathBuf::from(s)
                };
                let moved = std::fs::rename(&self.path, &backup).is_ok();
                self.entries = HashMap::new();
                warn!(
                    "the WAL at {:?} is corrupt and could not be parsed ({}). {} \
                     Starting a fresh journal so commands still run; an operation interrupted \
                     before this cannot be auto-recovered — re-run `linix sync` to reconcile.",
                    self.path,
                    e,
                    if moved {
                        format!("It has been moved to {:?} for inspection.", backup)
                    } else {
                        "It could not be moved aside; it will be overwritten on the next write."
                            .to_string()
                    },
                );
            }
        }
        Ok(())
    }

    /// Atomic because a torn write of the WAL loses the record of an in-flight action.
    pub fn flush(&self) -> Result<()> {
        trace!("flushing WAL");

        let data = serde_json::to_string_pretty(&self.entries)
            .map_err(|e| Error::Other(format!("Failed to serialize Journal: {}", e)))?;

        atomic_write(&self.path, &data)
            .map_err(|e| Error::Persist(format!("Atomic write of WAL Journal failed: {}", e)))
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
            staged_properties: HashMap::new(),
        };

        self.entries.insert(id.clone(), entry);
        self.flush()?;

        debug!("Operation {} marked as InProgress in WAL.", id);
        Ok(id)
    }

    pub fn record_success(&mut self, id: &str, properties: HashMap<String, String>) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Completed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            entry.staged_properties = properties;
            self.flush()?;
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
            self.flush()?;
            warn!("Operation {} recorded as Failed in WAL: {}", id, err);
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

    /// InProgress and Failed entries are NEVER purged until they are resolved.
    pub fn cleanup_expired_logs(&mut self, days_threshold: i64) -> Result<()> {
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
            self.flush()?;
        }

        Ok(())
    }

    pub fn cleanup(&mut self) -> Result<()> {
        debug!("journal maintenance");

        // An InProgress entry older than this is read as a crashed process, not a slow one:
        // the wrong call either abandons a live install or waits forever on a dead one.
        let stale_limit = Utc::now() - ChronoDuration::hours(4);
        let stale_ts = stale_limit.timestamp();

        for entry in self.entries.values_mut() {
            if entry.status == ActionStatus::InProgress && entry.started_at_unix < stale_ts {
                debug!("Marking stale task {} as Abandoned.", entry.id);
                entry.status = ActionStatus::Abandoned;
                entry.finished_at_unix = Some(Utc::now().timestamp());
            }
        }

        self.cleanup_expired_logs(7)?;

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
