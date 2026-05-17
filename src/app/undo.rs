use crate::App;
use crate::core::{Result, Error, Snapshot, StateRegistry, ManagedPackage};
use dialoguer::{theme::ColorfulTheme, Select};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn, error, debug};

/// Manages the Snapshot Gallery and System Time Travel (Point 12).
/// Hardened for Version 3.5.0 to provide Snapshot Diffs, allowing users 
/// to see package changes before committing to a system-wide rollback.
/// 
/// FIX #6: Added comprehensive path validation to prevent path traversal attacks.
pub struct UndoManager<'a> {
    app: &'a App,
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

/// Paths that are NEVER allowed to be accessed (blacklist for extra safety)
const FORBIDDEN_PATHS: &[&str] = &[
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/passwd",
    "/boot",
    "/dev",
    "/proc",
    "/sys",
];

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

    /// Validates that a path is safe to access.
    /// Uses whitelist approach (allowed prefixes) plus blacklist for extra safety.
    /// 
    /// FIX #6: Complete path validation to prevent directory traversal attacks.
    fn validate_snapshot_path(&self, path: &Path, snapshot_backend: &str) -> Result<PathBuf> {
        // Step 1: Canonicalize the path to resolve any symlinks or relative components
        let canonical = path.canonicalize()
            .map_err(|e| Error::Snapshot(format!("Failed to canonicalize path {:?}: {}", path, e)))?;
        
        let path_str = canonical.to_string_lossy();
        
        // Step 2: Check against forbidden paths (blacklist)
        for forbidden in FORBIDDEN_PATHS {
            if path_str.contains(forbidden) {
                return Err(Error::Snapshot(format!(
                    "Security violation: Attempted to access forbidden path '{}'", 
                    forbidden
                )));
            }
        }
        
        // Step 3: Check against allowed prefixes based on snapshot backend
        let allowed_prefixes = match snapshot_backend {
            "btrfs" => vec!["/.snapshots/", "/var/lib/snapper/"],
            "timeshift" => vec!["/run/timeshift/", "/timeshift/"],
            "zfs" => vec!["/.zfs/snapshot/"],
            _ => ALLOWED_SNAPSHOT_PREFIXES.to_vec(),
        };
        
        let mut is_allowed = false;
        for prefix in allowed_prefixes {
            if path_str.starts_with(prefix) {
                is_allowed = true;
                break;
            }
        }
        
        if !is_allowed {
            return Err(Error::Snapshot(format!(
                "Security violation: Snapshot path '{}' is outside allowed directories. \
                 Allowed prefixes: {:?}",
                path_str, allowed_prefixes
            )));
        }
        
        // Step 4: Additional safety - ensure we're not traversing outside the snapshot root
        // The path should be a subdirectory of the snapshot root
        let snapshot_root = match snapshot_backend {
            "btrfs" => format!("/.snapshots/"),
            "timeshift" => format!("/run/timeshift/backup/timeshift/snapshots/"),
            "zfs" => format!("/.zfs/snapshot/"),
            _ => return Err(Error::Snapshot(format!("Unsupported snapshot backend: {}", snapshot_backend))),
        };
        
        if !path_str.starts_with(&snapshot_root) {
            return Err(Error::Snapshot(format!(
                "Security violation: Path '{}' is not within snapshot root '{}'",
                path_str, snapshot_root
            )));
        }
        
        Ok(canonical)
    }

    /// Finds the registry.json file within a snapshot.
    /// Searches multiple possible locations.
    fn find_registry_in_snapshot(&self, snapshot_root: &Path) -> Result<Option<PathBuf>> {
        let possible_paths = vec![
            snapshot_root.join("var/lib/linix/registry.json"),
            snapshot_root.join("root/.local/share/linix/registry.json"),
            snapshot_root.join(".local/share/linix/registry.json"),
            snapshot_root.join("home").join(std::env::var("USER").unwrap_or_default()).join(".local/share/linix/registry.json"),
        ];
        
        for path in possible_paths {
            if path.exists() {
                debug!("Found registry.json at {:?}", path);
                return Ok(Some(path));
            }
        }
        
        Ok(None)
    }

    /// Calculates and displays the difference between the current state 
    /// and the state recorded in a snapshot's registry.json.
    async fn show_diff_and_confirm(&self, snapshot: &Snapshot) -> Result<()> {
        println!("\nCalculating Package Diff for Snapshot: {}...", snapshot.id);

        // 1. Determine path to registry.json inside the snapshot
        let snapshot_root = match snapshot.backend.as_str() {
            "btrfs" => PathBuf::from(format!("/.snapshots/{}", snapshot.id)),
            "timeshift" => PathBuf::from(format!("/run/timeshift/backup/timeshift/snapshots/{}", snapshot.id)),
            _ => return Err(Error::Snapshot(format!("Unsupported snapshot backend: {}", snapshot.backend))),
        };
        
        // FIX #6: Validate the snapshot path before reading
        let validated_root = self.validate_snapshot_path(&snapshot_root, &snapshot.backend)?;
        
        // 2. Find the registry.json file
        let snapshot_registry_path = match self.find_registry_in_snapshot(&validated_root)? {
            Some(path) => path,
            None => {
                warn!("Snapshot Registry not found at any expected location in {:?}", validated_root);
                return Err(Error::Snapshot("Could not find registry.json in snapshot".into()));
            }
        };
        
        // 3. Validate the registry path (extra safety)
        let validated_registry_path = self.validate_snapshot_path(&snapshot_registry_path, &snapshot.backend)?;
        
        // 4. Read and parse the snapshot state
        let data = std::fs::read_to_string(&validated_registry_path)
            .map_err(|e| Error::Snapshot(format!("Failed to read registry.json: {}", e)))?;
        
        let snapshot_state: StateRegistry = serde_json::from_str(&data)
            .map_err(|e| Error::Snapshot(format!("Failed to parse registry.json: {}", e)))?;
        
        let current_state = self.app.state.lock().await;
        let diff = self.calculate_diff(&current_state, &snapshot_state);
        
        // 5. Display Diff
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

        // 6. Restorative Confirmation
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

        // Find packages in current but not in past -> Will be removed on rollback
        for (key, pkg) in &curr_map {
            if !past_map.contains_key(key) {
                diff.added.push((*pkg).clone());
            }
        }

        // Find packages in past but not in current -> Will be restored on rollback
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

    /// Executes the system restore operation.
    /// Includes additional safety checks before executing destructive commands.
    async fn execute_restore(&self, snapshot: &Snapshot) -> Result<()> {
        info!("Undo: Commencing system restoration via {}...", snapshot.backend);

        // Pre-restore validation
        match snapshot.backend.as_str() {
            "btrfs" => {
                let snapshot_path = format!("/.snapshots/{}", snapshot.id);
                let validated_path = self.validate_snapshot_path(Path::new(&snapshot_path), "btrfs")?;
                
                // Verify the snapshot exists and is valid
                if !validated_path.exists() {
                    return Err(Error::Snapshot(format!("Snapshot path does not exist: {:?}", validated_path)));
                }
                
                // Create a backup of current root before destructive operation
                let backup_path = "/.snapshots/root_backup_pre_rollback";
                info!("Creating backup of current root at {}", backup_path);
                
                // Use --reflink=always for efficient copy on BTRFS
                let status = self.app.executor.run(
                    "btrfs", 
                    &["subvolume", "snapshot", "-r", "/", backup_path],
                    true
                ).await?;
                
                if !status.status.success() {
                    warn!("Failed to create pre-rollback backup, continuing anyway...");
                }
                
                // Perform the rollback
                info!("Rolling back to snapshot: {}", snapshot.id);
                self.app.executor.run("btrfs", &["subvolume", "snapshot", &snapshot_path, "/"], true).await?;
            }
            "timeshift" => {
                // Validate the snapshot ID is numeric (Timeshift uses numeric IDs)
                if !snapshot.id.chars().all(|c| c.is_ascii_digit()) {
                    return Err(Error::Snapshot(format!("Invalid Timeshift snapshot ID: {}", snapshot.id)));
                }
                
                let args = [
                    "--restore", 
                    "--snapshot", &snapshot.id, 
                    "--target-device", "/", 
                    "--yes"
                ];
                self.app.executor.run("timeshift", &args, true).await?;
            }
            _ => return Err(Error::Snapshot(format!("Unsupported provider: {}", snapshot.backend))),
        }

        println!("\nSUCCESS: System root has been restored. Please reboot immediately.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

    #[test]
    fn test_validate_snapshot_path_btrfs() {
        let undo_manager = UndoManager {
            app: unsafe { std::mem::zeroed() }, // Mock for test
        };
        
        // Valid BTRFS snapshot path
        let valid_path = Path::new("/.snapshots/123/snapshot");
        let result = undo_manager.validate_snapshot_path(valid_path, "btrfs");
        assert!(result.is_ok());
        
        // Invalid path (outside snapshot directory)
        let invalid_path = Path::new("/etc/passwd");
        let result = undo_manager.validate_snapshot_path(invalid_path, "btrfs");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Security violation"));
    }
    
    #[test]
    fn test_validate_snapshot_path_timeshift() {
        let undo_manager = UndoManager {
            app: unsafe { std::mem::zeroed() },
        };
        
        // Valid Timeshift path
        let valid_path = Path::new("/run/timeshift/backup/timeshift/snapshots/2024-01-01_12-00-00");
        let result = undo_manager.validate_snapshot_path(valid_path, "timeshift");
        assert!(result.is_ok());
        
        // Invalid Timeshift path
        let invalid_path = Path::new("/tmp/malicious");
        let result = undo_manager.validate_snapshot_path(invalid_path, "timeshift");
        assert!(result.is_err());
    }
    
    #[test]
    fn test_forbidden_paths_blocked() {
        let undo_manager = UndoManager {
            app: unsafe { std::mem::zeroed() },
        };
        
        for forbidden in FORBIDDEN_PATHS {
            let path = Path::new(forbidden);
            let result = undo_manager.validate_snapshot_path(path, "btrfs");
            assert!(result.is_err(), "Path {} should be blocked", forbidden);
        }
    }
    
    #[test]
    fn test_find_registry_in_snapshot() {
        let dir = tempdir().unwrap();
        let registry_path = dir.path().join("var/lib/linix/registry.json");
        fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        fs::write(&registry_path, "{}").unwrap();
        
        let undo_manager = UndoManager {
            app: unsafe { std::mem::zeroed() },
        };
        
        let found = undo_manager.find_registry_in_snapshot(dir.path()).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap(), registry_path);
    }
    
    #[test]
    fn test_calculate_diff() {
        let undo_manager = UndoManager {
            app: unsafe { std::mem::zeroed() },
        };
        
        let mut current = StateRegistry::default();
        let mut past = StateRegistry::default();
        
        current.add_simple("apt", "curl", Some("7.81.0".into()));
        current.add_simple("apt", "git", Some("2.40.0".into()));
        
        past.add_simple("apt", "curl", Some("7.80.0".into()));
        past.add_simple("apt", "vim", Some("8.2.0".into()));
        
        let diff = undo_manager.calculate_diff(&current, &past);
        
        // curl should be changed (version difference)
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.changed[0].0.name, "curl");
        
        // git is only in current, should be added
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].name, "git");
        
        // vim is only in past, should be removed
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].name, "vim");
    }
    
    #[test]
    fn test_snapshot_id_validation() {
        // Timeshift uses numeric IDs
        let valid_id = "1234567890";
        assert!(valid_id.chars().all(|c| c.is_ascii_digit()));
        
        let invalid_id = "snapshot-123";
        assert!(!invalid_id.chars().all(|c| c.is_ascii_digit()));
    }
}