use crate::core::{
    CommandExecutor, Error, ManagedPackage, Result, Snapshot, SnapshotManager, StateRegistry,
};
use dialoguer::{theme::ColorfulTheme, Select};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Manages the Snapshot Gallery and System Time Travel (Point 12).
///
/// Hardened for Phase 4.1: Decoupled from the global App object. Now receives
/// specific trait-based dependencies, enabling isolated testing of the rollback logic.
pub struct UndoManager {
    snapshot_manager: Arc<SnapshotManager>,
    state: Arc<Mutex<StateRegistry>>,
    executor: CommandExecutor,
    /// Manifest directory, so a snapshot rollback can also restore the matching generation.
    groups_dir: PathBuf,
}

#[derive(Debug, Default)]
struct StateDiff {
    added: Vec<ManagedPackage>,
    removed: Vec<ManagedPackage>,
    changed: Vec<(ManagedPackage, ManagedPackage)>, // (Current, Snapshot)
}

/// Allowed snapshot directories (whitelist approach for security)
const ALLOWED_SNAPSHOT_PREFIXES: &[&str] = &[
    "/.snapshots/",
    "/run/timeshift/",
    "/timeshift/",
    "/var/lib/snapper/",
    "/.zfs/snapshot/",
];

/// Paths that are NEVER allowed to be accessed
const FORBIDDEN_PATHS: &[&str] = &[
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/passwd",
    "/boot",
    "/dev",
    "/proc",
    "/sys",
];

impl UndoManager {
    /// Creates a new UndoManager with explicit dependency injection.
    pub fn new(
        snapshot_manager: Arc<SnapshotManager>,
        state: Arc<Mutex<StateRegistry>>,
        executor: CommandExecutor,
        groups_dir: PathBuf,
    ) -> Self {
        Self {
            snapshot_manager,
            state,
            executor,
            groups_dir,
        }
    }

    /// After restoring a filesystem snapshot, also restore the generation that was current
    /// at that snapshot's time — so the manifests and the realized-state record match the
    /// system you rolled back to. Best-effort: never fails the snapshot restore.
    async fn restore_matching_generation(&self, snapshot: &Snapshot) {
        let Some(when) = snapshot.parse_time() else {
            return;
        };
        let dir = {
            let state = self.state.lock().await;
            state
                .path
                .parent()
                .map(|p| p.join("generations"))
                .unwrap_or_else(|| crate::utils::safe_data_dir().join("generations"))
        };
        let store = crate::app::generation::GenerationStore::new(dir);
        match store.nearest_at_or_before(when).await {
            Ok(Some(gen)) => {
                let mut state = self.state.lock().await;
                if let Err(e) = store.restore(&gen.id, &mut state, &self.groups_dir).await {
                    warn!("Undo: could not restore matching generation {}: {}", gen.id, e);
                } else {
                    info!(
                        "Undo: also restored generation {} (state + manifests) matching the snapshot.",
                        gen.id
                    );
                }
            }
            Ok(None) => debug!("Undo: no generation at or before the snapshot's time."),
            Err(e) => warn!("Undo: generation lookup failed: {}", e),
        }
    }

    /// Entry point for 'linix undo'.
    pub async fn run_interactive(&self) -> Result<()> {
        info!("UndoManager: Querying available system snapshots...");

        let snapshots = self.snapshot_manager.list_snapshots().await?;
        if snapshots.is_empty() {
            println!("No system snapshots found. LiNix cannot perform time travel on this system.");
            return Ok(());
        }

        println!("\n--- LiNix Snapshot Gallery ---");
        let items: Vec<String> = snapshots
            .iter()
            .map(|s| {
                format!(
                    "[{}] {} - {} ({})",
                    s.backend, s.timestamp, s.description, s.id
                )
            })
            .collect();

        // Dialoguer is blocking; wrap in spawn_blocking
        let selection = tokio::task::spawn_blocking(move || {
            Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Select a system state to inspect/restore (ESC to cancel)")
                .default(0)
                .items(&items)
                .interact_opt()
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))?
        .map_err(|e| Error::Other(e.to_string()))?;

        if let Some(index) = selection {
            let selected = &snapshots[index];
            self.show_diff_and_confirm(selected).await?;
        }

        Ok(())
    }

    /// Validates that a path is safe to access.
    async fn validate_snapshot_path(&self, path: &Path, snapshot_backend: &str) -> Result<PathBuf> {
        let path_owned = path.to_path_buf();
        let canonical = tokio::task::spawn_blocking(move || path_owned.canonicalize())
            .await
            .map_err(|e| Error::Other(e.to_string()))?
            .map_err(|e| {
                Error::Snapshot(format!("Failed to canonicalize path {:?}: {}", path, e))
            })?;

        let path_str = canonical.to_string_lossy();

        for forbidden in FORBIDDEN_PATHS {
            if path_str.contains(forbidden) {
                return Err(Error::Snapshot(format!(
                    "Security violation: Attempted to access forbidden path '{}'",
                    forbidden
                )));
            }
        }

        let allowed_prefixes = match snapshot_backend {
            "btrfs" => vec!["/.snapshots/", "/var/lib/snapper/"],
            "timeshift" => vec!["/run/timeshift/", "/timeshift/"],
            "zfs" => vec!["/.zfs/snapshot/"],
            _ => ALLOWED_SNAPSHOT_PREFIXES.to_vec(),
        };

        let mut is_allowed = false;
        for prefix in &allowed_prefixes {
            if path_str.starts_with(prefix) {
                is_allowed = true;
                break;
            }
        }

        if !is_allowed {
            return Err(Error::Snapshot(format!(
                "Security violation: Snapshot path '{}' is outside allowed directories.",
                path_str
            )));
        }

        Ok(canonical)
    }

    async fn find_registry_in_snapshot(&self, snapshot_root: &Path) -> Result<Option<PathBuf>> {
        let possible_paths = vec![
            snapshot_root.join("var/lib/linix/registry.json"),
            snapshot_root.join("root/.local/share/linix/registry.json"),
            snapshot_root.join(".local/share/linix/registry.json"),
        ];

        for path in possible_paths {
            if tokio::fs::try_exists(&path).await.unwrap_or(false) {
                debug!("Found registry.json at {:?}", path);
                return Ok(Some(path));
            }
        }

        Ok(None)
    }

    async fn show_diff_and_confirm(&self, snapshot: &Snapshot) -> Result<()> {
        println!(
            "\nCalculating Package Diff for Snapshot: {}...",
            snapshot.id
        );

        let snapshot_root = match snapshot.backend.as_str() {
            "btrfs" => PathBuf::from(format!("/.snapshots/{}", snapshot.id)),
            "timeshift" => PathBuf::from(format!(
                "/run/timeshift/backup/timeshift/snapshots/{}",
                snapshot.id
            )),
            _ => {
                return Err(Error::Snapshot(format!(
                    "Unsupported snapshot backend: {}",
                    snapshot.backend
                )))
            }
        };

        let validated_root = self
            .validate_snapshot_path(&snapshot_root, &snapshot.backend)
            .await?;

        let snapshot_registry_path = match self.find_registry_in_snapshot(&validated_root).await? {
            Some(path) => path,
            None => {
                return Err(Error::Snapshot(
                    "Could not find registry.json in snapshot".into(),
                ));
            }
        };

        let data = fs::read_to_string(&snapshot_registry_path)
            .await
            .map_err(Error::from)?;

        let snapshot_state: StateRegistry =
            tokio::task::spawn_blocking(move || serde_json::from_str(&data))
                .await
                .map_err(|e| Error::Other(e.to_string()))?
                .map_err(Error::from)?;

        let current_state = self.state.lock().await;
        let diff = self.calculate_diff(&current_state, &snapshot_state);

        if !diff.added.is_empty() || !diff.removed.is_empty() || !diff.changed.is_empty() {
            println!("\nPACKAGE CHANGES (Rolling back will result in):");
            for p in &diff.removed {
                println!(
                    "  [+] Restore:  {}:{} (Version: {:?})",
                    p.backend, p.name, p.version
                );
            }
            for p in &diff.added {
                println!(
                    "  [-] Remove:   {}:{} (Not present in snapshot)",
                    p.backend, p.name
                );
            }
        } else {
            println!("\nNo package changes detected.");
        }

        warn!("\nCRITICAL: Reverting to this snapshot will overwrite your system root (/).");
        print!("Are you absolutely sure? Type 'RESTORE' to proceed: ");

        use std::io::{self, Write};
        let _ = io::stdout().flush();

        let confirm_res = tokio::task::spawn_blocking(|| {
            let mut input = String::new();
            io::stdin().read_line(&mut input).map(|_| input)
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))?
        .map_err(Error::from)?;

        if confirm_res.trim() == "RESTORE" {
            self.execute_restore(snapshot).await
        } else {
            info!("Restore aborted by user.");
            Ok(())
        }
    }

    fn calculate_diff(&self, current: &StateRegistry, past: &StateRegistry) -> StateDiff {
        let mut diff = StateDiff::default();
        let curr_map: HashMap<String, &ManagedPackage> = current
            .packages
            .iter()
            .map(|p| (format!("{}:{}", p.backend, p.name), p))
            .collect();
        let past_map: HashMap<String, &ManagedPackage> = past
            .packages
            .iter()
            .map(|p| (format!("{}:{}", p.backend, p.name), p))
            .collect();

        for (key, pkg) in &curr_map {
            if !past_map.contains_key(key) {
                diff.added.push((*pkg).clone());
            }
        }

        for (key, pkg) in &past_map {
            if !curr_map.contains_key(key) {
                diff.removed.push((*pkg).clone());
            } else {
                let curr_pkg = curr_map.get(key).unwrap();
                if curr_pkg.version != pkg.version {
                    diff.changed.push(((*curr_pkg).clone(), (*pkg).clone()));
                }
            }
        }

        diff
    }

    async fn execute_restore(&self, snapshot: &Snapshot) -> Result<()> {
        info!(
            "Undo: Commencing system restoration via {}...",
            snapshot.backend
        );

        match snapshot.backend.as_str() {
            "btrfs" => {
                let snapshot_path = format!("/.snapshots/{}", snapshot.id);
                self.executor
                    .run(
                        "btrfs",
                        &["subvolume", "snapshot", &snapshot_path, "/"],
                        true,
                    )
                    .await?;
            }
            "timeshift" => {
                let args = [
                    "--restore",
                    "--snapshot",
                    &snapshot.id,
                    "--target-device",
                    "/",
                    "--yes",
                ];
                self.executor.run("timeshift", &args, true).await?;
            }
            _ => {
                return Err(Error::Snapshot(format!(
                    "Unsupported provider: {}",
                    snapshot.backend
                )))
            }
        }

        // Pair the filesystem rollback with its generation so the manifests + state record
        // match the system you restored.
        self.restore_matching_generation(snapshot).await;

        println!("\nSUCCESS: System root has been restored. Please reboot.");
        Ok(())
    }
}
