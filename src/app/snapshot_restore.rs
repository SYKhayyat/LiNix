use crate::core::{Error, ManagedPackage, Result, Snapshot, SnapshotManager, StateRegistry};
use dialoguer::{theme::ColorfulTheme, Select};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

pub struct SnapshotRestore {
    snapshot_manager: Arc<SnapshotManager>,
    state: Arc<Mutex<StateRegistry>>,
}

#[derive(Debug, Default)]
struct StateDiff {
    added: Vec<ManagedPackage>,
    removed: Vec<ManagedPackage>,
    changed: Vec<(ManagedPackage, ManagedPackage)>, // (Current, Snapshot)
}

/// Snapshot roots `validate_snapshot_path` will read from. Enforced only on the read path
/// (mounting a snapshot to diff its registry); `execute_restore` hands the snapshot to
/// btrfs/timeshift, which write over `/` without consulting this list.
const ALLOWED_SNAPSHOT_PREFIXES: &[&str] = &[
    "/.snapshots/",
    "/run/timeshift/",
    "/timeshift/",
    "/var/lib/snapper/",
    "/.zfs/snapshot/",
];

/// Paths `validate_snapshot_path` refuses to read a snapshot **registry** out of — the guard
/// on the diff step, so a crafted snapshot path cannot make `snapshot restore` parse `/etc/shadow` as
/// JSON. It is NOT a global "never touch these" list (the name it used to have, `FORBIDDEN_
/// PATHS`, was a lie): `execute_restore` rolls the whole filesystem back over `/`, and
/// therefore over every path here. Adding an entry protects the registry-read path only.
const REGISTRY_READ_FORBIDDEN_PATHS: &[&str] = &[
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/passwd",
    "/boot",
    "/dev",
    "/proc",
    "/sys",
];

impl SnapshotRestore {
    pub fn new(snapshot_manager: Arc<SnapshotManager>, state: Arc<Mutex<StateRegistry>>) -> Self {
        Self {
            snapshot_manager,
            state,
        }
    }

    pub async fn run_interactive(&self) -> Result<()> {
        debug!("querying snapshots");

        let snapshots = self.snapshot_manager.list_snapshots().await?;
        if snapshots.is_empty() {
            println!("No system snapshots found. LiNix cannot perform time travel on this system.");
            return Ok(());
        }

        // Refused before the gallery is printed, not after: a list of restore points with no
        // way to choose one reads as a menu that ignored the keypress. The gallery is the
        // whole command, so there is no non-interactive form of it to fall back to —
        // `snapshot list` reports and `rollback <id>` acts, and both name themselves here.
        {
            use std::io::IsTerminal;
            if !std::io::stdin().is_terminal() {
                return Err(Error::Refused(
                    "Choosing a snapshot needs a terminal, and this shell has none.\n\
                     `linix snapshot list` prints the same gallery, and `linix rollback <id>` \
                     restores one by name without asking."
                        .to_string(),
                ));
            }
        }

        // Said before the gallery, not after the confirmation: a list of restore points is
        // read as an offer to restore one.
        if let Some(cap) = self.snapshot_manager.restore_capability() {
            if !cap.is_live() {
                let name = self
                    .snapshot_manager
                    .provider_name()
                    .unwrap_or("this provider");
                println!("\n{}", cap.describe(name));
                println!("The snapshots below can be inspected here, and put back elsewhere.");
            }
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

    async fn validate_snapshot_path(&self, path: &Path, snapshot_backend: &str) -> Result<PathBuf> {
        let path_owned = path.to_path_buf();
        let canonical = tokio::task::spawn_blocking(move || path_owned.canonicalize())
            .await
            .map_err(|e| Error::Other(e.to_string()))?
            .map_err(|e| {
                Error::Snapshot(format!("Failed to canonicalize path {:?}: {}", path, e))
            })?;

        let path_str = canonical.to_string_lossy();

        for forbidden in REGISTRY_READ_FORBIDDEN_PATHS {
            if path_str.contains(forbidden) {
                return Err(Error::Refused(format!(
                    "refusing to read a snapshot registry from '{}': that path is not a place a \
                     LiNix registry can legitimately live, and reading it as JSON is a way to \
                     turn `snapshot restore` into an arbitrary-file reader",
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
        let diff = Self::calculate_diff(&current_state, &snapshot_state);

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

        // The package list above is a SUMMARY, not the scope. A snapshot restore rolls the
        // entire filesystem back — every file, not just managed packages: configs you edited,
        // data you wrote, and anything else that changed since the snapshot are all reverted
        // too (S8). Say so plainly before asking, so "RESTORE" is informed consent.
        warn!(
            "\nCRITICAL: this does NOT just revert the packages listed above. It rolls your \
             ENTIRE filesystem (/) back to the snapshot — every file changed since then, \
             including configs and data, is reverted. There is no partial restore."
        );
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

    /// The package-level diff shown before a restore: what rolling back to `past` would add,
    /// remove, or change versions of, relative to `current`. Pure (no `self`) so it is unit-
    /// tested directly — the summary a user reads before confirming a whole-filesystem restore
    /// must be right.
    fn calculate_diff(current: &StateRegistry, past: &StateRegistry) -> StateDiff {
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
        info!("restoring the filesystem via {}", snapshot.backend);

        self.snapshot_manager.restore(&snapshot.id).await?;

        // The filesystem snapshot is a whole-`/` restore, so it already reverts your manifests
        // and `registry.json` along with everything else — there is no separate generation to
        // pair with anymore (the generation format was deleted; git is the manifest history).
        println!(
            "\nRestored from {}. Reboot to run the restored system.",
            snapshot.id
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn reg(pkgs: &[(&str, &str, &str)]) -> StateRegistry {
        let mut r = StateRegistry::default();
        for (backend, name, version) in pkgs {
            r.add(
                backend,
                name,
                Some(version.to_string()),
                HashMap::new(),
                Some("imperative".into()),
                false,
            );
        }
        r
    }

    #[test]
    fn diff_reports_added_removed_and_version_changed() {
        // current: what's installed now; past: the snapshot we'd roll back to.
        let current = reg(&[("apt", "curl", "8.4"), ("cargo", "rg", "14.0")]);
        let past = reg(&[("apt", "curl", "8.2"), ("apt", "nano", "7.0")]);

        let diff = SnapshotRestore::calculate_diff(&current, &past);

        // In current but not past -> rolling back would REMOVE it.
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].name, "rg");
        // In past but not current -> rolling back would RESTORE it.
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].name, "nano");
        // In both, different version -> a version change.
        assert_eq!(diff.changed.len(), 1);
        let (cur, old) = &diff.changed[0];
        assert_eq!(cur.name, "curl");
        assert_eq!(cur.version.as_deref(), Some("8.4"));
        assert_eq!(old.version.as_deref(), Some("8.2"));
    }

    #[test]
    fn identical_states_produce_an_empty_diff() {
        let a = reg(&[("apt", "curl", "8.4")]);
        let b = reg(&[("apt", "curl", "8.4")]);
        let diff = SnapshotRestore::calculate_diff(&a, &b);
        assert!(diff.added.is_empty() && diff.removed.is_empty() && diff.changed.is_empty());
    }
}
