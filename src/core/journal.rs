use crate::core::{Result, Error, PackageSpec};
use crate::utils::file::atomic_write;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::collections::HashMap;
use chrono::Utc;
use uuid::Uuid;
use tracing::{debug, warn};

/// Represents the state of a journal entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// Represents the full intent of a system modification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JournalAction {
    Install(PackageSpec),
    Remove { name: String, backend: String },
}

/// A deterministic record of a single system modification.
/// Hardened for Version 3.5.0: Uses i64 timestamps to ensure stable serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub action: JournalAction,
    pub status: ActionStatus,
    pub started_at_unix: i64,
    pub finished_at_unix: Option<i64>,
    pub error: Option<String>,
    /// Metadata intended to be merged into the StateRegistry upon successful completion.
    pub staged_properties: HashMap<String, String>,
}

/// The Mission-Critical Write-Ahead Log (WAL).
/// Ensures LiNix can recover from power failures or crashes mid-transaction.
pub struct Journal {
    path: PathBuf,
    pub entries: HashMap<String, JournalEntry>,
}

impl Journal {
    /// Initializes or loads an existing journal from the data directory.
    pub fn new() -> Result<Self> {
        let path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("journal.json");
        
        let mut journal = Self {
            path,
            entries: HashMap::new(),
        };

        if journal.path.exists() {
            journal.load()?;
        }

        Ok(journal)
    }

    /// Loads journal state from disk.
    fn load(&mut self) -> Result<()> {
        let data = std::fs::read_to_string(&self.path)?;
        if data.trim().is_empty() {
            return Ok(());
        }
        self.entries = serde_json::from_str(&data).map_err(|e| {
            Error::Other(format!("Corrupted WAL Journal: {}", e))
        })?;
        Ok(())
    }

    /// Synchronizes the current in-memory state to disk atomically.
    pub fn flush(&self) -> Result<()> {
        let data = serde_json::to_string_pretty(&self.entries).map_err(|e| {
            Error::Other(e.to_string())
        })?;
        atomic_write(&self.path, &data)
    }

    /// Generates a unique ID for a journal entry using UUID v4.
    fn generate_id(backend: &str, package: &str) -> String {
        let uuid = Uuid::new_v4();
        format!("{}:{}:{}", backend, package, uuid.simple())
    }

    /// Pre-registers a start of an action. 
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
        Ok(id)
    }

    /// Marks an action as successful and stores properties discovered during the run.
    pub fn record_success(&mut self, id: &str, properties: HashMap<String, String>) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Completed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            entry.staged_properties = properties;
            self.flush()?;
        }
        Ok(())
    }

    /// Marks an action as failed with an error message.
    pub fn record_failure(&mut self, id: &str, err: &str) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Failed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            entry.error = Some(err.to_string());
            self.flush()?;
        }
        Ok(())
    }

    /// Returns a list of operations that were left in an inconsistent state.
    pub fn get_incomplete_actions(&self) -> Vec<JournalEntry> {
        self.entries.values()
            .filter(|e| e.status == ActionStatus::InProgress || e.status == ActionStatus::Failed)
            .cloned()
            .collect()
    }

    /// Returns true if any operations are currently in an inconsistent "InProgress" state.
    pub fn needs_recovery(&self) -> bool {
        self.entries.values().any(|e| e.status == ActionStatus::InProgress)
    }

    /// Purges completed entries from the journal to keep it lean.
    pub fn cleanup(&mut self) -> Result<()> {
        self.entries.retain(|_, e| e.status != ActionStatus::Completed);
        if self.entries.is_empty() {
            if self.path.exists() {
                let _ = std::fs::remove_file(&self.path);
            }
        } else {
            self.flush()?;
        }
        Ok(())
    }
}