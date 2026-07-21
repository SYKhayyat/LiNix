use crate::core::{Error, Result};
use crate::utils::file::atomic_write;
use crate::utils::safe_data_dir;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, trace};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostMetadata {
    pub backend: String,
    pub options: HashMap<String, String>,
    pub properties: HashMap<String, String>,
    pub requires: Vec<String>,
    pub removed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPackage {
    pub name: String,
    pub backend: String,
    pub version: Option<String>,
    pub installed_at: u64,
    pub expires_at: Option<u64>,
    pub options: HashMap<String, String>,
    pub source: Option<String>,
    pub is_transient: bool,
    pub session_id: Option<String>,
}

// Suspension — the mirror image of a lease.
//
// A lease is a *temporary install*: a package present now that removes itself
// once time is up. A suspension is a *temporary uninstall*: a package removed
// now that reinstalls itself once its timer elapses (`restore_at`) or its owning
// ephemeral shell session ends (`session_id`). The user is always in charge —
// suspensions are only ever created by an explicit `remove --temp`. Version is
// best-effort: recorded if the backend surfaced one, but restore never depends
// on it (reinstall-by-name is enough, per the product decision).

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suspension {
    pub backend: String,
    pub name: String,
    /// Best-effort version captured at suspend time; restore does not require it.
    pub version: Option<String>,
    /// Unix time at which the package should be restored. `None` = session-scoped.
    pub restore_at: Option<u64>,
    /// Ephemeral shell session that owns this suspension; restored on session end.
    pub session_id: Option<String>,
    /// When the suspension was created (Unix seconds).
    pub suspended_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateRegistry {
    /// Skipped by serde, so a deserialized registry carries an empty path until the loader
    /// restores it — saving before that would write to the wrong place.
    #[serde(skip)]
    pub path: PathBuf,
    pub packages: Vec<ManagedPackage>,
    /// Removed packages, kept as a record after uninstall.
    pub ghosts: HashMap<String, GhostMetadata>,
    pub active_session_id: Option<String>,
    /// Packages temporarily uninstalled that are awaiting restoration.
    pub suspensions: Vec<Suspension>,
    /// Packages the user has "held": never auto-upgraded until explicitly unheld. Entries are
    /// `backend:name` or a bare `name` (matching either form).
    pub held: Vec<String>,
}

impl StateRegistry {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            packages: Vec::new(),
            ghosts: HashMap::new(),
            active_session_id: None,
            suspensions: Vec::new(),
            held: Vec::new(),
        }
    }

    /// Hold a package so upgrades never touch it. `key` may be `backend:name` or a bare name.
    /// Returns false if it was already held.
    pub fn hold(&mut self, key: &str) -> bool {
        if self.held.iter().any(|k| k == key) {
            return false;
        }
        self.held.push(key.to_string());
        true
    }

    pub fn unhold(&mut self, key: &str) -> bool {
        let before = self.held.len();
        self.held.retain(|k| k != key);
        self.held.len() != before
    }

    /// True if `backend:name` (or its bare `name`) is currently held.
    pub fn is_held(&self, backend: &str, name: &str) -> bool {
        let qualified = format!("{}:{}", backend, name);
        self.held.iter().any(|k| k == name || k == &qualified)
    }

    pub fn list_held(&self) -> &[String] {
        &self.held
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        debug!(
            "loading state from {:?}",
            path
        );

        if !path.exists() {
            info!(
                "No state file found at {:?}. Initializing empty registry.",
                path
            );
            return Ok(Self::new(path.to_path_buf()));
        }

        let data = std::fs::read_to_string(path)
            .map_err(|e| Error::Io(format!("Registry Read Error at {:?}: {}", path, e)))?;

        if data.trim().is_empty() {
            trace!("State file is empty, returning default.");
            return Ok(Self::new(path.to_path_buf()));
        }

        let mut registry: Self = serde_json::from_str(&data).map_err(|e| {
            Error::Other(format!(
                "the state registry at {} cannot be read: {}\n  \
                 It records what LiNix believes it manages. A missing or unreadable one is \
                 not something to guess at — move it aside and run `linix adopt` to rebuild \
                 it from what is actually installed.",
                path.display(),
                e
            ))
        })?;

        // `path` is #[serde(skip)], so it comes back empty and must be restored here or the
        // next save writes to "".
        registry.path = path.to_path_buf();

        debug!(
            "Successfully loaded {} managed packages and {} ghosts.",
            registry.packages.len(),
            registry.ghosts.len()
        );
        Ok(registry)
    }

    pub fn load_default() -> Result<Self> {
        let default_path = safe_data_dir().join("registry.json");
        Self::load_from(&default_path)
    }

    pub fn save(&self) -> Result<()> {
        trace!("saving state to {:?}", self.path);

        if let Some(parent) = self.path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Error::Io(format!("Failed to create registry directory: {}", e))
                })?;
            }
        }

        let data = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Other(format!("State Serialization Error: {}", e)))?;

        atomic_write(&self.path, &data)
            .map_err(|e| Error::Persist(format!("Atomic write failed for state registry: {}", e)))
    }

    pub fn add(
        &mut self,
        backend: &str,
        name: &str,
        version: Option<String>,
        options: HashMap<String, String>,
        source: Option<String>,
        is_transient: bool,
    ) {
        // Deliberately does NOT read `@lease` / `@duration`. II.16 retired them: a lease is
        // a dated line now (`@expires=<absolute>`), which the resolver reads and sync acts
        // on. Reading them here made a key the grammar now refuses into a real expiry — a
        // package that uninstalls itself, on a path the guard does not see (S19, C3).
        let expires_at = None;

        let session_id = if is_transient {
            self.active_session_id.clone()
        } else {
            None
        };

        self.packages
            .retain(|p| !(p.backend == backend && p.name == name));
        let ghost_key = format!("{}:{}", backend, name);
        self.ghosts.remove(&ghost_key);

        let new_pkg = ManagedPackage {
            name: name.to_string(),
            backend: backend.to_string(),
            version,
            installed_at: Self::now(),
            expires_at,
            options,
            source,
            is_transient,
            session_id,
        };

        trace!(
            "Finalizing addition of {}:{} (Source: {:?}, Transient: {})",
            backend,
            name,
            new_pkg.source,
            is_transient
        );

        self.packages.push(new_pkg);
        debug!(
            "Package {}:{} is now under management.",
            backend, name
        );
    }

    pub fn add_simple(&mut self, backend: &str, name: &str, version: Option<String>) {
        self.add(backend, name, version, HashMap::new(), None, false);
    }

    pub fn remove(&mut self, backend: &str, name: &str) {
        if let Some(pos) = self
            .packages
            .iter()
            .position(|p| p.backend == backend && p.name == name)
        {
            let pkg = self.packages.remove(pos);

            let ghost_key = format!("{}:{}", backend, name);
            self.ghosts.insert(
                ghost_key,
                GhostMetadata {
                    backend: backend.to_string(),
                    options: pkg.options,
                    properties: HashMap::new(),
                    requires: Vec::new(),
                    removed_at: Self::now(),
                },
            );
            debug!(
                "Package {}:{} archived as Ghost.",
                backend, name
            );
        } else {
            trace!(
                "Requested removal of {}:{} but it was not managed.",
                backend,
                name
            );
        }
    }

    /// Drops a package from management WITHOUT recording a removal. Returns true if it was
    /// managed.
    ///
    /// Distinct from `remove`, which archives a Ghost stamped `removed_at` — that is the
    /// right record when a package was actually uninstalled, and a false one here: after
    /// `linix unmanage` the package is still installed and LiNix has merely stopped
    /// claiming it. Writing a ghost would tell every later reader it was deleted.
    pub fn forget(&mut self, backend: &str, name: &str) -> bool {
        if let Some(pos) = self
            .packages
            .iter()
            .position(|p| p.backend == backend && p.name == name)
        {
            self.packages.remove(pos);
            debug!(
                "Package {}:{} forgotten (left installed).",
                backend, name
            );
            true
        } else {
            false
        }
    }

    pub fn get_expired_packages(&self) -> Vec<(String, String)> {
        let now = Self::now();
        self.packages
            .iter()
            .filter(|p| p.expires_at.is_some_and(|expiry| now >= expiry))
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect()
    }

    pub fn get_transient_packages(&self, session_id: &str) -> Vec<(String, String)> {
        trace!(
            "Scanning for transient packages in session '{}'",
            session_id
        );
        self.packages
            .iter()
            .filter(|p| p.is_transient && p.session_id.as_deref() == Some(session_id))
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect()
    }

    /// Records a temporary uninstall (the mirror of a lease). `duration` is optional:
    /// `Some("2h")` schedules a timed restore, `None` ties the restore to the currently
    /// active shell session (restored when that shell exits). Returns the resolved
    /// restore timestamp, or an error if the duration string is malformed.
    pub fn suspend(
        &mut self,
        backend: &str,
        name: &str,
        version: Option<String>,
        duration: Option<&str>,
    ) -> Result<Option<u64>> {
        let restore_at = match duration {
            Some(d) => Some(Self::parse_duration(d).ok_or_else(|| {
                Error::Validation(format!(
                    "Invalid duration format: '{}'. Use 30d, 2h, 15m, etc.",
                    d
                ))
            })?),
            None => None,
        };

        // A session-scoped suspension (no duration) needs an active session to belong to.
        let session_id = if restore_at.is_none() {
            self.active_session_id.clone()
        } else {
            None
        };

        // Replace any prior suspension for the same target so re-suspending refreshes it
        // rather than stacking duplicate restores.
        self.suspensions
            .retain(|s| !(s.backend == backend && s.name == name));
        self.suspensions.push(Suspension {
            backend: backend.to_string(),
            name: name.to_string(),
            version,
            restore_at,
            session_id,
            suspended_at: Self::now(),
        });
        Ok(restore_at)
    }

    /// Returns suspensions whose timed restore is now due (`restore_at <= now`).
    /// Session-scoped suspensions (no `restore_at`) are never returned here.
    pub fn get_due_suspensions(&self) -> Vec<Suspension> {
        let now = Self::now();
        self.suspensions
            .iter()
            .filter(|s| s.restore_at.is_some_and(|at| now >= at))
            .cloned()
            .collect()
    }

    pub fn get_session_suspensions(&self, session_id: &str) -> Vec<Suspension> {
        self.suspensions
            .iter()
            .filter(|s| s.session_id.as_deref() == Some(session_id))
            .cloned()
            .collect()
    }

    /// Drops a suspension record (called once its package has been restored, or the
    /// restore was abandoned). Returns true if a matching record was removed.
    pub fn clear_suspension(&mut self, backend: &str, name: &str) -> bool {
        let before = self.suspensions.len();
        self.suspensions
            .retain(|s| !(s.backend == backend && s.name == name));
        self.suspensions.len() != before
    }

    pub fn list_suspensions(&self) -> &[Suspension] {
        &self.suspensions
    }

    pub fn is_managed(&self, backend: &str, name: &str) -> bool {
        self.packages
            .iter()
            .any(|p| p.backend == backend && p.name == name)
    }

    pub fn get_package(&self, backend: &str, name: &str) -> Option<&ManagedPackage> {
        self.packages
            .iter()
            .find(|p| p.backend == backend && p.name == name)
    }

    /// Public so callers can validate a user-supplied duration up front (a malformed
    /// `--temp` must fail loudly, never silently degrade into a permanent action).
    pub fn parse_duration(duration_str: &str) -> Option<u64> {
        if duration_str.is_empty() {
            return None;
        }
        let unit = duration_str.chars().last()?;
        let val_part = &duration_str[..duration_str.len() - 1];
        let value: u64 = val_part.parse().ok()?;
        let seconds = match unit {
            's' => value,
            'm' => value * 60,
            'h' => value * 3600,
            'd' => value * 86400,
            _ => return None,
        };
        Some(Self::now() + seconds)
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

impl Default for StateRegistry {
    fn default() -> Self {
        let default_path = safe_data_dir().join("registry.json");
        Self::new(default_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> StateRegistry {
        StateRegistry::new(PathBuf::from("/tmp/linix-test-registry.json"))
    }

    #[test]
    fn timed_suspension_sets_future_restore_and_no_session() {
        let mut r = reg();
        let at = r
            .suspend("apt", "htop", Some("3.0".into()), Some("1h"))
            .unwrap();
        assert!(at.is_some());
        assert!(at.unwrap() > StateRegistry::now());
        let s = &r.list_suspensions()[0];
        assert_eq!(s.backend, "apt");
        assert_eq!(s.name, "htop");
        assert_eq!(s.version.as_deref(), Some("3.0"));
        assert!(s.session_id.is_none());
    }

    #[test]
    fn session_scoped_suspension_binds_active_session() {
        let mut r = reg();
        r.active_session_id = Some("shell-abc".into());
        let at = r.suspend("dnf", "nano", None, None).unwrap();
        assert!(
            at.is_none(),
            "session-scoped suspension has no timed restore"
        );
        let s = &r.list_suspensions()[0];
        assert_eq!(s.session_id.as_deref(), Some("shell-abc"));
        assert_eq!(r.get_session_suspensions("shell-abc").len(), 1);
        assert_eq!(r.get_session_suspensions("other").len(), 0);
    }

    #[test]
    fn invalid_duration_is_rejected() {
        let mut r = reg();
        assert!(r.suspend("apt", "htop", None, Some("banana")).is_err());
        assert!(r.list_suspensions().is_empty());
    }

    #[test]
    fn resuspending_replaces_prior_record() {
        let mut r = reg();
        r.suspend("apt", "htop", None, Some("1h")).unwrap();
        r.suspend("apt", "htop", None, Some("2h")).unwrap();
        assert_eq!(r.list_suspensions().len(), 1, "no duplicate stacking");
    }

    #[test]
    fn due_suspensions_are_only_past_deadline() {
        let mut r = reg();
        r.suspensions.push(Suspension {
            backend: "apt".into(),
            name: "due".into(),
            version: None,
            restore_at: Some(StateRegistry::now().saturating_sub(10)),
            session_id: None,
            suspended_at: StateRegistry::now(),
        });
        r.suspend("apt", "later", None, Some("30d")).unwrap();
        let due = r.get_due_suspensions();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].name, "due");
    }

    #[test]
    fn session_scoped_never_appears_in_timed_sweep() {
        let mut r = reg();
        r.active_session_id = Some("s1".into());
        r.suspend("apt", "nano", None, None).unwrap();
        assert!(r.get_due_suspensions().is_empty());
    }

    #[test]
    fn clear_suspension_removes_only_the_target() {
        let mut r = reg();
        r.suspend("apt", "a", None, Some("1h")).unwrap();
        r.suspend("apt", "b", None, Some("1h")).unwrap();
        assert!(r.clear_suspension("apt", "a"));
        assert!(!r.clear_suspension("apt", "a"), "already gone");
        assert_eq!(r.list_suspensions().len(), 1);
        assert_eq!(r.list_suspensions()[0].name, "b");
    }

    #[test]
    fn a_registry_missing_a_field_is_refused_rather_than_filled_in() {
        // There is no old-format reader. A registry with no `suspensions` key is one this
        // build did not write, and the honest answer is to say so — filling it with an empty
        // list says "nothing is suspended", which is a claim about the machine nobody checked.
        let missing = r#"{"packages":[],"ghosts":{},"active_session_id":null}"#;
        assert!(serde_json::from_str::<StateRegistry>(missing).is_err());
    }

    #[test]
    fn hold_matches_qualified_and_bare_names() {
        let mut r = StateRegistry::new(PathBuf::from("/tmp/x"));
        assert!(r.hold("cargo:ripgrep"));
        assert!(!r.hold("cargo:ripgrep"), "no duplicate");
        assert!(r.hold("curl"));

        assert!(r.is_held("cargo", "ripgrep"));
        assert!(r.is_held("apt", "curl")); // bare-name match across any backend
        assert!(!r.is_held("cargo", "bat"));

        assert!(r.unhold("cargo:ripgrep"));
        assert!(!r.is_held("cargo", "ripgrep"));
        assert!(!r.unhold("cargo:ripgrep"), "already gone");
        assert_eq!(r.list_held(), &["curl".to_string()]);
    }
}
