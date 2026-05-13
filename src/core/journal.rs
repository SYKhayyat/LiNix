use crate::core::{Result, Error, PackageSpec, ActionStatus};
use crate::utils::file::atomic_write;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// Represents the full intent of a system modification.
/// Storing the full Spec ensures that recovery (healing) preserves
/// all custom options like quotas, sandboxing, and checksums.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JournalAction {
    Install(PackageSpec),
    Remove { name: String, backend: String },
}

/// A deterministic record of a single system modification.
/// Hardened for Version 3.5.0 to support perfect recovery and staged state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub action: JournalAction,
    pub status: ActionStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// Metadata intended to be merged into the StateRegistry upon successful completion.
    pub staged_properties: HashMap<String, String>,
}

/// The Mission-Critical Write-Ahead Log (WAL).
/// Ensures LiNix can recover from power failures or crashes mid-transaction.
/// 
/// In Version 3.5.0, the Journal acts as a "Staging Area." 
/// Transactions write to the Journal first. Only after the Journal confirms
/// success is the main StateRegistry (registry.json) updated.
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

    /// Pre-registers a start of an action. 
    /// Stores the full action enum for non-destructive recovery.
    pub fn record_start(&mut self, action: JournalAction) -> Result<String> {
        let (b_name, p_name) = match &action {
            JournalAction::Install(s) => (&s.backend, &s.name),
            JournalAction::Remove { name, backend } => (backend, name),
        };

        let id = format!("{}:{}:{:?}", b_name, p_name, Utc::now().timestamp_nanos());
        
        let entry = JournalEntry {
            id: id.clone(),
            action,
            status: ActionStatus::InProgress,
            started_at: Utc::now(),
            finished_at: None,
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
            entry.finished_at = Some(Utc::now());
            entry.staged_properties = properties;
            self.flush()?;
        }
        Ok(())
    }

    /// Marks an action as failed with an error message.
    pub fn record_failure(&mut self, id: &str, err: &str) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Failed;
            entry.finished_at = Some(Utc::now());
            entry.error = Some(err.to_string());
            self.flush()?;
        }
        Ok(())
    }

    /// Returns a list of operations that were left in an inconsistent state.
    /// Unlike v3.4.0, this returns the full Action enum for perfect re-execution.
    pub fn get_incomplete_actions(&self) -> Vec<JournalEntry> {
        self.entries.values()
            .filter(|e| e.status == ActionStatus::InProgress || e.status == ActionStatus::Failed)
            .cloned()
            .collect()
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

    /// Checks if a specific package has a pending or in-progress action.
    pub fn is_pending(&self, backend: &str, package: &str) -> bool {
        self.entries.values().any(|e| {
            let matches = match &e.action {
                JournalAction::Install(s) => s.backend == backend && s.name == package,
                JournalAction::Remove { name, backend: b } => b == backend && name == package,
            };
            matches && (e.status == ActionStatus::InProgress || e.status == ActionStatus::Pending)
        })
    }
}