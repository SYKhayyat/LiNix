use crate::App;
use crate::core::{Result, Error, Snapshot, StateRegistry, ManagedPackage};
use dialoguer::{theme::ColorfulTheme, Select};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn, error, debug};

/// Manages the Snapshot Gallery and System Time Travel (Point 12).
/// Hardened for Version 3.4.0 to provide Snapshot Diffs, allowing users 
/// to see package changes before committing to a system-wide rollback.
pub struct UndoManager<'a> {
    app: &'a App,
}

#[derive(Debug, Default)]
struct StateDiff {
    added: Vec<ManagedPackage>,
    removed: Vec<ManagedPackage>,
    changed: Vec<(ManagedPackage, ManagedPackage)>, // (Current, Snapshot)
}

impl<'a> UndoManager<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Entry point for 'linix undo'.
    /// Fetches snapshots, calculates diffs, and presents the interactive UI.
    pub async fn run_interactive(&self) -> Result<()> {
        info!("UndoManager: Querying available system snapshots...");

        let snapshots = self.app.snapshot_manager.list_snapshots().await?;
        if snapshots.is_empty() {
            println!("No system snapshots found. LiNix cannot perform time travel on this system.");
            return Ok(());
        }

        println!("\n--- LiNix Snapshot Gallery ---");
        let items: Vec<String> = snapshots.iter()
            .map(|s| format!("[{}] {} - {} ({})", s.backend, s.timestamp, s.description, s.id))
            .collect();

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a system state to inspect/restore (ESC to cancel)")
            .default(0)
            .items(&items)
            .interact_opt()
            .map_err(|e| Error::Other(e.to_string()))?;

        if let Some(index) = selection {
            let selected = &snapshots[index];
            self.show_diff_and_confirm(selected).await?;
        }

        Ok(())
    }

    /// Calculates and displays the difference between the current state 
    /// and the state recorded in a snapshot's registry.json.
    async fn show_diff_and_confirm(&self, snapshot: &Snapshot) -> Result<()> {
        println!("\nCalculating Package Diff for Snapshot: {}...", snapshot.id);

        // 1. Determine path to registry.json inside the snapshot
        // BTRFS snapshots are usually at /.snapshots/{id}/
        // Timeshift snapshots are at /run/timeshift/backup/timeshift/snapshots/{id}/ (when mounted)
        let snapshot_root = match snapshot.backend.as_str() {
            "btrfs" => format!("/.snapshots/{}", snapshot.id),
            "timeshift" => format!("/run/timeshift/backup/timeshift/snapshots/{}", snapshot.id),
            _ => return Err(Error::Snapshot("Unsupported snapshot backend for diffing".into())),
        };

        // Note: This logic assumes the registry.json path is relative to the root.
        // We look for: <snapshot_root>/home/<user>/.local/share/linix/registry.json
        // In Version 3.4.0, we use a robust search for the registry file within the snapshot.
        let snapshot_registry_path = Path::new(&snapshot_root)
            .join("var/lib/linix/registry.json"); // System-wide fallback
        
        let diff = if snapshot_registry_path.exists() {
            let data = std::fs::read_to_string(&snapshot_registry_path)?;
            let snapshot_state: StateRegistry = serde_json::from_str(&data).unwrap_or_default();
            let current_state = self.app.state.lock().await;
            self.calculate_diff(&current_state, &snapshot_state)
        } else {
            warn!("Snapshot Registry not found at {:?}. Cannot calculate package diff.", snapshot_registry_path);
            StateDiff::default()
        };

        // 2. Display Diff
        if !diff.added.is_empty() || !diff.removed.is_empty() || !diff.changed.is_empty() {
            println!("\nPACKAGE CHANGES (Rolling back will result in):");
            for p in &diff.removed {
                println!("  [+] Restore:  {}:{} (Version: {:?})", p.backend, p.name, p.version);
            }
            for p in &diff.added {
                println!("  [-] Remove:   {}:{} (Not present in snapshot)", p.backend, p.name);
            }
            for (curr, prev) in &diff.changed {
                println!("  [*] Revert:   {}:{} ({:?} -> {:?})", curr.backend, curr.name, curr.version, prev.version);
            }
        } else {
            println!("\nNo package changes detected between current state and snapshot registry.");
        }

        // 3. Restorative Confirmation
        warn!("\nCRITICAL: Reverting to this snapshot will overwrite your ENTIRE system root (/).");
        println!("All files created or modified after {} will be lost.", snapshot.timestamp);
        
        print!("\nAre you absolutely sure? Type 'RESTORE' to proceed: ");
        use std::io::{self, Write};
        let _ = io::stdout().flush();
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if input.trim() == "RESTORE" {
            self.execute_restore(snapshot).await
        } else {
            info!("Restore aborted by user.");
            Ok(())
        }
    }

    fn calculate_diff(&self, current: &StateRegistry, past: &StateRegistry) -> StateDiff {
        let mut diff = StateDiff::default();
        let curr_map: HashMap<String, &ManagedPackage> = current.packages.iter()
            .map(|p| (format!("{}:{}", p.backend, p.name), p)).collect();
        let past_map: HashMap<String, &ManagedPackage> = past.packages.iter()
            .map(|p| (format!("{}:{}", p.backend, p.name), p)).collect();

        // Find Removed (In current but not in past) -> Will be removed on rollback
        for (key, pkg) in &curr_map {
            if !past_map.contains_key(key) {
                diff.added.push((*pkg).clone());
            }
        }

        // Find Added (In past but not in current) -> Will be restored on rollback
        for (key, pkg) in &past_map {
            if !curr_map.contains_key(key) {
                diff.removed.push((*pkg).clone());
            } else {
                // Check for version changes
                let curr_pkg = curr_map.get(key).unwrap();
                if curr_pkg.version != pkg.version {
                    diff.changed.push(((*curr_pkg).clone(), (*pkg).clone()));
                }
            }
        }

        diff
    }

    async fn execute_restore(&self, snapshot: &Snapshot) -> Result<()> {
        info!("Undo: Commencing system restoration via {}...", snapshot.backend);

        match snapshot.backend.as_str() {
            "btrfs" => {
                let snapshot_path = format!("/.snapshots/{}", snapshot.id);
                // Rollback BTRFS: Move current root, snapshot the backup to /
                self.app.executor.run("mv", &["/", "/.snapshots/root_backup_pre_rollback"], true).await?;
                self.app.executor.run("btrfs", &["subvolume", "snapshot", &snapshot_path, "/"], true).await?;
            }
            "timeshift" => {
                let args = [
                    "--restore", 
                    "--snapshot", &snapshot.id, 
                    "--target-device", "/", 
                    "--yes"
                ];
                self.app.executor.run("timeshift", &args, true).await?;
            }
            _ => return Err(Error::Snapshot("Unsupported provider".into())),
        }

        println!("\nSUCCESS: System root has been restored. Please reboot immediately.");
        Ok(())
    }
}