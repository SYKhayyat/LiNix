use crate::config::Config;
use crate::core::{CommandExecutor, Error, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command as StdCommand;
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub timestamp: String,
    pub description: String,
    pub backend: String,
}

impl Snapshot {
    pub fn parse_time(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.timestamp)
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
    }
}

#[async_trait]
pub trait SnapshotProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn is_available(&self) -> bool;
    async fn create(&self, label: &str) -> Result<Snapshot>;
    async fn list(&self) -> Result<Vec<Snapshot>>;
    async fn delete(&self, id: &str) -> Result<()>;
    async fn restore(&self, id: &str) -> Result<()>;
}

pub struct BtrfsProvider {
    pub executor: CommandExecutor,
    pub snapshot_root: String,
}

#[async_trait]
impl SnapshotProvider for BtrfsProvider {
    fn name(&self) -> &str {
        "btrfs"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux")
            && self.executor.command_exists("btrfs").await
            && Path::new(&self.snapshot_root).exists()
    }

    async fn create(&self, label: &str) -> Result<Snapshot> {
        let ts_id = Local::now().format("%Y%m%d%H%M%S").to_string();
        let id = format!("linix_pre_{}_{}", label, ts_id);
        let path = format!("{}/{}", self.snapshot_root, id);

        info!("BTRFS: Creating read-only snapshot: {}", id);
        self.executor
            .run("btrfs", &["subvolume", "snapshot", "-r", "/", &path], true)
            .await?;

        Ok(Snapshot {
            id,
            timestamp: Utc::now().to_rfc3339(),
            description: label.to_string(),
            backend: "btrfs".into(),
        })
    }

    async fn list(&self) -> Result<Vec<Snapshot>> {
        let out = self
            .executor
            .run_output("btrfs", &["subvolume", "list", "/"], false)
            .await?;
        Ok(out
            .lines()
            .filter(|l| l.contains("linix_pre_"))
            .filter_map(|l| {
                let id = l.split('/').next_back()?.trim();
                Some(Snapshot {
                    id: id.to_string(),
                    timestamp: Utc::now().to_rfc3339(),
                    description: "BTRFS System State".into(),
                    backend: "btrfs".into(),
                })
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let path = format!("{}/{}", self.snapshot_root, id);
        self.executor
            .run("btrfs", &["subvolume", "delete", &path], true)
            .await
            .map(|_| ())
    }

    async fn restore(&self, id: &str) -> Result<()> {
        let path = format!("{}/{}", self.snapshot_root, id);
        info!("BTRFS: Rolling back to: {}", id);
        self.executor
            .run("btrfs", &["subvolume", "snapshot", &path, "/"], true)
            .await
            .map(|_| ())
    }
}

pub struct ZfsProvider {
    pub executor: CommandExecutor,
    pub dataset: String,
}

#[async_trait]
impl SnapshotProvider for ZfsProvider {
    fn name(&self) -> &str {
        "zfs"
    }
    async fn is_available(&self) -> bool {
        self.executor.command_exists("zfs").await
    }

    async fn create(&self, label: &str) -> Result<Snapshot> {
        let id = format!(
            "{}@linix_{}",
            self.dataset,
            Local::now().format("%Y%m%d_%H%M%S")
        );
        info!("ZFS: Creating recursive snapshot: {}", id);
        self.executor
            .run("zfs", &["snapshot", "-r", &id], true)
            .await?;
        Ok(Snapshot {
            id,
            timestamp: Utc::now().to_rfc3339(),
            description: label.to_string(),
            backend: "zfs".into(),
        })
    }

    async fn list(&self) -> Result<Vec<Snapshot>> {
        let out = self
            .executor
            .run_output(
                "zfs",
                &["list", "-H", "-r", "-t", "snapshot", "-o", "name"],
                false,
            )
            .await?;
        Ok(out
            .lines()
            .filter(|l| l.contains("@linix_"))
            .map(|l| Snapshot {
                id: l.trim().to_string(),
                timestamp: Utc::now().to_rfc3339(),
                description: "ZFS Snapshot".into(),
                backend: "zfs".into(),
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        self.executor
            .run("zfs", &["destroy", "-r", id], true)
            .await
            .map(|_| ())
    }

    async fn restore(&self, id: &str) -> Result<()> {
        self.executor
            .run("zfs", &["rollback", "-r", id], true)
            .await
            .map(|_| ())
    }
}

pub struct TimeshiftProvider {
    pub executor: CommandExecutor,
}

#[async_trait]
impl SnapshotProvider for TimeshiftProvider {
    fn name(&self) -> &str {
        "timeshift"
    }
    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && self.executor.command_exists("timeshift").await
    }

    async fn create(&self, label: &str) -> Result<Snapshot> {
        let out = self
            .executor
            .run_output(
                "timeshift",
                &["--create", "--comments", label, "--tags", "D"],
                true,
            )
            .await?;
        let id = out
            .lines()
            .find(|l| l.contains("Snapshot: "))
            .map(|l| l.replace("Snapshot: ", "").trim().to_string())
            .unwrap_or_else(|| Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
        Ok(Snapshot {
            id,
            timestamp: Utc::now().to_rfc3339(),
            description: label.to_string(),
            backend: "timeshift".into(),
        })
    }

    async fn list(&self) -> Result<Vec<Snapshot>> {
        let out = self
            .executor
            .run_output("timeshift", &["--list"], true)
            .await?;
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
        self.executor
            .run("timeshift", &["--delete", "--snapshot", id], true)
            .await
            .map(|_| ())
    }

    async fn restore(&self, id: &str) -> Result<()> {
        self.executor
            .run(
                "timeshift",
                &[
                    "--restore",
                    "--snapshot",
                    id,
                    "--target-device",
                    "/",
                    "--yes",
                ],
                true,
            )
            .await
            .map(|_| ())
    }
}

pub struct WindowsRestoreProvider {
    pub executor: CommandExecutor,
}

#[async_trait]
impl SnapshotProvider for WindowsRestoreProvider {
    fn name(&self) -> &str {
        "windows_restore"
    }
    async fn is_available(&self) -> bool {
        cfg!(target_os = "windows")
    }

    async fn create(&self, label: &str) -> Result<Snapshot> {
        let ps_cmd = format!(
            "Checkpoint-Computer -Description 'LiNix: {}' -RestorePointType 'APPLICATION_INSTALL'",
            label
        );
        self.executor
            .run("powershell", &["-Command", &ps_cmd], true)
            .await?;
        Ok(Snapshot {
            id: Local::now().timestamp().to_string(),
            timestamp: Utc::now().to_rfc3339(),
            description: label.to_string(),
            backend: "windows_restore".into(),
        })
    }

    async fn list(&self) -> Result<Vec<Snapshot>> {
        let ps_cmd = "Get-ComputerRestorePoint | ConvertTo-Json";
        let out = self
            .executor
            .run_output("powershell", &["-Command", ps_cmd], false)
            .await?;
        if out.is_empty() || out == "null" {
            return Ok(vec![]);
        }
        let json: serde_json::Value = serde_json::from_str(&out).map_err(Error::from)?;
        let mut list = Vec::new();
        if let Some(items) = json.as_array() {
            for item in items {
                list.push(Snapshot {
                    id: item["SequenceNumber"].to_string(),
                    timestamp: item["CreationTime"].as_str().unwrap_or("").to_string(),
                    description: item["Description"].as_str().unwrap_or("").to_string(),
                    backend: "windows_restore".into(),
                });
            }
        }
        Ok(list)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let ps_cmd = format!("Get-WmiObject -Namespace root\\default -Class SystemRestore | ForEach-Object {{ $_.DeleteStatus({}) }}", id);
        self.executor
            .run("powershell", &["-Command", &ps_cmd], true)
            .await
            .map(|_| ())
    }

    async fn restore(&self, id: &str) -> Result<()> {
        let ps_cmd = format!("Restore-Computer -RestorePoint {} -Confirm:$false", id);
        self.executor
            .run("powershell", &["-Command", &ps_cmd], true)
            .await
            .map(|_| ())
    }
}

pub struct SnapshotManager {
    provider: Option<Box<dyn SnapshotProvider>>,
}

impl SnapshotManager {
    pub fn with_provider(provider: Box<dyn SnapshotProvider>) -> Self {
        Self {
            provider: Some(provider),
        }
    }

    pub async fn new(executor: CommandExecutor, config: &Config) -> Self {
        let mut providers: Vec<Box<dyn SnapshotProvider>> = vec![
            Box::new(BtrfsProvider {
                executor: executor.duplicate(),
                snapshot_root: config.btrfs_path.clone(),
            }),
            Box::new(TimeshiftProvider {
                executor: executor.duplicate(),
            }),
            Box::new(WindowsRestoreProvider {
                executor: executor.duplicate(),
            }),
        ];

        if executor.command_exists("zfs").await {
            let dataset = config.zfs_dataset.clone().unwrap_or_else(|| {
                StdCommand::new("zfs")
                    .args(["list", "-H", "-o", "name", "-r", "/"])
                    .output()
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default()
            });
            if !dataset.is_empty() {
                providers.push(Box::new(ZfsProvider {
                    executor: executor.duplicate(),
                    dataset,
                }));
            }
        }

        let mut active = None;
        for p in providers {
            if p.is_available().await {
                active = Some(p);
                break;
            }
        }
        Self { provider: active }
    }

    pub async fn auto_snapshot(&self, label: &str) -> Result<Option<Snapshot>> {
        if let Some(ref p) = self.provider {
            Ok(Some(p.create(label).await?))
        } else {
            Ok(None)
        }
    }

    /// True when an active snapshot provider is available (so rollback is possible).
    /// Used by `canary`/`bisect`/policy `require_snapshot` to fail fast when there is no
    /// safety net rather than performing an unrecoverable change.
    pub fn has_provider(&self) -> bool {
        self.provider.is_some()
    }

    pub async fn prune_stale_snapshots(
        &self,
        max_age_days: u32,
        max_count: u32,
        is_dry_run: bool,
    ) -> Result<()> {
        let p = match &self.provider {
            Some(p) => p,
            None => return Ok(()),
        };
        let mut list = p.list().await?;
        if list.is_empty() {
            return Ok(());
        }

        list.sort_by_key(|s| s.parse_time().unwrap_or(Utc::now()));
        let mut to_delete = HashSet::new();
        let now = Utc::now();
        let age_limit = ChronoDuration::days(max_age_days as i64);

        for s in &list {
            if let Some(time) = s.parse_time() {
                if now.signed_duration_since(time) > age_limit {
                    to_delete.insert(s.id.clone());
                }
            }
        }

        let remaining: Vec<_> = list.iter().filter(|s| !to_delete.contains(&s.id)).collect();
        if remaining.len() > max_count as usize {
            let overflow = remaining.len() - max_count as usize;
            for snap in remaining.iter().take(overflow) {
                to_delete.insert(snap.id.clone());
            }
        }

        for id in to_delete {
            if is_dry_run {
                debug!("Snapshot: [DRY-RUN] Would prune {}", id);
            } else {
                p.delete(&id).await?;
            }
        }
        Ok(())
    }

    pub async fn list_snapshots(&self) -> Result<Vec<Snapshot>> {
        if let Some(ref p) = self.provider {
            p.list().await
        } else {
            Ok(vec![])
        }
    }

    /// Only ever deletes snapshots whose id contains "linix", so retention cannot reap a
    /// user's or another tool's snapshots. Inactive policy or no provider = no-op.
    pub async fn prune_with_policy(
        &self,
        policy: &crate::core::RetentionPolicy,
        now: DateTime<Utc>,
        dry_run: bool,
    ) -> Result<Vec<String>> {
        let p = match &self.provider {
            Some(p) => p,
            None => return Ok(vec![]),
        };
        if !policy.is_active() {
            return Ok(vec![]);
        }
        let list: Vec<Snapshot> = p
            .list()
            .await?
            .into_iter()
            .filter(|s| s.id.contains("linix"))
            .collect();
        let items: Vec<crate::core::RetentionItem> = list
            .iter()
            .map(|s| crate::core::RetentionItem {
                id: s.id.clone(),
                label: s.description.clone(),
                timestamp: s.parse_time().unwrap_or(now),
                pinned: false,
            })
            .collect();
        let doomed = policy.select_deletions(&items, now);
        for id in &doomed {
            if dry_run {
                debug!("Snapshot: [DRY-RUN] retention would prune {}", id);
            } else if let Err(e) = p.delete(id).await {
                debug!("Snapshot: retention could not delete {}: {}", id, e);
            }
        }
        Ok(doomed)
    }

    pub async fn restore_snapshot(&self, id: &str) -> Result<()> {
        if let Some(ref p) = self.provider {
            p.restore(id).await
        } else {
            Err(Error::Snapshot("No active provider".into()))
        }
    }
}
