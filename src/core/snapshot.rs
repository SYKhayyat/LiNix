use crate::config::Config;
use crate::core::{CommandExecutor, Error, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command as StdCommand;
use tracing::{debug, info, warn};

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
    PreRebuild,
}

impl SnapshotLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            SnapshotLabel::PreSync => "pre_sync",
            SnapshotLabel::PreUpgrade => "pre_upgrade",
            SnapshotLabel::PurgeUnmanaged => "purge-unmanaged",
            SnapshotLabel::PreCanary => "pre_canary",
            SnapshotLabel::PreRebuild => "pre_rebuild",
        }
    }
}

impl std::fmt::Display for SnapshotLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Taking a snapshot and putting one back are different capabilities, and a provider can
/// have the first without the second. `btrfs subvolume snapshot SRC /` exits 0 and creates a
/// nested subvolume; nothing is rolled back. Everything that offers an undo asks this first,
/// so the offer is not made where it cannot be kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreCapability {
    /// The running system is put back by `restore`.
    Live,
    /// The snapshot is real and restorable, but not from a running system. `how` says what
    /// the person at the machine has to do instead. Owned because a config-driven provider
    /// (U27) supplies its own sentence, and a built-in supplies a `&'static` one via `.into()`.
    NotFromRunningSystem { how: String },
}

impl RestoreCapability {
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Live)
    }

    /// The one sentence `doctor` and the pre-change notice both print, so the two cannot
    /// come to disagree about what this machine can do.
    pub fn describe(&self, provider: &str) -> String {
        match self {
            Self::Live => format!("{}: snapshots can be taken and restored.", provider),
            Self::NotFromRunningSystem { how } => format!(
                "{}: snapshots can be taken but NOT restored from a running system — {}",
                provider, how
            ),
        }
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
    fn restore_capability(&self) -> RestoreCapability;
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
        Err(Error::Snapshot(format!(
            "LiNix cannot roll this machine back to {} while it is running.\n  \
             {}\n  \
             The snapshot is intact at {}/{}. Rolling back to it means swapping the root \
             subvolume from a live USB or the boot menu — a step LiNix will not take on your \
             behalf, because getting it wrong leaves a machine that does not boot.",
            id,
            BTRFS_NOT_LIVE,
            self.snapshot_root,
            id
        )))
    }

    fn restore_capability(&self) -> RestoreCapability {
        RestoreCapability::NotFromRunningSystem {
            how: BTRFS_NOT_LIVE.into(),
        }
    }
}

/// `btrfs subvolume snapshot <snap> /` exits 0 and creates `<snap>` *inside* `/` as a nested
/// subvolume. Every recovery path in this tree believed that exit code once.
const BTRFS_NOT_LIVE: &str =
    "a btrfs rollback replaces the root subvolume, which cannot be done over a mounted `/`; \
     boot from other media and swap the subvolume there.";

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

    fn restore_capability(&self) -> RestoreCapability {
        RestoreCapability::Live
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

    fn restore_capability(&self) -> RestoreCapability {
        RestoreCapability::Live
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

    fn restore_capability(&self) -> RestoreCapability {
        RestoreCapability::Live
    }
}

/// macOS APFS local snapshots (U29). Every Mac ships APFS, and `tmutil localsnapshot` takes a
/// whole-volume snapshot with no configuration — so the second platform LiNix supports gains the
/// safety net it had none of.
///
/// **Declared create-only, on purpose (V.60).** An APFS restore is not a live operation: it
/// needs a reboot into the recovery environment and `Restore from Time Machine`/`tmutil restore`,
/// which LiNix cannot drive on a running system. Claiming `Live` here would be the exact bug
/// V.60 exists for — a `restore` that exits without rolling the machine back. So it snapshots,
/// and it refuses the rollback with the steps to do it by hand.
pub struct ApfsProvider {
    pub executor: CommandExecutor,
}

/// The mount point APFS local snapshots are taken of. The system volume, always.
const APFS_VOLUME: &str = "/";

const APFS_NOT_LIVE: &str =
    "an APFS snapshot is restored by rebooting into macOS Recovery and using Time Machine / \
     `tmutil restore`; LiNix cannot do that over a running system.";

#[async_trait]
impl SnapshotProvider for ApfsProvider {
    fn name(&self) -> &str {
        "apfs"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "macos") && self.executor.command_exists("tmutil").await
    }

    async fn create(&self, label: SnapshotLabel) -> Result<Snapshot> {
        // `tmutil localsnapshot` names the snapshot itself, by date, and does not take a label —
        // so the LiNix marker lands in the description (like the Windows provider), where
        // ownership (S3) reads it. `label` is an enum, so nothing user-supplied reaches here.
        let out = self
            .executor
            .run_output("tmutil", &["localsnapshot"], true)
            .await?;
        // `tmutil` prints `Created local snapshot with date: 2026-07-26-120000`. Read the date
        // back as the id; fall back to now-formatted if the phrasing changes.
        let id = out
            .lines()
            .find_map(|l| l.rsplit_once("date: ").map(|(_, d)| d.trim().to_string()))
            .unwrap_or_else(|| Local::now().format("%Y-%m-%d-%H%M%S").to_string());
        Ok(Snapshot {
            id,
            timestamp: Utc::now().to_rfc3339(),
            description: format!("LiNix: {}", label),
            backend: "apfs".into(),
        })
    }

    async fn list(&self) -> Result<Vec<Snapshot>> {
        let out = self
            .executor
            .run_output("tmutil", &["listlocalsnapshots", APFS_VOLUME], false)
            .await?;
        Ok(out
            .lines()
            .filter_map(|l| {
                // `com.apple.TimeMachine.2026-07-26-120000.local` → the date is the id.
                let line = l.trim();
                let date = line
                    .strip_prefix("com.apple.TimeMachine.")
                    .and_then(|s| s.strip_suffix(".local"))
                    .unwrap_or(line);
                if date.is_empty() {
                    return None;
                }
                Some(Snapshot {
                    id: date.to_string(),
                    timestamp: Utc::now().to_rfc3339(),
                    // LiNix cannot tell from `tmutil` which snapshots it made — the marker is in
                    // a description APFS does not store. So these are reported but never reaped by
                    // retention (is_linix_owned is false), which is the safe direction: LiNix
                    // never deletes an APFS snapshot it cannot prove it created.
                    description: "APFS local snapshot".into(),
                    backend: "apfs".into(),
                })
            })
            .collect())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        self.executor
            .run("tmutil", &["deletelocalsnapshots", id], true)
            .await
            .map(|_| ())
    }

    async fn restore(&self, id: &str) -> Result<()> {
        Err(Error::Snapshot(format!(
            "LiNix cannot roll this Mac back to {} while it is running.\n  {}\n  \
             The snapshot is intact and listed by `tmutil listlocalsnapshots /`.",
            id, APFS_NOT_LIVE
        )))
    }

    fn restore_capability(&self) -> RestoreCapability {
        RestoreCapability::NotFromRunningSystem {
            how: APFS_NOT_LIVE.into(),
        }
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

/// A snapshot provider described entirely in `adapters/snapshot.toml` (U27) — the same "rows,
/// not Rust" move the backend, firewall, settings and init layers already made. A filesystem
/// with create/list/delete/restore-shaped commands becomes a provider with no source change.
///
/// **The one rule that keeps this from being the V.60 footgun: a capability the row does not
/// declare, it does not have.** `restores_running_system` defaults to `false`, so a provider is
/// create-only unless the file *says* it can put a running machine back — and saying so is the
/// line a reviewer sees in the diff. A row that omits it can snapshot and can refuse a rollback;
/// it can never run a command that "restores" and rolls nothing back.
#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotProviderDef {
    pub name: String,
    /// Restrict to one OS (`std::env::consts::OS`). Absent means any.
    #[serde(default)]
    pub os: Option<String>,
    /// The command whose presence means this provider can act on this machine.
    pub detect: String,
    /// What `{source}` expands to (a dataset, a volume group, a subvolume path). Optional.
    #[serde(default)]
    pub source: String,
    /// Placeholders: `{id}`, `{label}`, `{source}`.
    pub create: Vec<String>,
    pub list: Vec<String>,
    /// Placeholders: `{id}`.
    pub delete: Vec<String>,
    /// Placeholders: `{id}`. Empty means this provider cannot restore at all.
    #[serde(default)]
    pub restore: Vec<String>,
    /// The safe default (U27/V.60): a provider is create-only unless the file names this true.
    #[serde(default)]
    pub restores_running_system: bool,
    /// A regex whose first capture group is a snapshot id on each `list` line.
    pub list_pattern: String,
    /// The sentence shown when a create-only provider is asked to restore. A default is supplied
    /// when the row omits it, so the refusal is never blank.
    #[serde(default)]
    pub restore_how: Option<String>,
}

impl SnapshotProviderDef {
    /// A row LiNix will drive, or why it will not. It must be able to create, list and delete;
    /// restore is the capability that is allowed to be absent, and its absence is the safe state.
    pub fn is_usable(&self) -> Option<&'static str> {
        if self.name.trim().is_empty() {
            return Some("it has no `name`");
        }
        if self.detect.trim().is_empty() {
            return Some("it has no `detect` command");
        }
        if self.create.is_empty() {
            return Some("it cannot create a snapshot");
        }
        if self.list.is_empty() {
            return Some("it cannot list snapshots, so retention could never reap them");
        }
        if self.delete.is_empty() {
            return Some("it cannot delete a snapshot, so retention could never reap them");
        }
        if self.list_pattern.trim().is_empty() {
            return Some("it has no `list_pattern`, so a listed line has no id");
        }
        None
    }

    fn applies_to_this_os(&self) -> bool {
        match &self.os {
            Some(os) => os.eq_ignore_ascii_case(std::env::consts::OS),
            None => true,
        }
    }

    /// Whether a declared row can actually put a running system back. `restores_running_system`
    /// alone is not enough — a row that claims it but gives no `restore` command still cannot,
    /// and claiming `Live` there is exactly the V.60 lie.
    fn is_live(&self) -> bool {
        self.restores_running_system && !self.restore.is_empty()
    }
}

pub struct ConfigSnapshotProvider {
    pub executor: CommandExecutor,
    pub def: SnapshotProviderDef,
}

impl ConfigSnapshotProvider {
    fn fill(cmd: &[String], id: &str, label: &str, source: &str) -> Vec<String> {
        cmd.iter()
            .map(|a| {
                a.replace("{id}", id)
                    .replace("{label}", label)
                    .replace("{source}", source)
            })
            .collect()
    }

    async fn run(&self, cmd: Vec<String>) -> Result<()> {
        let (prog, args) = cmd
            .split_first()
            .ok_or_else(|| Error::Snapshot("a snapshot command is empty".into()))?;
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.executor.run(prog, &refs, true).await.map(|_| ())
    }
}

#[async_trait]
impl SnapshotProvider for ConfigSnapshotProvider {
    fn name(&self) -> &str {
        &self.def.name
    }

    async fn is_available(&self) -> bool {
        self.def.applies_to_this_os() && self.executor.command_exists(&self.def.detect).await
    }

    async fn create(&self, label: SnapshotLabel) -> Result<Snapshot> {
        // The id carries the `linix_` marker so ownership (S3) and retention recognize it — a
        // config provider that let a user's own snapshots look like LiNix's would have retention
        // reap them.
        let ts = Local::now().format("%Y%m%d_%H%M%S");
        let id = format!("linix_{}_{}", label.as_str(), ts);
        let cmd = Self::fill(&self.def.create, &id, label.as_str(), &self.def.source);
        info!("{}: creating snapshot {}", self.def.name, id);
        self.run(cmd).await?;
        Ok(Snapshot {
            id,
            timestamp: Utc::now().to_rfc3339(),
            description: label.to_string(),
            backend: self.def.name.clone(),
        })
    }

    async fn list(&self) -> Result<Vec<Snapshot>> {
        let (prog, args) = self
            .def
            .list
            .split_first()
            .ok_or_else(|| Error::Snapshot("a snapshot list command is empty".into()))?;
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.executor.run_output(prog, &refs, false).await?;
        let re = regex::Regex::new(&self.def.list_pattern)
            .map_err(|e| Error::Snapshot(format!("bad list_pattern: {}", e)))?;
        let mut snaps = Vec::new();
        for line in out.lines() {
            let Some(caps) = re.captures(line) else {
                continue;
            };
            let Some(m) = caps.get(1) else { continue };
            let id = m.as_str().to_string();
            snaps.push(Snapshot {
                timestamp: Snapshot::timestamp_from_id(&id)
                    .unwrap_or_else(|| Utc::now().to_rfc3339()),
                id,
                description: self.def.name.clone(),
                backend: self.def.name.clone(),
            });
        }
        Ok(snaps)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let cmd = Self::fill(&self.def.delete, id, "", &self.def.source);
        self.run(cmd).await
    }

    async fn restore(&self, id: &str) -> Result<()> {
        if !self.def.is_live() {
            // The V.60 refusal, config-driven: a provider that did not declare live restore does
            // not run a "restore" that might roll nothing back. It says so and leaves the
            // snapshot intact.
            return Err(Error::Snapshot(format!(
                "{}: this provider cannot roll a running system back to {}. {}",
                self.def.name,
                id,
                self.def
                    .restore_how
                    .clone()
                    .unwrap_or_else(|| "The snapshot is intact; restore it by hand.".into())
            )));
        }
        let cmd = Self::fill(&self.def.restore, id, "", &self.def.source);
        self.run(cmd).await
    }

    fn restore_capability(&self) -> RestoreCapability {
        if self.def.is_live() {
            RestoreCapability::Live
        } else {
            RestoreCapability::NotFromRunningSystem {
                how: self.def.restore_how.clone().unwrap_or_else(|| {
                    "this provider was not declared able to restore a running system".into()
                }),
            }
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SnapshotProviderFile {
    #[serde(default)]
    pub snapshot: Vec<SnapshotProviderDef>,
}

/// The config-driven providers this repo carries, if `adapters/snapshot.toml` is approved
/// through the one II.12 ledger every adapter file goes through. An unapproved or unparseable
/// file yields none, loudly — never a silent partial safety net.
fn config_snapshot_defs(config: &Config) -> Vec<SnapshotProviderDef> {
    let layout = config.layout();
    let path = layout.adapter_snapshot_file();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            warn!("could not read adapters/snapshot.toml: {}", e);
            return Vec::new();
        }
    };
    if let Some(refusal) = crate::core::hook_lock::adapter_refusal(&path, &content, &layout.locks_dir())
    {
        tracing::error!("{}", refusal);
        return Vec::new();
    }
    match toml::from_str::<SnapshotProviderFile>(&content) {
        Ok(f) => f
            .snapshot
            .into_iter()
            .filter(|d| match d.is_usable() {
                Some(why) => {
                    warn!("ignoring the `{}` snapshot provider: {}.", d.name, why);
                    false
                }
                None => true,
            })
            .collect(),
        Err(e) => {
            warn!("ignoring adapters/snapshot.toml: {}", e);
            Vec::new()
        }
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
            Box::new(ApfsProvider {
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

        // Config-declared providers register LAST (U27), so a `adapters/snapshot.toml` row can
        // never shadow a built-in — the `custom_backends.toml` rule applied to the safety layer.
        for def in config_snapshot_defs(config) {
            providers.push(Box::new(ConfigSnapshotProvider {
                executor: executor.duplicate(),
                def,
            }));
        }

        let active = Self::choose(providers, &config.snapshot_priority).await;
        Self { provider: active }
    }

    /// The active provider (U28). When a `snapshot_priority` is declared, the first *available*
    /// provider named in it wins — chosen by the user's declared order, not by registration
    /// order and not by capability-guessing. A provider named in the list but absent from the
    /// machine is skipped; a name in the list that matches no provider is ignored. With no list,
    /// the first available in registration order wins (built-ins first), which is the historical
    /// behaviour untouched.
    async fn choose(
        providers: Vec<Box<dyn SnapshotProvider>>,
        priority: &[String],
    ) -> Option<Box<dyn SnapshotProvider>> {
        // Which are actually usable on this machine, in registration order.
        let mut available: Vec<Box<dyn SnapshotProvider>> = Vec::new();
        for p in providers {
            if p.is_available().await {
                available.push(p);
            }
        }
        if priority.is_empty() {
            return available.into_iter().next();
        }
        for want in priority {
            if let Some(pos) = available
                .iter()
                .position(|p| p.name().eq_ignore_ascii_case(want))
            {
                return Some(available.swap_remove(pos));
            }
        }
        // A declared priority that names nothing present: fall back rather than leave the machine
        // with no safety net it could have had.
        available.into_iter().next()
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

    pub fn provider_name(&self) -> Option<&str> {
        self.provider.as_ref().map(|p| p.name())
    }

    /// What this machine's provider can do, or `None` when it takes no snapshots at all.
    pub fn restore_capability(&self) -> Option<RestoreCapability> {
        self.provider.as_ref().map(|p| p.restore_capability())
    }

    /// The one place a snapshot is put back. `undo` and every other recovery path calls
    /// this, so a provider that refuses refuses everywhere.
    pub async fn restore(&self, id: &str) -> Result<()> {
        let p = self.provider.as_ref().ok_or_else(|| {
            Error::Snapshot("this machine takes no snapshots, so there is none to restore".into())
        })?;
        p.restore(id).await
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
        // The return value is what the caller prints as "pruned N", so it carries only the
        // ids whose delete actually succeeded — a snapshot still on disk must never be
        // counted as reaped.
        let mut pruned = Vec::new();
        let mut failed = Vec::new();
        for id in &doomed {
            if dry_run {
                debug!("[DRY-RUN] retention would prune {}", id);
                pruned.push(id.clone());
            } else {
                match p.delete(id).await {
                    Ok(()) => pruned.push(id.clone()),
                    Err(e) => failed.push(format!("{} ({})", id, e)),
                }
            }
        }
        if !failed.is_empty() {
            warn!(
                "retention could not delete {} snapshot(s), still on disk: {}",
                failed.len(),
                failed.join(", ")
            );
        }
        Ok(pruned)
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

    fn def(name: &str) -> SnapshotProviderDef {
        SnapshotProviderDef {
            name: name.into(),
            os: None,
            detect: "true".into(),
            source: "tank/root".into(),
            create: vec!["mk".into(), "{id}".into(), "{source}".into()],
            list: vec!["ls".into()],
            delete: vec!["rm".into(), "{id}".into()],
            restore: vec![],
            restores_running_system: false,
            list_pattern: r"(\S+)".into(),
            restore_how: None,
        }
    }

    /// U27/V.60: a config provider is create-only unless the file *names* the live-restore
    /// capability. The default — omit the field — is the safe one, and even declaring the flag is
    /// not enough without a `restore` command to run.
    #[test]
    fn a_config_provider_is_create_only_unless_it_declares_both() {
        let mut d = def("lvm");
        assert!(!d.is_live(), "the default must be create-only");

        d.restores_running_system = true;
        assert!(!d.is_live(), "the flag alone, with no restore command, is not live");

        d.restore = vec!["merge".into(), "{id}".into()];
        assert!(d.is_live(), "flag AND a restore command is live");
    }

    #[test]
    fn a_create_only_config_provider_reports_not_from_running_system() {
        let d = def("lvm");
        let cap = ConfigSnapshotProvider {
            executor: CommandExecutor::new(true, false),
            def: d,
        }
        .restore_capability();
        assert!(!cap.is_live());
        match cap {
            RestoreCapability::NotFromRunningSystem { how } => assert!(!how.is_empty()),
            _ => panic!("a create-only provider must not report Live"),
        }
    }

    #[tokio::test]
    async fn a_create_only_config_provider_refuses_restore_and_names_the_snapshot() {
        let p = ConfigSnapshotProvider {
            executor: CommandExecutor::new(true, false),
            def: def("lvm"),
        };
        let err = p.restore("linix_pre_sync_20260726_120000").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("linix_pre_sync_20260726_120000"), "{}", msg);
        assert!(msg.contains("cannot roll"), "{}", msg);
    }

    #[test]
    fn a_config_provider_missing_a_required_command_is_refused() {
        let mut d = def("lvm");
        d.create = vec![];
        assert!(d.is_usable().is_some());
        let mut d = def("lvm");
        d.list = vec![];
        assert!(d.is_usable().is_some());
        let mut d = def("lvm");
        d.list_pattern = String::new();
        assert!(d.is_usable().is_some());
        assert!(def("lvm").is_usable().is_none(), "a complete row is usable");
    }

    #[test]
    fn the_snapshot_provider_schema_parses() {
        let toml = r#"
[[snapshot]]
name = "lvm"
detect = "lvcreate"
source = "vg0/root"
create = ["lvcreate", "--snapshot", "--name", "{id}", "{source}"]
list = ["lvs", "--noheadings", "-o", "lv_name"]
delete = ["lvremove", "-y", "{id}"]
restore = ["lvconvert", "--merge", "{id}"]
restores_running_system = true
list_pattern = '(linix_\S+)'
"#;
        let file: SnapshotProviderFile = toml::from_str(toml).unwrap();
        assert_eq!(file.snapshot.len(), 1);
        assert!(file.snapshot[0].is_live());
        assert!(file.snapshot[0].is_usable().is_none());
    }

    // A trivial provider for the priority test: available iff `here`, named `name`.
    struct Fake {
        name: String,
        here: bool,
    }
    #[async_trait]
    impl SnapshotProvider for Fake {
        fn name(&self) -> &str {
            &self.name
        }
        async fn is_available(&self) -> bool {
            self.here
        }
        async fn create(&self, _l: SnapshotLabel) -> Result<Snapshot> {
            unreachable!()
        }
        async fn list(&self) -> Result<Vec<Snapshot>> {
            Ok(vec![])
        }
        async fn delete(&self, _id: &str) -> Result<()> {
            Ok(())
        }
        async fn restore(&self, _id: &str) -> Result<()> {
            Ok(())
        }
        fn restore_capability(&self) -> RestoreCapability {
            RestoreCapability::Live
        }
    }

    fn fake(name: &str, here: bool) -> Box<dyn SnapshotProvider> {
        Box::new(Fake {
            name: name.into(),
            here,
        })
    }

    #[tokio::test]
    async fn priority_picks_the_first_available_in_the_declared_order() {
        // btrfs and zfs both present; the list prefers zfs, so zfs wins over registration order.
        let providers = vec![fake("btrfs", true), fake("zfs", true)];
        let chosen = SnapshotManager::choose(providers, &["zfs".into(), "btrfs".into()])
            .await
            .unwrap();
        assert_eq!(chosen.name(), "zfs");
    }

    #[tokio::test]
    async fn priority_skips_a_named_provider_that_is_absent() {
        // The list names zfs first, but zfs is not on this machine — so the next available named
        // one wins, not "nothing".
        let providers = vec![fake("btrfs", true), fake("zfs", false)];
        let chosen = SnapshotManager::choose(providers, &["zfs".into(), "btrfs".into()])
            .await
            .unwrap();
        assert_eq!(chosen.name(), "btrfs");
    }

    #[tokio::test]
    async fn no_priority_keeps_registration_order() {
        let providers = vec![fake("btrfs", true), fake("zfs", true)];
        let chosen = SnapshotManager::choose(providers, &[]).await.unwrap();
        assert_eq!(chosen.name(), "btrfs", "built-ins first when no list is declared");
    }

    #[tokio::test]
    async fn a_priority_that_names_nothing_present_still_falls_back() {
        let providers = vec![fake("btrfs", true)];
        let chosen = SnapshotManager::choose(providers, &["apfs".into()])
            .await
            .unwrap();
        assert_eq!(chosen.name(), "btrfs");
    }

    /// APFS is create-only (U29/V.60): a Mac restore needs a recovery-mode reboot, so claiming
    /// Live would be the V.60 lie.
    #[test]
    fn apfs_is_declared_create_only() {
        let p = ApfsProvider {
            executor: CommandExecutor::new(true, false),
        };
        assert!(!p.restore_capability().is_live());
    }
}
