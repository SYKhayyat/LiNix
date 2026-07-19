use crate::config::Config;
use crate::core::{CommandExecutor, Error, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
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

    /// Recover a snapshot's creation time from the timestamp LiNix embeds in the id it
    /// generates (S2). `list()` cannot get this from btrfs/zfs — their creation-time flags and
    /// output formats vary by version — but every id LiNix makes carries the time in a fixed
    /// shape:
    ///
    /// - btrfs: `linix_pre_<label>_<YYYYMMDDHHMMSS>`
    /// - zfs:   `<dataset>@linix_<YYYYMMDD_HHMMSS>`
    ///
    /// The digits are local wall-clock (that is how `create()` formats them), so they are read
    /// back as local time. Returns `None` for an id in neither shape — e.g. a snapshot LiNix
    /// did not create — so the caller can fall back rather than trust a wrong time.
    ///
    /// This is the fix for the bug where `list()` stamped every snapshot with `Utc::now()`, so
    /// each read as zero seconds old and age-based retention (`max_age_days`, `keep_days`) could
    /// never fire — a retention policy that silently keeps everything (P3).
    pub fn time_from_id(id: &str) -> Option<DateTime<Utc>> {
        // zfs first: the part after the last `@linix_`, formatted `%Y%m%d_%H%M%S`.
        if let Some(rest) = id.rsplit_once("@linix_") {
            if let Ok(naive) = NaiveDateTime::parse_from_str(rest.1.trim(), "%Y%m%d_%H%M%S") {
                return local_naive_to_utc(naive);
            }
        }
        // btrfs: the trailing `_<14 digits>` group.
        if let Some(tail) = id.rsplit('_').next() {
            if tail.len() == 14 && tail.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(naive) = NaiveDateTime::parse_from_str(tail, "%Y%m%d%H%M%S") {
                    return local_naive_to_utc(naive);
                }
            }
        }
        None
    }

    /// The rfc3339 string for the [`Snapshot::time_from_id`] of `id`, or `None` if the id
    /// carries no recognizable time. Used by `list()` to fill the `timestamp` field.
    pub fn timestamp_from_id(id: &str) -> Option<String> {
        Self::time_from_id(id).map(|t| t.to_rfc3339())
    }

    /// Whether LiNix created this snapshot — the ownership test retention uses so it never
    /// reclaims a restore point the user made by hand (S3).
    ///
    /// The marker lands in different fields per provider: btrfs/zfs put `linix_` in the **id**
    /// (`linix_pre_…`, `…@linix_…`), while Windows System Restore forces the id to a bare
    /// `SequenceNumber` and carries `LiNix:` in the **description**. Checking only the id — the
    /// old bug — meant nothing LiNix created on Windows was ever pruned. So check both, and do
    /// it case-insensitively to catch `LiNix:` as well as `linix_`.
    pub fn is_linix_owned(&self) -> bool {
        let has_marker = |s: &str| s.to_lowercase().contains("linix");
        has_marker(&self.id) || has_marker(&self.description)
    }
}

/// Interpret a naive datetime as local wall-clock (how snapshot ids are formatted) and convert
/// to UTC. Ambiguous local times (a DST fall-back hour) resolve to the earlier instant, which
/// for a retention age is close enough and never panics.
fn local_naive_to_utc(naive: NaiveDateTime) -> Option<DateTime<Utc>> {
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Why LiNix took a snapshot. There are exactly these four, and they are the only text that
/// reaches the Windows provider's PowerShell interpolation — a `&str` there would put a future
/// `--label` flag one hop from an elevated shell (SEC5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotLabel {
    PreSync,
    PreUpgrade,
    PurgeUnmanaged,
    PreCanary,
}

impl SnapshotLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            SnapshotLabel::PreSync => "pre_sync",
            SnapshotLabel::PreUpgrade => "pre_upgrade",
            SnapshotLabel::PurgeUnmanaged => "purge-unmanaged",
            SnapshotLabel::PreCanary => "pre_canary",
        }
    }
}

impl std::fmt::Display for SnapshotLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[async_trait]
pub trait SnapshotProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn is_available(&self) -> bool;
    async fn create(&self, label: SnapshotLabel) -> Result<Snapshot>;
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

    async fn create(&self, label: SnapshotLabel) -> Result<Snapshot> {
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
                    // S2: read the real creation time out of the id, not `Utc::now()` — else
                    // every listed snapshot is zero seconds old and retention keeps them all.
                    timestamp: Snapshot::timestamp_from_id(id)
                        .unwrap_or_else(|| Utc::now().to_rfc3339()),
                    id: id.to_string(),
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

    async fn create(&self, label: SnapshotLabel) -> Result<Snapshot> {
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
            .map(|l| {
                let id = l.trim();
                Snapshot {
                    // S2: derive the age from the id, not `Utc::now()`.
                    timestamp: Snapshot::timestamp_from_id(id)
                        .unwrap_or_else(|| Utc::now().to_rfc3339()),
                    id: id.to_string(),
                    description: "ZFS Snapshot".into(),
                    backend: "zfs".into(),
                }
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

    async fn create(&self, label: SnapshotLabel) -> Result<Snapshot> {
        let out = self
            .executor
            .run_output(
                "timeshift",
                &["--create", "--comments", label.as_str(), "--tags", "D"],
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

    async fn create(&self, label: SnapshotLabel) -> Result<Snapshot> {
        // `label` is an enum, so no caller can bring a `'` to this interpolation (SEC5).
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
                // Parsed at the boundary Windows hands it over, so nothing downstream carries
                // a SequenceNumber that was never a number (SEC5). `to_string()` here kept the
                // JSON quotes when the field came back as a string.
                let seq = item["SequenceNumber"]
                    .as_u64()
                    .or_else(|| item["SequenceNumber"].as_str()?.trim().parse().ok());
                let Some(seq) = seq else {
                    debug!("skipping a restore point with no usable SequenceNumber: {item}");
                    continue;
                };
                list.push(Snapshot {
                    id: seq.to_string(),
                    timestamp: item["CreationTime"].as_str().unwrap_or("").to_string(),
                    description: item["Description"].as_str().unwrap_or("").to_string(),
                    backend: "windows_restore".into(),
                });
            }
        }
        Ok(list)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let seq = Self::sequence_number(id)?;
        let ps_cmd = format!("Get-WmiObject -Namespace root\\default -Class SystemRestore | ForEach-Object {{ $_.DeleteStatus({}) }}", seq);
        self.executor
            .run("powershell", &["-Command", &ps_cmd], true)
            .await
            .map(|_| ())
    }

    async fn restore(&self, id: &str) -> Result<()> {
        let seq = Self::sequence_number(id)?;
        let ps_cmd = format!("Restore-Computer -RestorePoint {} -Confirm:$false", seq);
        self.executor
            .run("powershell", &["-Command", &ps_cmd], true)
            .await
            .map(|_| ())
    }
}

impl WindowsRestoreProvider {
    /// A Windows restore point is a `SequenceNumber`. Both PowerShell commands below
    /// interpolate it unquoted and run elevated, so it becomes a number here or it does not
    /// reach them at all — there is no quoting to get right for a `u32` (SEC5).
    fn sequence_number(id: &str) -> Result<u32> {
        id.trim().parse::<u32>().map_err(|_| {
            Error::Validation(format!(
                "`{}` is not a Windows restore point — an id is a SequenceNumber, a plain number.",
                id
            ))
        })
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

    pub async fn auto_snapshot(&self, label: SnapshotLabel) -> Result<Option<Snapshot>> {
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
            .filter(|s| s.is_linix_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restore_point_id_that_is_not_a_number_never_reaches_powershell() {
        // SEC5. Both PowerShell strings interpolate the id unquoted and run elevated.
        assert_eq!(WindowsRestoreProvider::sequence_number("42").unwrap(), 42);
        assert_eq!(WindowsRestoreProvider::sequence_number("  7 ").unwrap(), 7);
        for bad in [
            "1); Start-Process calc; #",
            "-1",
            "1 2",
            "",
            "$(whoami)",
            "0x10",
        ] {
            assert!(
                WindowsRestoreProvider::sequence_number(bad).is_err(),
                "`{}` must be refused",
                bad
            );
        }
    }

    #[test]
    fn every_snapshot_label_is_a_fixed_string() {
        // SEC5: the enum is the guard, so the set is closed and quote-free by construction.
        for l in [
            SnapshotLabel::PreSync,
            SnapshotLabel::PreUpgrade,
            SnapshotLabel::PurgeUnmanaged,
            SnapshotLabel::PreCanary,
        ] {
            assert!(
                !l.as_str().contains('\'') && !l.as_str().is_empty(),
                "{} must be safe inside a single-quoted PowerShell string",
                l
            );
        }
    }

    // Build ids the way `create()` does, from a known local time, so a round-trip proves the
    // parse regardless of the test machine's timezone.
    fn btrfs_id(local: DateTime<Local>) -> String {
        format!("linix_pre_pre_sync_{}", local.format("%Y%m%d%H%M%S"))
    }
    fn zfs_id(local: DateTime<Local>) -> String {
        format!("tank/root@linix_{}", local.format("%Y%m%d_%H%M%S"))
    }

    #[test]
    fn btrfs_id_round_trips_to_its_creation_time() {
        let t = Local.with_ymd_and_hms(2026, 7, 17, 14, 30, 22).unwrap();
        let parsed = Snapshot::time_from_id(&btrfs_id(t)).expect("btrfs id carries a time");
        assert_eq!(parsed, t.with_timezone(&Utc));
    }

    #[test]
    fn zfs_id_round_trips_to_its_creation_time() {
        let t = Local.with_ymd_and_hms(2026, 7, 17, 14, 30, 22).unwrap();
        let parsed = Snapshot::time_from_id(&zfs_id(t)).expect("zfs id carries a time");
        assert_eq!(parsed, t.with_timezone(&Utc));
    }

    #[test]
    fn an_older_id_parses_to_an_earlier_time_than_a_newer_one() {
        // The property retention actually depends on: order is preserved.
        let older = Local.with_ymd_and_hms(2026, 7, 10, 9, 0, 0).unwrap();
        let newer = Local.with_ymd_and_hms(2026, 7, 17, 9, 0, 0).unwrap();
        assert!(
            Snapshot::time_from_id(&btrfs_id(older)).unwrap()
                < Snapshot::time_from_id(&btrfs_id(newer)).unwrap()
        );
    }

    #[test]
    fn an_id_with_no_embedded_time_returns_none() {
        // A snapshot LiNix did not create, or a malformed id: no guess, so the caller falls
        // back rather than trusting a wrong time.
        assert!(Snapshot::time_from_id("some_manual_snapshot").is_none());
        assert!(Snapshot::time_from_id("tank/root@weekly-2026").is_none());
        // Right shape, non-numeric tail.
        assert!(Snapshot::time_from_id("linix_pre_sync_notadate12").is_none());
    }

    fn snap(id: &str, description: &str, backend: &str) -> Snapshot {
        Snapshot {
            id: id.into(),
            timestamp: Utc::now().to_rfc3339(),
            description: description.into(),
            backend: backend.into(),
        }
    }

    #[test]
    fn ownership_is_recognized_across_every_provider() {
        // S3: the marker lands in different fields per provider. All of these are LiNix's.
        assert!(snap(
            "linix_pre_pre_sync_20260717143022",
            "BTRFS System State",
            "btrfs"
        )
        .is_linix_owned());
        assert!(snap("tank/root@linix_20260717_143022", "ZFS Snapshot", "zfs").is_linix_owned());
        // Windows: id is a bare sequence number, marker is in the description — the case the
        // old id-only filter missed entirely.
        assert!(snap("12", "LiNix: pre_sync", "windows_restore").is_linix_owned());
    }

    #[test]
    fn a_user_made_snapshot_is_not_owned_and_is_left_alone() {
        assert!(!snap("12", "Windows Update", "windows_restore").is_linix_owned());
        assert!(!snap("tank/root@weekly", "manual weekly", "zfs").is_linix_owned());
    }

    #[test]
    fn a_parsed_snapshot_reads_its_real_age_not_zero() {
        // The bug in one assertion: a snapshot created a week ago must NOT read as ~now.
        let a_week_ago = Local::now() - chrono::Duration::days(7);
        let snap = Snapshot {
            id: btrfs_id(a_week_ago),
            timestamp: Snapshot::timestamp_from_id(&btrfs_id(a_week_ago)).unwrap(),
            description: "test".into(),
            backend: "btrfs".into(),
        };
        let age = Utc::now() - snap.parse_time().unwrap();
        assert!(age.num_days() >= 6, "age should be ~7 days, was {:?}", age);
    }
}
