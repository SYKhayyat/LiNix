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
    /// A crash `cleanup` has given up on: no longer healable.
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
        debug!("Journal: Initializing WAL at {:?}", path);

        let mut journal = Self {
            path,
            entries: HashMap::new(),
        };

        if journal.path.exists() {
            journal.load_sync()?;
        } else {
            trace!("Journal: No existing WAL found, starting fresh.");
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

        self.entries = serde_json::from_str(&data).map_err(|e| {
            Error::Other(format!(
                "WAL Journal is corrupted and cannot be parsed: {}",
                e
            ))
        })?;

        debug!(
            "Journal: Successfully loaded {} historical log entries.",
            self.entries.len()
        );
        Ok(())
    }

    /// Atomic because a torn write of the WAL loses the record of an in-flight action.
    pub fn flush(&self) -> Result<()> {
        trace!("Journal: Initiating atomic WAL flush.");

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

        debug!("Journal: Operation {} marked as InProgress in WAL.", id);
        Ok(id)
    }

    pub fn record_success(&mut self, id: &str, properties: HashMap<String, String>) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Completed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            entry.staged_properties = properties;
            self.flush()?;
            trace!("Journal: Operation {} marked as Completed.", id);
        } else {
            warn!(
                "Journal: Attempted to mark unknown operation {} as successful.",
                id
            );
        }
        Ok(())
    }

    pub fn record_failure(&mut self, id: &str, err: &str) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Failed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            entry.error = Some(err.to_string());
            self.flush()?;
            warn!(
                "Journal: Operation {} recorded as Failed in WAL: {}",
                id, err
            );
        } else {
            warn!(
                "Journal: Attempted to record failure for unknown operation {}.",
                id
            );
        }
        Ok(())
    }

    /// `InProgress` and `Failed` only — what `heal` can still act on.
    ///
    /// Deliberately not "everything that isn't Completed": `Pending` never touched the
    /// system, and `Abandoned` is a crash `cleanup` has already given up on (4h). That
    /// means a crash left unhealed for 4 hours stops being healable, because `cleanup`
    /// reclassifies it out of this set.
    pub fn get_incomplete_actions(&self) -> Vec<JournalEntry> {
        self.entries
            .values()
            .filter(|e| e.status == ActionStatus::InProgress || e.status == ActionStatus::Failed)
            .cloned()
            .collect()
    }

    /// True makes `sync` run `heal` on its own, without asking.
    pub fn needs_recovery(&self) -> bool {
        self.entries
            .values()
            .any(|e| e.status == ActionStatus::InProgress)
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
                    trace!("Journal: Pruning expired log record: {}", id);
                    return false;
                }
            }
            true
        });

        let purged = initial_count - self.entries.len();
        if purged > 0 {
            info!(
                "Journal: Maintenance complete. Purged {} historical records older than {} days.",
                purged, days_threshold
            );
            self.flush()?;
        }

        Ok(())
    }

    pub fn cleanup(&mut self) -> Result<()> {
        debug!("Journal: Commencing routine maintenance.");

        // An InProgress entry older than this is read as a crashed process, not a slow one:
        // the wrong call either abandons a live install or waits forever on a dead one.
        let stale_limit = Utc::now() - ChronoDuration::hours(4);
        let stale_ts = stale_limit.timestamp();

        for entry in self.entries.values_mut() {
            if entry.status == ActionStatus::InProgress && entry.started_at_unix < stale_ts {
                debug!("Journal: Marking stale task {} as Abandoned.", entry.id);
                entry.status = ActionStatus::Abandoned;
                entry.finished_at_unix = Some(Utc::now().timestamp());
            }
        }

        self.cleanup_expired_logs(7)?;

        if self.entries.is_empty() && self.path.exists() {
            trace!("Journal: WAL is empty. Removing journal file.");
            let _ = std::fs::remove_file(&self.path);
        }

        Ok(())
    }
}
