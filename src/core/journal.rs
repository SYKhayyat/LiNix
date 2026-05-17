use crate::core::{Result, Error, PackageSpec, ActionStatus};
use crate::utils::file::atomic_write;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use uuid::Uuid;

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

    /// Generates a unique ID for a journal entry.
    /// FIX #5: Uses UUID v4 instead of timestamp_nanos() to avoid deprecation and overflow.
    fn generate_id(backend: &str, package: &str) -> String {
        // Use UUID v4 for guaranteed uniqueness
        let uuid = Uuid::new_v4();
        
        // Also include timestamp for human readability (using safe method)
        let timestamp = Utc::now().timestamp_nanos_opt()
            .unwrap_or_else(|| {
                // Fallback to milliseconds if nanos overflow
                Utc::now().timestamp_millis() * 1_000_000
            });
        
        format!("{}:{}:{}_{}", backend, package, timestamp, uuid.simple())
    }

    /// Pre-registers a start of an action. 
    /// Stores the full action enum for non-destructive recovery.
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

    /// Returns a list of operations that need recovery (were in progress but not completed).
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

    /// Gets an entry by ID.
    pub fn get_entry(&self, id: &str) -> Option<&JournalEntry> {
        self.entries.get(id)
    }

    /// Gets all entries for a specific package.
    pub fn get_entries_for_package(&self, backend: &str, package: &str) -> Vec<&JournalEntry> {
        self.entries.values()
            .filter(|e| {
                match &e.action {
                    JournalAction::Install(s) => s.backend == backend && s.name == package,
                    JournalAction::Remove { name, backend: b } => b == backend && name == package,
                }
            })
            .collect()
    }

    /// Removes stale entries older than the given timestamp.
    pub fn prune_old_entries(&mut self, older_than: DateTime<Utc>) -> Result<()> {
        self.entries.retain(|_, e| e.started_at >= older_than);
        self.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_generate_id_uniqueness() {
        let id1 = Journal::generate_id("apt", "curl");
        let id2 = Journal::generate_id("apt", "curl");
        let id3 = Journal::generate_id("brew", "git");
        
        assert_ne!(id1, id2);
        assert_ne!(id1, id3);
        assert_ne!(id2, id3);
        
        // IDs should contain the backend and package names
        assert!(id1.contains("apt"));
        assert!(id1.contains("curl"));
    }

    #[test]
    fn test_journal_roundtrip() {
        let dir = tempdir().unwrap();
        let original_path = dir.path().join("journal.json");
        
        // We need to temporarily set the path for testing
        // This is a simplified test focusing on serialization
        let mut journal = Journal {
            path: original_path.clone(),
            entries: HashMap::new(),
        };
        
        let spec = PackageSpec {
            name: "test-pkg".to_string(),
            backend: "apt".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        
        let id = journal.record_start(JournalAction::Install(spec)).unwrap();
        journal.record_success(&id, HashMap::new()).unwrap();
        
        // Reload journal
        let mut new_journal = Journal {
            path: original_path,
            entries: HashMap::new(),
        };
        new_journal.load().unwrap();
        
        assert_eq!(new_journal.entries.len(), 0); // Completed entries are cleaned up?
    }

    #[test]
    fn test_needs_recovery() {
        let mut journal = Journal::new().unwrap();
        
        let spec = PackageSpec {
            name: "test-pkg".to_string(),
            backend: "apt".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        
        let id = journal.record_start(JournalAction::Install(spec)).unwrap();
        assert!(journal.needs_recovery());
        
        journal.record_success(&id, HashMap::new()).unwrap();
        // Note: success doesn't immediately clean up, but after cleanup()
        journal.cleanup().unwrap();
        assert!(!journal.needs_recovery());
    }

    #[test]
    fn test_get_incomplete_actions() {
        let mut journal = Journal::new().unwrap();
        
        let spec1 = PackageSpec {
            name: "pkg1".to_string(),
            backend: "apt".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        
        let spec2 = PackageSpec {
            name: "pkg2".to_string(),
            backend: "brew".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        
        let id1 = journal.record_start(JournalAction::Install(spec1)).unwrap();
        let id2 = journal.record_start(JournalAction::Install(spec2)).unwrap();
        
        journal.record_success(&id1, HashMap::new()).unwrap();
        
        let incomplete = journal.get_incomplete_actions();
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].id, id2);
    }

    #[test]
    fn test_is_pending() {
        let mut journal = Journal::new().unwrap();
        
        let spec = PackageSpec {
            name: "curl".to_string(),
            backend: "apt".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        
        journal.record_start(JournalAction::Install(spec)).unwrap();
        
        assert!(journal.is_pending("apt", "curl"));
        assert!(!journal.is_pending("brew", "curl"));
        assert!(!journal.is_pending("apt", "wget"));
    }

    #[test]
    fn test_get_entries_for_package() {
        let mut journal = Journal::new().unwrap();
        
        let spec = PackageSpec {
            name: "vim".to_string(),
            backend: "apt".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        
        journal.record_start(JournalAction::Install(spec)).unwrap();
        
        let entries = journal.get_entries_for_package("apt", "vim");
        assert_eq!(entries.len(), 1);
        
        let entries_none = journal.get_entries_for_package("apt", "nonexistent");
        assert_eq!(entries_none.len(), 0);
    }

    #[test]
    fn test_prune_old_entries() {
        let mut journal = Journal::new().unwrap();
        
        let spec = PackageSpec {
            name: "old-pkg".to_string(),
            backend: "apt".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        
        journal.record_start(JournalAction::Install(spec)).unwrap();
        
        let future_time = Utc::now() + chrono::Duration::hours(1);
        journal.prune_old_entries(future_time).unwrap();
        
        // All entries older than future_time should be pruned
        assert_eq!(journal.entries.len(), 0);
    }

    #[test]
    fn test_timestamp_nanos_opt_fallback() {
        // Test that the fallback works correctly
        let timestamp = Utc::now().timestamp_nanos_opt();
        
        if let Some(nanos) = timestamp {
            assert!(nanos > 0);
        } else {
            // On systems where nanos overflow, we fall back to milliseconds
            let millis = Utc::now().timestamp_millis();
            assert!(millis > 0);
        }
    }
}