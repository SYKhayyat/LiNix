use crate::core::{CommandExecutor, Result, Error};
use async_trait::async_trait;
use chrono::Local;
use std::path::Path;
use std::process::Command as StdCommand;
use tracing::{info, warn, debug};

/// Represents a system-level restorable state in the Snapshot Gallery.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub timestamp: String,
    pub description: String,
    pub backend: String,
}

/// Abstract interface for system snapshots.
/// Hardened for Version 3.5.0 with Cross-Platform support.
/// FIX #21: Added ZFS snapshot provider.
#[async_trait]
pub trait SnapshotProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn is_available(&self) -> bool;
    
    /// Creates a new system snapshot with a given label.
    async fn create(&self, label: &str) -> Result<Snapshot>;
    
    /// Lists all snapshots currently managed by this provider.
    async fn list(&self) -> Result<Vec<Snapshot>>;
    
    /// Deletes a specific snapshot by its unique ID.
    async fn delete(&self, id: &str) -> Result<()>;
    
    /// Restores from a specific snapshot.
    async fn restore(&self, id: &str) -> Result<()> {
        Err(Error::Snapshot(format!("Restore not implemented for {}", self.name())))
    }
}

/// BTRFS Implementation using subvolume snapshots.
pub struct BtrfsProvider {
    pub executor: CommandExecutor,
}

#[async_trait]
impl SnapshotProvider for BtrfsProvider {
    fn name(&self) -> &str { "btrfs" }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && 
        self.executor.command_exists_sync("btrfs") && 
        Path::new("/.snapshots").exists()
    }

    async fn create(&self, label: &str) -> Result<Snapshot> {
        let ts = Local::now().format("%Y%m%d%H%M%S").to_string();
        let id = format!("linix_pre_{}_{}", label, ts);
        let path = format!("/.snapshots/{}", id);
        
        info!("BTRFS: Creating read-only snapshot of / at {}", path);
        self.executor.run("btrfs", &["subvolume", "snapshot", "-r", "/", &path], true).await?;
        
        Ok(Snapshot {
            id,
            timestamp: Local::now().to_rfc3339(),
            description: label.to_string(),
            backend: "btrfs".into(),
        })
    }

    async fn list(&self) -> Result<Vec<Snapshot>> {
        let out = self.executor.run_output("btrfs", &["subvolume", "list", "/"], false).await?;
        Ok(out.lines()
            .filter(|l| l.contains("linix_pre_"))
            .filter_map(|l| {
                let path_part = l.split("path ").last()?;
                let id = path_part.split('/').last()?;
                Some(Snapshot {
                    id: id.to_string(),
                    timestamp: "UTC".into(),
                    description: "LiNix System State".into(),
                    backend: "btrfs".into(),
                })
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let path = format!("/.snapshots/{}", id);
        self.executor.run("btrfs", &["subvolume", "delete", &path], true).await?;
        Ok(())
    }
}

/// FIX #21: ZFS Implementation using zfs snapshot command.
pub struct ZfsProvider {
    pub executor: CommandExecutor,
    pub pool: Option<String>,
}

impl ZfsProvider {
    pub fn new(executor: CommandExecutor) -> Self {
        // Try to auto-detect the root dataset
        let pool = Self::detect_root_dataset();
        Self { executor, pool }
    }
    
    /// Detects the root ZFS dataset (e.g., "rpool/ROOT/ubuntu").
    fn detect_root_dataset() -> Option<String> {
        // Try to get the dataset mounted at /
        let output = StdCommand::new("zfs")
            .args(&["list", "-H", "-o", "name", "-r", "/"])
            .output()
            .ok()?;
        
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
        
        // Fallback: try common pool names
        let common_pools = ["rpool", "zroot", "tank", "bpool"];
        for pool in common_pools {
            let check = StdCommand::new("zfs")
                .args(&["list", "-H", "-o", "name", pool])
                .output();
            if let Ok(out) = check {
                if out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty() {
                    return Some(pool.to_string());
                }
            }
        }
        
        None
    }
    
    /// Gets the full dataset path for a snapshot.
    fn get_snapshot_name(&self, label: &str) -> String {
        let ts = Local::now().format("%Y%m%d%H%M%S").to_string();
        let dataset = self.pool.as_deref().unwrap_or("rpool");
        format!("{}@linix_{}_{}", dataset, label, ts)
    }
}

#[async_trait]
impl SnapshotProvider for ZfsProvider {
    fn name(&self) -> &str { "zfs" }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") || cfg!(target_os = "freebsd") || cfg!(target_os = "illumos") &&
        self.executor.command_exists_sync("zfs") &&
        self.pool.is_some()
    }

    async fn create(&self, label: &str) -> Result<Snapshot> {
        let snapshot_name = self.get_snapshot_name(label);
        let id = snapshot_name.clone();
        
        info!("ZFS: Creating snapshot: {}", snapshot_name);
        
        // Create recursive snapshot for the entire dataset
        self.executor.run("zfs", &["snapshot", "-r", &snapshot_name], true).await?;
        
        // Also create a hold to prevent accidental deletion (optional)
        let _ = self.executor.run("zfs", &["hold", "linix-protect", &snapshot_name], true).await;
        
        Ok(Snapshot {
            id,
            timestamp: Local::now().to_rfc3339(),
            description: label.to_string(),
            backend: "zfs".into(),
        })
    }

    async fn list(&self) -> Result<Vec<Snapshot>> {
        let dataset = self.pool.as_deref().unwrap_or("rpool");
        let out = self.executor.run_output("zfs", &["list", "-H", "-r", "-t", "snapshot", "-o", "name,creation", dataset], false).await?;
        
        let mut snapshots = Vec::new();
        for line in out.lines() {
            if line.contains("linix_") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(name) = parts.first() {
                    let id = name.to_string();
                    let timestamp = parts.get(1).map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string());
                    
                    // Extract description from snapshot name (everything after @)
                    let description = name.split('@').last()
                        .map(|s| s.replace("linix_", ""))
                        .unwrap_or_else(|| "ZFS Snapshot".to_string());
                    
                    snapshots.push(Snapshot {
                        id,
                        timestamp,
                        description,
                        backend: "zfs".into(),
                    });
                }
            }
        }
        
        Ok(snapshots)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        info!("ZFS: Deleting snapshot: {}", id);
        
        // Release the hold if it exists
        let _ = self.executor.run("zfs", &["release", "linix-protect", id], true).await;
        
        // Destroy the snapshot
        self.executor.run("zfs", &["destroy", "-r", id], true).await?;
        Ok(())
    }
    
    async fn restore(&self, id: &str) -> Result<()> {
        info!("ZFS: Rolling back to snapshot: {}", id);
        
        // Rollback to the specified snapshot
        // This will destroy any snapshots and bookmarks after this one
        self.executor.run("zfs", &["rollback", "-r", id], true).await?;
        
        println!("\nSUCCESS: System rolled back to snapshot {}. Please reboot.", id);
        Ok(())
    }
}

/// Timeshift Implementation for standard Linux distributions.
pub struct TimeshiftProvider {
    pub executor: CommandExecutor,
}

#[async_trait]
impl SnapshotProvider for TimeshiftProvider {
    fn name(&self) -> &str { "timeshift" }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && self.executor.command_exists_sync("timeshift")
    }

    async fn create(&self, label: &str) -> Result<Snapshot> {
        info!("Timeshift: Initializing safety snapshot...");
        let out = self.executor.run_output("timeshift", &["--create", "--comments", label, "--tags", "D"], true).await?;
        
        let id = out.lines()
            .find(|l| l.contains("Snapshot: "))
            .map(|l| l.replace("Snapshot: ", "").trim().to_string())
            .unwrap_or_else(|| Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());

        Ok(Snapshot {
            id,
            timestamp: Local::now().to_rfc3339(),
            description: label.to_string(),
            backend: "timeshift".into(),
        })
    }

    async fn list(&self) -> Result<Vec<Snapshot>> {
        let out = self.executor.run_output("timeshift", &["--list"], true).await?;
        let mut results = Vec::new();

        for line in out.lines() {
            if line.contains(">") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(id) = parts.get(2) {
                    results.push(Snapshot {
                        id: id.to_string(),
                        timestamp: parts.get(1).unwrap_or(&"unknown").to_string(),
                        description: parts.get(4..).unwrap_or(&[]).join(" "),
                        backend: "timeshift".into(),
                    });
                }
            }
        }
        Ok(results)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        self.executor.run("timeshift", &["--delete", "--snapshot", id], true).await?;
        Ok(())
    }
}

/// Windows System Restore Point Provider.
pub struct WindowsRestoreProvider {
    pub executor: CommandExecutor,
}

#[async_trait]
impl SnapshotProvider for WindowsRestoreProvider {
    fn name(&self) -> &str { "windows_restore" }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "windows")
    }

    async fn create(&self, label: &str) -> Result<Snapshot> {
        info!("Windows: Creating System Restore Point: {}...", label);
        
        let ps_cmd = format!(
            "Checkpoint-Computer -Description 'LiNix: {}' -RestorePointType 'APPLICATION_INSTALL'", 
            label
        );
        
        self.executor.run("powershell", &["-Command", &ps_cmd], true).await?;

        Ok(Snapshot {
            id: Local::now().timestamp().to_string(),
            timestamp: Local::now().to_rfc3339(),
            description: label.to_string(),
            backend: "windows_restore".into(),
        })
    }

    async fn list(&self) -> Result<Vec<Snapshot>> {
        let ps_cmd = "Get-ComputerRestorePoint | Select-Object SequenceNumber, CreationTime, Description | ConvertTo-Json";
        let out = self.executor.run_output("powershell", &["-Command", ps_cmd], false).await?;
        
        if out.is_empty() || out == "null" { return Ok(vec![]); }

        let json: serde_json::Value = serde_json::from_str(&out).map_err(|e| Error::Other(e.to_string()))?;
        let mut snapshots = Vec::new();

        if let Some(items) = json.as_array() {
            for item in items {
                snapshots.push(Snapshot {
                    id: item["SequenceNumber"].to_string(),
                    timestamp: item["CreationTime"].as_str().unwrap_or("unknown").to_string(),
                    description: item["Description"].as_str().unwrap_or("").to_string(),
                    backend: "windows_restore".into(),
                });
            }
        }

        Ok(snapshots)
    }

    async fn delete(&self, _id: &str) -> Result<()> {
        // Windows does not allow deleting specific restore points easily via CLI
        Ok(())
    }
}

/// The Snapshot Factory: Detects and manages system-level atomic recovery points.
/// FIX #21: Added ZFS provider to the detection chain.
pub struct SnapshotManager {
    provider: Option<Box<dyn SnapshotProvider>>,
}

impl SnapshotManager {
    pub async fn new(executor: CommandExecutor) -> Self {
        let providers: Vec<Box<dyn SnapshotProvider>> = vec![
            Box::new(ZfsProvider::new(executor.clone())),  // FIX #21: ZFS first (most efficient)
            Box::new(BtrfsProvider { executor: executor.clone() }),
            Box::new(TimeshiftProvider { executor: executor.clone() }),
            Box::new(WindowsRestoreProvider { executor: executor.clone() }),
        ];

        let mut active_provider = None;
        for p in providers {
            if p.is_available().await {
                debug!("SnapshotManager: Selected {} as active provider.", p.name());
                active_provider = Some(p);
                break;
            }
        }

        Self { provider: active_provider }
    }

    pub async fn auto_snapshot(&self, label: &str) -> Result<Option<Snapshot>> {
        if let Some(ref p) = self.provider {
            info!("Safety: Generating system-level rollback point via {}...", p.name());
            Ok(Some(p.create(label).await?))
        } else {
            warn!("Safety: No snapshot provider available. Transactions will proceed without a system rollback point.");
            Ok(None)
        }
    }

    pub async fn list_snapshots(&self) -> Result<Vec<Snapshot>> {
        if let Some(ref p) = self.provider {
            p.list().await
        } else {
            Ok(vec![])
        }
    }
    
    pub async fn restore_snapshot(&self, id: &str) -> Result<()> {
        if let Some(ref p) = self.provider {
            p.restore(id).await
        } else {
            Err(Error::Snapshot("No snapshot provider available for restore".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_zfs_detection() {
        let executor = CommandExecutor::new(true, false);
        let provider = ZfsProvider::new(executor);
        
        // Just test that the provider can be created
        assert_eq!(provider.name(), "zfs");
    }
    
    #[test]
    fn test_zfs_snapshot_name_format() {
        let executor = CommandExecutor::new(true, false);
        let provider = ZfsProvider::new(executor);
        
        let snapshot_name = provider.get_snapshot_name("test");
        assert!(snapshot_name.contains("@linix_test_"));
        assert!(snapshot_name.len() > 20);
    }
    
    #[test]
    fn test_snapshot_struct() {
        let snapshot = Snapshot {
            id: "test-123".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            description: "Test snapshot".to_string(),
            backend: "zfs".to_string(),
        };
        
        assert_eq!(snapshot.id, "test-123");
        assert_eq!(snapshot.backend, "zfs");
    }
}