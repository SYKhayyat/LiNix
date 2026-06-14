use crate::core::{Result, Error, PackageSpec};
use crate::utils::file::atomic_write;
use serde::{Deserialize, Serialize};
use std::path::{PathBuf};
use std::collections::HashMap;
use chrono::{Utc, Duration as ChronoDuration};
use uuid::Uuid;
use tracing::{info, debug, warn, trace};

/// Represents the lifecycle stage of a specific system modification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionStatus {
    /// The action has been planned but not yet started.
    Pending,
    /// The backend is currently executing this modification.
    InProgress,
    /// The modification was successful.
    Completed,
    /// The modification failed and may require manual intervention or heal.
    Failed,
    /// The modification was interrupted by a crash and is no longer being tracked.
    Abandoned,
}

/// Represents the specific intent recorded in the WAL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JournalAction {
    /// Request to install or upgrade a package.
    Install(PackageSpec),
    /// Request to remove a package from the system.
    Remove { 
        name: String, 
        backend: String 
    },
}

/// A deterministic record of a single system modification.
/// 
/// This structure is the source of truth for the 'linix heal' command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Globally unique identifier for this specific operation.
    pub id: String,
    /// The action being performed (Install/Remove).
    pub action: JournalAction,
    /// Current status of the operation.
    pub status: ActionStatus,
    /// Unix timestamp when the operation was first recorded.
    pub started_at_unix: i64,
    /// Unix timestamp when the operation reached a terminal state (Completed/Failed).
    pub finished_at_unix: Option<i64>,
    /// If Failed, contains the error message returned by the backend.
    pub error: Option<String>,
    /// Metadata intended to be merged into the StateRegistry upon success.
    pub staged_properties: HashMap<String, String>,
}

/// The Mission-Critical Write-Ahead Log (WAL).
/// 
/// The Journal ensures LiNix can recover from power failures, OS crashes, 
/// or process kills mid-transaction. Before any backend is invoked, an 
/// 'InProgress' entry is flushed to disk.
pub struct Journal {
    /// Path to the journal.json file.
    path: PathBuf,
    /// In-memory cache of the journal entries.
    pub entries: HashMap<String, JournalEntry>,
}

impl Journal {
    /// Initializes the Journal by loading existing records from the data directory.
    pub fn new() -> Result<Self> {
        let path = crate::utils::safe_data_dir().join("journal.json");
        debug!("Journal: Initializing mission-critical WAL at {:?}", path);
        
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

    /// Synchronously loads the journal from disk.
    /// Note: Usually called during Kernel initialization.
    fn load_sync(&mut self) -> Result<()> {
        let data = std::fs::read_to_string(&self.path).map_err(|e| {
            Error::Io(format!("Failed to read WAL Journal at {:?}: {}", self.path, e))
        })?;

        if data.trim().is_empty() {
            return Ok(());
        }

        self.entries = serde_json::from_str(&data).map_err(|e| {
            Error::Other(format!("WAL Journal is corrupted and cannot be parsed: {}", e))
        })?;

        debug!("Journal: Successfully loaded {} historical log entries.", self.entries.len());
        Ok(())
    }

    /// Atomically flushes the current state of all entries to disk.
    /// This prevents log corruption during unexpected shutdowns.
    pub fn flush(&self) -> Result<()> {
        trace!("Journal: Initiating atomic WAL flush.");
        
        let data = serde_json::to_string_pretty(&self.entries).map_err(|e| {
            Error::Other(format!("Failed to serialize Journal: {}", e))
        })?;

        atomic_write(&self.path, &data).map_err(|e| {
            Error::Persist(format!("Atomic write of WAL Journal failed: {}", e))
        })
    }

    /// Generates a unique ID for a journal entry.
    fn generate_id(backend: &str, package: &str) -> String {
        format!("{}:{}:{}", backend, package, Uuid::new_v4().simple())
    }

    /// Records the intent to start a modification.
    /// This MUST be called and flushed before calling any backend commands.
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

    /// Marks an operation as successfully completed and stores result metadata.
    pub fn record_success(&mut self, id: &str, properties: HashMap<String, String>) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Completed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            entry.staged_properties = properties;
            self.flush()?;
            trace!("Journal: Operation {} marked as Completed.", id);
        } else {
            warn!("Journal: Attempted to mark unknown operation {} as successful.", id);
        }
        Ok(())
    }

    /// Marks an operation as failed with an error record.
    pub fn record_failure(&mut self, id: &str, err: &str) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Failed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            entry.error = Some(err.to_string());
            self.flush()?;
            warn!("Journal: Operation {} recorded as Failed in WAL: {}", id, err);
        } else {
            warn!("Journal: Attempted to record failure for unknown operation {}.", id);
        }
        Ok(())
    }

    /// Retrieves all entries that did not reach a 'Completed' status.
    /// Used by the 'linix heal' system.
    pub fn get_incomplete_actions(&self) -> Vec<JournalEntry> {
        self.entries.values()
            .filter(|e| e.status == ActionStatus::InProgress || e.status == ActionStatus::Failed)
            .cloned()
            .collect()
    }

    /// Checks if the WAL contains any actions currently in progress.
    /// If true, LiNix will prompt the user to run 'heal'.
    pub fn needs_recovery(&self) -> bool {
        self.entries.values().any(|e| e.status == ActionStatus::InProgress)
    }

    /// Bug Fix 10: Automatic cleanup of historical logs.
    /// 
    /// Removes Completed or Abandoned entries older than the threshold.
    /// InProgress and Failed entries are NEVER purged until they are resolved.
    pub fn cleanup_expired_logs(&mut self, days_threshold: i64) -> Result<()> {
        let cutoff = Utc::now() - ChronoDuration::days(days_threshold);
        let cutoff_ts = cutoff.timestamp();

        let initial_count = self.entries.len();

        self.entries.retain(|id, entry| {
            let is_terminal = entry.status == ActionStatus::Completed 
                           || entry.status == ActionStatus::Abandoned;
            
            if is_terminal {
                // Determine the relevant timestamp for age check
                let terminal_time = entry.finished_at_unix.unwrap_or(entry.started_at_unix);
                if terminal_time < cutoff_ts {
                    trace!("Journal: Pruning expired log record: {}", id);
                    return false; // Evict from map
                }
            }
            true // Keep in map
        });

        let purged = initial_count - self.entries.len();
        if purged > 0 {
            info!("Journal: Maintenance complete. Purged {} historical records older than {} days.", purged, days_threshold);
            self.flush()?;
        }

        Ok(())
    }

    /// Unified Maintenance Entry Point.
    /// Resolves compiler error: "no method named cleanup found for struct tokio::sync::MutexGuard<'_, Journal>"
    pub fn cleanup(&mut self) -> Result<()> {
        debug!("Journal: Commencing routine maintenance.");

        // 1. Transition stale 'InProgress' tasks to 'Abandoned'
        // If a task started more than 4 hours ago and is still in progress, 
        // we assume the process that created it crashed.
        let stale_limit = Utc::now() - ChronoDuration::hours(4);
        let stale_ts = stale_limit.timestamp();

        for entry in self.entries.values_mut() {
            if entry.status == ActionStatus::InProgress && entry.started_at_unix < stale_ts {
                debug!("Journal: Marking stale task {} as Abandoned.", entry.id);
                entry.status = ActionStatus::Abandoned;
                entry.finished_at_unix = Some(Utc::now().timestamp());
            }
        }

        // 2. Perform Bug Fix 10 pruning (7-day retention)
        self.cleanup_expired_logs(7)?;

        // 3. Maintenance: If the journal is now empty, delete the file to save disk space
        if self.entries.is_empty() && self.path.exists() {
            trace!("Journal: WAL is empty. Removing journal file.");
            let _ = std::fs::remove_file(&self.path);
        }

        Ok(())
    }
}