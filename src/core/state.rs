use crate::core::{Error, Result};
use crate::utils::file::persist;
use crate::utils::safe_data_dir;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, trace};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPackage {
    pub name: String,
    pub backend: String,
    pub version: Option<String>,
    pub installed_at: u64,
    pub expires_at: Option<u64>,
    /// The declaration's options as the grammar produced them.
    ///
    /// **One type from the line to the record.** It was `HashMap<String, String>` here too, so
    /// `PackageSpec`'s options had to be flattened on the way in — and a flatten is the `;`-join
    /// this change exists to remove, moved one function along. `registry.json` therefore stores
    /// `{"k": ["v"]}` rather than `{"k": "v"}`; nothing reads the old shape, and under NO LEGACY
    /// nothing is written to read it.
    pub options: crate::config::grammar::Options,
    /// Why this row exists — a `file:line` a user wrote, or the verb that took ownership
    /// (`adopt`, `imperative`, `plan`, `sync`). Not optional: a managed row Shall cannot
    /// attribute is one II.7 will remove with no answer to `why`, and that is exactly the
    /// shape the dependency expander wrote before Y9 (V.115a).
    pub source: String,
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

/// `registry.json` stores the managed packages as a JSON array, and it still does.
///
/// The in-memory shape is a map because every question asked of it is a `(backend, name)`
/// lookup; the file's shape is an array because that is what is on every machine that has run
/// Shall. **A record format is not a thing to change for an in-memory decision** — a reader
/// that met the other shape would have to be written, and NO LEGACY means the honest way to
/// have two shapes is to have the conversion here, once, and one format on disk.
mod packages_as_list {
    use super::ManagedPackage;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(
        packages: &BTreeMap<(String, String), ManagedPackage>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        packages.values().collect::<Vec<_>>().serialize(s)
    }

    /// Last row wins for a duplicated `(backend, name)`, which is what the `Vec` did: `add`
    /// dropped every earlier row for the same key before pushing.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<(String, String), ManagedPackage>, D::Error> {
        Ok(Vec::<ManagedPackage>::deserialize(d)?
            .into_iter()
            .map(|p| ((p.backend.clone(), p.name.clone()), p))
            .collect())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateRegistry {
    /// Skipped by serde, so a deserialized registry carries an empty path until the loader
    /// restores it — saving before that would write to the wrong place.
    #[serde(skip)]
    pub path: PathBuf,
    /// Keyed by `(backend, name)`, which is the identity every question here asks about.
    ///
    /// **A `Vec` made the two commonest operations linear, and one of them reallocating.**
    /// `is_managed` scanned, and `add` was a full `retain` — an `O(n)` scan *and* a rewrite of
    /// the whole vector per package added — so `declare`'s loop adopting N packages into a
    /// registry of M was `O(N·M)` twice over. `managed_index()` was written to treat the scan
    /// at the two worst sites and reached two of seven; the map removes the class instead, and
    /// `reconcile_ownership` on the default sync path is the site it was never spread to.
    ///
    /// Serialised as the same JSON array it always was — see [`packages_as_list`]. The file
    /// on every machine that has run Shall is not a thing to rewrite for an in-memory shape.
    #[serde(with = "packages_as_list")]
    packages: BTreeMap<(String, String), ManagedPackage>,
    /// Removed packages, kept as a record after uninstall.
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
            packages: BTreeMap::new(),
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
    ///
    /// Compares the two halves in place rather than formatting a key to compare against. This
    /// is asked from inside the planner's fan-out, once per declared package — a `format!`
    /// there was one heap allocation per declaration to answer a question that needs none.
    pub fn is_held(&self, backend: &str, name: &str) -> bool {
        self.held.iter().any(|k| match k.split_once(':') {
            Some((b, n)) => b == backend && n == name,
            None => k == name,
        })
    }

    pub fn list_held(&self) -> &[String] {
        &self.held
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        debug!("loading state from {:?}", path);

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
                 It records what Shall believes it manages. A missing or unreadable one is \
                 not something to guess at — move it aside and run `shall adopt` to rebuild \
                 it from what is actually installed.",
                path.display(),
                e
            ))
        })?;

        // `path` is #[serde(skip)], so it comes back empty and must be restored here or the
        // next save writes to "".
        registry.path = path.to_path_buf();

        debug!(
            "Successfully loaded {} managed packages.",
            registry.packages.len()
        );
        Ok(registry)
    }

    pub fn load_default() -> Result<Self> {
        let default_path = safe_data_dir().join("registry.json");
        Self::load_from(&default_path)
    }

    /// Persist the registry, and report whether the bytes reached the disk.
    ///
    /// Seventeen call sites save this file and three — `adopt`, `hold`, `unhold` — did not ask
    /// whether the run was a preview, so the decision lives in [`persist`] instead of here.
    /// `adopt` was the expensive one: it recorded every discovered package as managed while its
    /// *manifest* write correctly went nowhere, which is the one state the model reads as **the
    /// user deleted every line** — and the next `sync`, the one `shall check` then recommends,
    /// uninstalls them.
    ///
    /// A caller says `Held` or `would hold` from the returned answer rather than by asking the
    /// flag a second time.
    pub fn save(&self) -> Result<bool> {
        self.snapshot()?.write()
    }

    /// The bytes to write, taken while the lock is held, so the writing can happen after it is
    /// released and off the runtime.
    ///
    /// `sync` used to deep-clone the whole registry to hand it to `spawn_blocking` — every
    /// `ManagedPackage` including its `properties: HashMap`, so a few hundred map allocations
    /// to cross a thread boundary with data that was about to be serialised anyway.
    pub fn snapshot(&self) -> Result<StateSnapshot> {
        trace!("serialising state for {:?}", self.path);
        // Compact, not pretty: this is a machine-read registry of every managed package,
        // rewritten in full after every mutation, and `shall` is the only thing that reads it
        // back. Pretty printing roughly doubles the bytes for a file nobody opens.
        let data = serde_json::to_string(self)
            .map_err(|e| Error::Other(format!("State Serialization Error: {}", e)))?;
        Ok(StateSnapshot {
            path: self.path.clone(),
            data,
            packages: self.packages.len(),
            held: self.held.len(),
        })
    }
}

/// A registry serialised and ready to write. See [`StateRegistry::snapshot`].
pub struct StateSnapshot {
    path: PathBuf,
    data: String,
    packages: usize,
    held: usize,
}

impl StateSnapshot {
    pub fn write(&self) -> Result<bool> {
        let written = persist(&self.path, &self.data).map_err(|e| {
            Error::Persist(format!("Atomic write failed for state registry: {}", e))
        })?;
        if !written {
            // The counts are what *would have been* recorded, said in those words. Read as a
            // count of the operation they are the opposite of the truth: `--dry-run unhold`
            // releases the one hold there is, and the old wording then reported "0 hold(s)
            // were not recorded" — a number belonging to the registry, printed as if it were
            // about the release. Same shape as `adopt`'s "listed in the manifest" (R-2).
            crate::would_warn!(
                "{} was not written — it would have recorded {} managed package(s) \
                 and {} hold(s)",
                self.path.display(),
                self.packages,
                self.held
            );
        }
        Ok(written)
    }

    /// Write these bytes on the blocking pool, and give back the same answer.
    ///
    /// **The second half of what `snapshot` is for, and the half most callers skipped.**
    /// `persist` ends in `sync_all` — a physical flush — so a caller that writes on a runtime
    /// worker parks it on the disk (II.52), and eleven of them did it while still holding the
    /// global state mutex, which made every one of the sixty other `state.lock()` sites wait
    /// out that flush. In `leases` and `upgrade` that is a stall injected into the middle of a
    /// concurrent wave. Durability is not what is traded away: the flush still happens, on a
    /// thread that is allowed to sit on it.
    pub async fn write_off_the_runtime(self) -> Result<bool> {
        crate::core::off_the_runtime(move || self.write()).await?
    }
}

/// Serialise the registry under the lock, then write it off the runtime.
///
/// The whole of [`StateRegistry::save`] for a caller that has nothing else to do under the
/// lock. A caller that mutates first takes [`StateRegistry::snapshot`] from its own guard and
/// calls [`StateSnapshot::write_off_the_runtime`] after dropping it.
pub async fn save_off_the_runtime(state: &tokio::sync::Mutex<StateRegistry>) -> Result<bool> {
    let snapshot = state.lock().await.snapshot()?;
    snapshot.write_off_the_runtime().await
}

impl StateRegistry {
    pub fn add(
        &mut self,
        backend: &str,
        name: &str,
        version: Option<String>,
        options: crate::config::grammar::Options,
        source: &str,
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

        let new_pkg = ManagedPackage {
            name: name.to_string(),
            backend: backend.to_string(),
            version,
            installed_at: Self::now(),
            expires_at,
            options,
            source: source.to_string(),
            is_transient,
            session_id,
        };

        trace!(
            "Finalizing addition of {}:{} (Source: {}, Transient: {})",
            backend,
            name,
            new_pkg.source,
            is_transient
        );

        // Replaces any row for the same `(backend, name)`, which is what the `retain` here
        // did, without walking and rewriting the whole registry to do it.
        self.packages
            .insert((backend.to_string(), name.to_string()), new_pkg);
        debug!("Package {}:{} is now under management.", backend, name);
    }

    /// Drop a package from management. `true` if it was managed.
    ///
    /// **This was two functions until the ghosts went.** `remove` archived a `GhostMetadata`
    /// stamped `removed_at` and `forget` did not, and the doc comment on `forget` explained the
    /// difference at length: after `shall unmanage` the package is still installed, so calling
    /// it removed would tell every later reader it was deleted. That distinction was expressed
    /// *only* by the ghost record — which nothing ever read — so with the ghosts deleted the two
    /// bodies were the same six lines under two names, one of them carrying a comment about a
    /// record that no longer existed. Whether a package was uninstalled or merely unmanaged is
    /// not a thing Shall stores, and there is now one function saying so.
    pub fn remove(&mut self, backend: &str, name: &str) -> bool {
        if self
            .packages
            .remove(&(backend.to_string(), name.to_string()))
            .is_some()
        {
            debug!("Package {}:{} is no longer managed.", backend, name);
            true
        } else {
            trace!(
                "Requested removal of {}:{} but it was not managed.",
                backend,
                name
            );
            false
        }
    }

    pub fn get_expired_packages(&self) -> Vec<(String, String)> {
        let now = Self::now();
        self.packages
            .values()
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
            .values()
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

    /// One lookup, not a scan.
    ///
    /// The allocation is the cost of a `BTreeMap` keyed by owned strings; it is one pair of
    /// `String`s against a walk of the whole registry, and the seven sites that asked this in a
    /// loop are why the walk was the thing worth removing. `managed_index()` used to exist to
    /// dodge the scan and reached two of those seven.
    pub fn is_managed(&self, backend: &str, name: &str) -> bool {
        self.get_package(backend, name).is_some()
    }

    pub fn get_package(&self, backend: &str, name: &str) -> Option<&ManagedPackage> {
        self.packages.get(&(backend.to_string(), name.to_string()))
    }

    /// Everything under management, in `(backend, name)` order.
    pub fn managed(&self) -> impl Iterator<Item = &ManagedPackage> {
        self.packages.values()
    }

    /// How many packages are under management.
    pub fn managed_count(&self) -> usize {
        self.packages.len()
    }

    /// One managed row, mutably — for the few callers that edit a field of a record already
    /// under management rather than replacing it.
    pub fn managed_mut(&mut self, backend: &str, name: &str) -> Option<&mut ManagedPackage> {
        self.packages
            .get_mut(&(backend.to_string(), name.to_string()))
    }

    /// Put one already-built record under management, replacing any row for the same key.
    ///
    /// [`StateRegistry::add`] is the door for a real install — it decides the origin, the
    /// session and the timestamps. This is for a caller that already holds the whole record.
    pub fn manage(&mut self, pkg: ManagedPackage) {
        self.packages
            .insert((pkg.backend.clone(), pkg.name.clone()), pkg);
    }

    /// Replace the whole set — for a restore, and for tests that construct a registry.
    pub fn set_managed(&mut self, packages: impl IntoIterator<Item = ManagedPackage>) {
        self.packages = packages
            .into_iter()
            .map(|p| ((p.backend.clone(), p.name.clone()), p))
            .collect();
    }

    /// Public so callers can validate a user-supplied duration up front (a malformed
    /// `--temp` must fail loudly, never silently degrade into a permanent action).
    pub fn parse_duration(duration_str: &str) -> Option<u64> {
        if duration_str.is_empty() {
            return None;
        }
        let unit = duration_str.chars().last()?;
        // Sliced by `len_utf8`, not by one byte: the unit is a `char` and the remainder is a
        // byte range, so a multi-byte final character put the split inside a codepoint and
        // `panic = "abort"` turned a typo on a non-Latin keyboard into a dead process with no
        // message. The refusal below is what the caller is written to report.
        let (val_part, _) = duration_str.split_at(duration_str.len() - unit.len_utf8());
        let value: u64 = val_part.parse().ok()?;
        // Checked, because the multiplication wraps: `[profile.release]` sets no
        // `overflow-checks`, so a duration large enough to overflow produced a restore time in
        // the near past and the package came back at the wrong moment with no error.
        let seconds = match unit {
            's' => value,
            'm' => value.checked_mul(60)?,
            'h' => value.checked_mul(3600)?,
            'd' => value.checked_mul(86400)?,
            _ => return None,
        };
        Self::now().checked_add(seconds)
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
        StateRegistry::new(PathBuf::from("/tmp/shall-test-registry.json"))
    }

    /// A managed row with no origin is the shape the dependency expander wrote (`Y9`,
    /// V.115a): II.7 removes what Shall manages and nobody declared, so an unattributable
    /// row is a package scheduled for deletion with no answer to `why`. The type stops one
    /// being *written*; this stops one being *read back in* — quietly dropping it would
    /// unmanage a package that is still installed, and quietly keeping it is the original bug.
    #[test]
    fn a_row_with_no_origin_is_refused_rather_than_read() {
        // `tempfile`, not a fixed name: `config.rs` already states the rule this used to break —
        // one directory shared by every concurrent run of the suite, with a cleanup that deletes
        // what another run is still reading.
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("registry.json");

        // Serialized from a real registry, so the fixture cannot fail to load for some
        // unrelated missing field and be mistaken for the refusal under test.
        let mut r = reg();
        r.add(
            "apt",
            "libssl3",
            None,
            Default::default(),
            "modules/dev.txt:2",
            false,
        );
        let good = serde_json::to_string(&r).unwrap();
        let bad = good.replace("\"modules/dev.txt:2\"", "null");
        assert_ne!(good, bad, "the fixture must actually lose its origin");

        std::fs::write(&path, &bad).unwrap();
        let err = StateRegistry::load_from(&path)
            .expect_err("a row with no origin must not load")
            .to_string();
        // The message has to be actionable, not just a serde dump: the fix is to rebuild the
        // ledger from the machine, and nothing else in the tree will say so.
        assert!(err.contains("adopt"), "{err}");

        // The same file with its origin loads, so the refusal is about the missing field and
        // not about the shape of the row.
        std::fs::write(&path, &good).unwrap();
        let loaded = StateRegistry::load_from(&path).unwrap();
        assert_eq!(loaded.managed().next().unwrap().source, "modules/dev.txt:2");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every writer supplies an origin, and it survives the round trip. The `add` signature
    /// makes an absent one impossible; this pins that an *empty* one cannot sneak through the
    /// same door, since `""` is a `&str` and the type cannot tell the difference.
    #[test]
    fn every_managed_row_carries_the_origin_it_was_given() {
        let mut r = reg();
        for source in ["modules/dev.txt:14", "adopt", "imperative", "hook:choco"] {
            r.add("apt", "jq", None, Default::default(), source, false);
            let row = r.get_package("apt", "jq").unwrap();
            assert_eq!(row.source, source);
            assert!(!row.source.is_empty());
        }
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
        let missing = r#"{"packages":[],"active_session_id":null}"#;
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
    /// **A malformed `--temp` is refused, and refusing it is not the same as dying.**
    ///
    /// The unit is read as a `char` and the number as a byte range. Taking one byte off the
    /// end landed inside a multi-byte character, and with `panic = "abort"` the process was
    /// gone before `suspend` could turn the `None` into the message it is written to produce.
    /// The oversized cases cover the other half: the multiplication used to wrap, which turned
    /// a far-future restore time into one already past.
    #[test]
    fn a_malformed_duration_is_refused_rather_than_fatal() {
        for bad in [
            "7\u{434}",  // Cyrillic unit, two bytes
            "7\u{4e00}", // three bytes
            "\u{434}",   // no digits at all
            "7x",
            "7",
            "",
            "-1d",
            "99999999999999999d",    // overflows u64 seconds
            "18446744073709551615s", // overflows `now + seconds`
        ] {
            assert_eq!(
                StateRegistry::parse_duration(bad),
                None,
                "`{}` must be refused",
                bad
            );
        }
    }

    #[test]
    fn a_well_formed_duration_is_that_many_seconds_from_now() {
        let now = StateRegistry::now();
        for (spec, secs) in [("30s", 30), ("5m", 300), ("2h", 7200), ("7d", 604_800)] {
            let at = StateRegistry::parse_duration(spec).expect(spec);
            assert!(
                at.abs_diff(now + secs) <= 2,
                "`{}` should be {}s out, got {}",
                spec,
                secs,
                at as i64 - now as i64
            );
        }
    }
}
