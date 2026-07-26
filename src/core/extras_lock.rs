//! The applied-extras ledger (S20): what `sync` has actually put in place, so it can tell
//! when a `repo:` / `shim:` / `service:` / `link:` / `schedule:` line is *removed*.
//!
//! Packages have the registry: LiNix records what it installed, so deleting a package line
//! makes the package drift and `sync` removes it. Extras had no such record — apply was
//! one-way. Delete a `service:nginx` line and nothing disabled the service; delete a `repo:`
//! line and the repository stayed configured. `sync` could not even *detect* the removal,
//! because it had nothing to compare "what is declared now" against.
//!
//! This ledger is that missing half. After each successful apply, `sync` records the set of
//! extra keys it put in place (`locks/extras.toml`). On the next run it diffs the newly
//! declared set against the recorded one; anything recorded-but-no-longer-declared is drift,
//! and gets undone — exactly what removing a package line already does.
//!
//! Pure: the ledger, the key, and the diff. Executing an undo (disabling a service, deleting
//! a shim) is the caller's job.

use crate::config::grammar::Statement;
use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The stable identity of an applied extra, `<kind>:<id>`. Parseable back into an undo action
/// (see `App::reconcile_extras`), and stable across runs so the same declaration always keys
/// the same ledger entry. Returns `None` for statements that are not applied extras (packages,
/// set-math, `use`) — those are tracked elsewhere or not at all.
pub fn extra_key(stmt: &Statement) -> Option<String> {
    match stmt {
        // A link is keyed by its DESTINATION, not by its source — the one place this ledger
        // departs from [`Statement::key`]. The undo has to remove what LiNix wrote, and by the
        // time it runs the declaration is gone, so a key naming the source would hand the
        // teardown the file in your repo and leave the deployed one in place. Keying the
        // destination also makes an edited `@target=` a removal of the old destination and an
        // install of the new, instead of leaving the old one forever.
        Statement::Link(name, opts) => Some(format!(
            "link:{}",
            opts.one("target")
                .and_then(|t| crate::backends::link::resolve_target(t).ok())
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| name.clone())
        )),
        // `exec:` is deliberately NOT an extra. Extras are nouns whose teardown undoes what
        // they put in place; a verb has no such inverse, and a script that succeeds makes its
        // own `when` false — so wiring it into this ledger would re-run or "undo" it every
        // time the condition swung. Its lifecycle is `locks/exec.toml`, not here (XIII.3).
        //
        // A dotfiles tree is excluded for the opposite reason: its files ARE keyed here, but
        // individually by the tree applier — one ledger row per placed file (U22), which this
        // function has no way to enumerate from the declaration alone.
        // `generate:` is excluded for the same reason as `exec:`: it is a verb that runs a
        // command, not a noun with an inverse. Its output declarations ARE nouns and are keyed
        // here individually once merged, but the generate line itself has no teardown.
        Statement::Exec(..) | Statement::Generate(..) | Statement::Dotfiles(..) => None,
        // Everything else with a keyword is a noun with an inverse: deleting a `firewall:` line
        // closes the port (N5), deleting a `service:` line disables the service.
        _ => stmt.kind().map(|_| stmt.key()),
    }
}

/// A drifted extra, split into what it is and what to act on: `service:nginx` → `("service",
/// "nginx")`, `repo:apt:ppa:x/y` → `("repo", "apt:ppa:x/y")`. The kind picks the undo; the
/// rest is that undo's argument.
pub fn split_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(':')
}

/// `locks/extras.toml`: the set of extra keys the last successful sync put in place. A
/// `BTreeSet` so the file is ordered and diffs cleanly in git.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExtrasLedger {
    #[serde(default)]
    applied: BTreeSet<String>,
}

impl ExtrasLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn path_in(locks_dir: &Path) -> PathBuf {
        locks_dir.join("extras.toml")
    }

    /// Load the ledger. A missing file means nothing has been applied yet — the correct
    /// starting state, and never an error.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s)
                .map_err(|e| Error::Toml(format!("reading {}: {}", path.display(), e))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(Error::Io(format!("reading {}: {}", path.display(), e))),
        }
    }

    /// Write the ledger, creating `locks/` if needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| Error::Io(format!("creating {}: {}", dir.display(), e)))?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| Error::Toml(format!("serializing extras ledger: {}", e)))?;
        std::fs::write(path, body)
            .map_err(|e| Error::Io(format!("writing {}: {}", path.display(), e)))
    }

    /// The keys that were applied but are no longer declared — the extras to undo. Sorted
    /// (the set is ordered) so the report and the undo run in a stable order.
    pub fn drift(&self, declared: &BTreeSet<String>) -> Vec<String> {
        self.applied.difference(declared).cloned().collect()
    }

    /// Replace the recorded set with what is declared now. Called after a successful apply, so
    /// the ledger always reflects the last state `sync` actually put in place.
    pub fn record(&mut self, declared: BTreeSet<String>) {
        self.applied = declared;
    }

    pub fn applied(&self) -> &BTreeSet<String> {
        &self.applied
    }

    pub fn is_empty(&self) -> bool {
        self.applied.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::grammar::Options;

    fn set(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn keys_are_stable_and_parseable_per_kind() {
        assert_eq!(
            extra_key(&Statement::Shim("rg".into(), Options::default())).as_deref(),
            Some("shim:rg")
        );
        assert_eq!(
            extra_key(&Statement::Service("nginx".into(), Options::default())).as_deref(),
            Some("service:nginx")
        );
        assert_eq!(
            extra_key(&Statement::Repo {
                backend: "apt".into(),
                spec: "ppa:x/y".into()
            })
            .as_deref(),
            Some("repo:apt:ppa:x/y")
        );
        // A repo key splits into kind + (backend:spec) so the undo gets the whole target.
        assert_eq!(split_key("repo:apt:ppa:x/y"), Some(("repo", "apt:ppa:x/y")));
        assert_eq!(split_key("service:nginx"), Some(("service", "nginx")));
    }

    #[test]
    fn a_package_line_has_no_extra_key() {
        // Packages are tracked by the registry, not this ledger.
        assert!(extra_key(&Statement::Subtract("vim".into())).is_none());
    }

    #[test]
    fn drift_is_recorded_minus_declared() {
        let mut ledger = ExtrasLedger::new();
        ledger.record(set(&["service:nginx", "shim:rg", "repo:apt:ppa:x/y"]));
        // The user deleted the service line; the other two remain.
        let declared = set(&["shim:rg", "repo:apt:ppa:x/y"]);
        assert_eq!(ledger.drift(&declared), vec!["service:nginx".to_string()]);
    }

    #[test]
    fn nothing_drifts_when_everything_is_still_declared() {
        let mut ledger = ExtrasLedger::new();
        ledger.record(set(&["shim:rg"]));
        assert!(ledger.drift(&set(&["shim:rg"])).is_empty());
    }

    #[test]
    fn a_newly_declared_extra_is_not_drift() {
        // A key declared now but not in the ledger is an ADD, not a removal — apply handles
        // it; drift() must not report it.
        let ledger = ExtrasLedger::new();
        assert!(ledger.drift(&set(&["shim:new"])).is_empty());
    }

    #[test]
    fn the_ledger_round_trips_through_toml() {
        let mut ledger = ExtrasLedger::new();
        ledger.record(set(&["service:nginx", "shim:rg"]));
        let body = toml::to_string_pretty(&ledger).unwrap();
        let parsed: ExtrasLedger = toml::from_str(&body).unwrap();
        assert_eq!(ledger, parsed);
    }

    #[test]
    fn a_missing_file_loads_empty() {
        assert!(ExtrasLedger::load(Path::new("no/such/extras.toml"))
            .unwrap()
            .is_empty());
    }
}
