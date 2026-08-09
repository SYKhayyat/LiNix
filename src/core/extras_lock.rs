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

use crate::config::grammar::{ResourceKind, Statement};
use crate::core::ledger::LockFile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The identity of one applied extra: **what kind of thing it is, and which one**.
///
/// `Statement::key()` produces two different key spaces and its type does not say which. A
/// package statement keys `backend:name`; a keyword statement keys `kind:subject`. The hazard is
/// named in `Statement::kind`'s own doc comment — *"re-splitting `key` on `:` … would read
/// `apt:jq` as the kind `apt`"* — and it was still being re-split, by hand, in three places that
/// deal in extras keys and by five more that deal in package keys and must not be confused with
/// them.
///
/// This is the extras half, as one type. It is **the only producer and the only reader** of a
/// `<kind>:<subject>` string: `Display` writes it, [`FromStr`](std::str::FromStr) reads it, and
/// the ledger on disk is a set of those strings.
///
/// **Parsed at the boundary, not at load.** The ledger deserialises as strings and each row is
/// parsed where it is used, so one unreadable row — a file written by a newer LiNix, an edit by
/// hand — is reported and kept rather than failing the whole file. Forgetting a row is the one
/// outcome that cannot be undone.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtraKey {
    pub kind: ResourceKind,
    /// Everything after the keyword, verbatim. A `repo:` subject is itself `backend:spec`, which
    /// is why this is not split further here: that inner structure is the repo backend's, and
    /// splitting it twice in one type is how the second reader gets it wrong.
    pub subject: String,
}

impl ExtraKey {
    pub fn new(kind: ResourceKind, subject: impl Into<String>) -> Self {
        Self {
            kind,
            subject: subject.into(),
        }
    }

    /// The ledger key of a file LiNix placed, from its destination.
    ///
    /// A second caller asks this question from the other end: a `dotfiles:` tree has the
    /// destination in hand and needs to know whether the ledger already claims it. That question
    /// and [`extra_key`]'s `link:` arm must produce the same string or a teardown searches for a
    /// row nothing wrote, so there is one constructor and that arm calls it.
    pub fn link(destination: &Path) -> Self {
        Self::new(ResourceKind::Link, destination.display().to_string())
    }
}

impl std::fmt::Display for ExtraKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind, self.subject)
    }
}

impl std::str::FromStr for ExtraKey {
    type Err = ();

    /// `service:nginx` → `(Service, "nginx")`; `repo:apt:ppa:x/y` → `(Repo, "apt:ppa:x/y")`.
    /// Split at the **first** colon: the kind is one keyword and everything after it belongs to
    /// the subject, including a `repo:` subject's own colons.
    fn from_str(s: &str) -> std::result::Result<Self, ()> {
        let (kind, subject) = s.split_once(':').ok_or(())?;
        Ok(Self::new(kind.parse()?, subject))
    }
}

/// The stable identity of an applied extra, `<kind>:<id>`. Parseable back into an undo action
/// (see `Extras::reconcile`), and stable across runs so the same declaration always keys
/// the same ledger entry. Returns `None` for statements that are not applied extras (packages,
/// set-math, `use`) — those are tracked elsewhere or not at all.
pub fn extra_key(stmt: &Statement) -> Option<ExtraKey> {
    match stmt {
        // A link is keyed by its DESTINATION, not by its source — the one place this ledger
        // departs from [`Statement::key`]. The undo has to remove what LiNix wrote, and by the
        // time it runs the declaration is gone, so a key naming the source would hand the
        // teardown the file in your repo and leave the deployed one in place. Keying the
        // destination also makes an edited `@target=` a removal of the old destination and an
        // install of the new, instead of leaving the old one forever.
        Statement::Link(name, opts) => Some(
            opts.one("target")
                .and_then(|t| crate::backends::link::resolve_target(t).ok())
                .map(|p| ExtraKey::link(&p))
                .unwrap_or_else(|| ExtraKey::new(ResourceKind::Link, name)),
        ),
        // `exec:` is deliberately NOT an extra. Extras are nouns whose teardown undoes what
        // they put in place; a verb has no such inverse, and a script that succeeds makes its
        // own `when` false — so wiring it into this ledger would re-run or "undo" it every
        // time the condition swung. Its lifecycle is `locks/exec.toml`, not here (XIII.3).
        //
        // A dotfiles tree is excluded for the opposite reason: its files ARE keyed here, one
        // row per placed file (U22), but the rows come from `Dotfiles::links` — which walks
        // the tree — because this function has only the declaration and a tree's contents are
        // a fact about the disk. That is precisely why the row it documents did not exist for
        // two weeks: nothing was in a position to write it, and four documents said otherwise.
        // `generate:` is excluded for the same reason as `exec:`: it is a verb that runs a
        // command, not a noun with an inverse. Its output declarations ARE nouns and are keyed
        // here individually once merged, but the generate line itself has no teardown.
        Statement::Exec(..) | Statement::Generate(..) | Statement::Dotfiles(..) => None,
        // **A setting is keyed with its scope**, because the teardown resets what the
        // declaration wrote and `@scope=` is the only thing that says where that was. Without
        // it the key is `setting:org.gnome.x/y` for both a user line and a system one, and the
        // removal reset the store's default scope — so deleting a `setting:x@scope=system` line
        // reset the USER key and left the machine-wide value in place, silently.
        //
        // Unscoped keys stay exactly as they were: `@scope=` is written only to override a
        // store's own default, so a key with no suffix means that default, which is what every
        // existing row in every ledger already means.
        Statement::Setting(name, opts) => Some(ExtraKey::new(
            ResourceKind::Setting,
            match opts.one("scope") {
                Some(scope) => format!("{}@scope={}", name, scope),
                None => name.clone(),
            },
        )),
        // Everything else with a keyword is a noun with an inverse: deleting a `firewall:` line
        // closes the port (N5), deleting a `service:` line disables the service.
        //
        // Built from the kind and the subject rather than from `Statement::key()`, so the key
        // this ledger writes and the key it reads back are one construction. `key()` is
        // documented as the *display* form of a line; that the two agree for these kinds is
        // true and is not a promise anybody made.
        _ => Some(ExtraKey::new(stmt.kind()?, stmt.subject()?)),
    }
}

/// `locks/extras.toml`: the set of extra keys the last successful sync put in place. A
/// `BTreeSet` so the file is ordered and diffs cleanly in git.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExtrasLedger {
    #[serde(default)]
    applied: BTreeSet<String>,
}

impl LockFile for ExtrasLedger {
    const WHAT: &'static str = "the extras ledger";
}

impl ExtrasLedger {
    pub fn path_in(locks_dir: &Path) -> PathBuf {
        locks_dir.join("extras.toml")
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

    fn shown(stmt: Statement) -> Option<String> {
        extra_key(&stmt).map(|k| k.to_string())
    }

    #[test]
    fn keys_are_stable_and_parseable_per_kind() {
        assert_eq!(
            shown(Statement::Shim("rg".into(), Options::default())).as_deref(),
            Some("shim:rg")
        );
        assert_eq!(
            shown(Statement::Service("nginx".into(), Options::default())).as_deref(),
            Some("service:nginx")
        );
        assert_eq!(
            shown(Statement::Repo {
                backend: "apt".into(),
                spec: "ppa:x/y".into()
            })
            .as_deref(),
            Some("repo:apt:ppa:x/y")
        );
    }

    /// **What is written is what is read back.** The ledger is a set of these strings on disk,
    /// so `Display` and `FromStr` being inverses is the whole of its wire format.
    ///
    /// A `repo:` subject carries its own colons, which is why the split is at the FIRST one and
    /// why the subject is not split again here: `repo:apt:ppa:x/y` must hand the undo
    /// `apt:ppa:x/y`, whole.
    #[test]
    fn a_key_round_trips_through_the_string_the_ledger_stores() {
        for (text, kind, subject) in [
            ("service:nginx", ResourceKind::Service, "nginx"),
            ("repo:apt:ppa:x/y", ResourceKind::Repo, "apt:ppa:x/y"),
            ("link:/home/u/.vimrc", ResourceKind::Link, "/home/u/.vimrc"),
            ("firewall:22/tcp", ResourceKind::Firewall, "22/tcp"),
        ] {
            let parsed: ExtraKey = text.parse().unwrap_or_else(|_| panic!("`{text}` parses"));
            assert_eq!(parsed, ExtraKey::new(kind, subject));
            assert_eq!(parsed.to_string(), text);
        }

        // A package key is not an extras key, which is the confusion `Statement::kind`'s own
        // doc comment warns about: re-splitting on `:` would read `apt:jq` as the kind `apt`.
        assert_eq!("apt:jq".parse::<ExtraKey>(), Err(()));
        // And a bare word names no kind at all.
        assert_eq!("nginx".parse::<ExtraKey>(), Err(()));
    }

    /// **Every key this ledger can write names a kind the teardown can dispatch on.**
    ///
    /// The teardown is exhaustive over `ResourceKind` now, so the one way a row can still
    /// arrive un-actionable is for the *key* to open with something that is not a keyword. That
    /// is what a `&str` dispatch could never notice: it matched the arms it knew and shrugged
    /// at the rest, and the shrug reported the undo as done.
    #[test]
    fn every_ledger_key_names_a_kind_the_teardown_can_dispatch_on() {
        let o = Options::default;
        let statements = [
            Statement::Shim("rg".into(), o()),
            Statement::Service("nginx".into(), o()),
            Statement::Setting("dark".into(), o()),
            Statement::Link("src".into(), o()),
            Statement::Schedule("nightly".into(), o()),
            Statement::Firewall("22/tcp".into(), o()),
            Statement::Repo {
                backend: "apt".into(),
                spec: "ppa:x/y".into(),
            },
        ];
        for stmt in &statements {
            let key = extra_key(stmt).unwrap_or_else(|| panic!("{stmt:?} produced no key"));
            let parsed: ExtraKey = key
                .to_string()
                .parse()
                .unwrap_or_else(|_| panic!("`{key}` does not read back as a key"));
            assert_eq!(parsed, key, "`{key}` does not survive its own round trip");
            assert_eq!(
                Some(key.kind),
                stmt.kind(),
                "`{key}` names a different kind than the statement it came from"
            );
        }

        // And the three that must NOT be keyed: a verb has no inverse, and a tree's rows are
        // the `link:` keys its files were placed under.
        assert!(extra_key(&Statement::Exec("./x.sh".into(), o())).is_none());
        assert!(extra_key(&Statement::Generate("./x.sh".into(), o())).is_none());
        assert!(extra_key(&Statement::Dotfiles("tree".into(), o())).is_none());
    }

    /// **The scope rides the key, because by teardown time the line that carried it is gone.**
    ///
    /// `setting:x@scope=system` and `setting:x` were the same ledger row, so removing the
    /// system-scoped line reset the USER key and left the machine-wide value in place, reporting
    /// success. The removal reads the scope back off the subject; an unscoped key still means
    /// the store's own default, which is what every row written before this said.
    #[test]
    fn a_scoped_setting_is_a_different_row_from_an_unscoped_one() {
        let mut system = Options::default();
        system.insert("scope", "system");
        let scoped = extra_key(&Statement::Setting("org.gnome.desktop/theme".into(), system))
            .expect("a setting is an extra");
        assert_eq!(
            scoped.to_string(),
            "setting:org.gnome.desktop/theme@scope=system"
        );

        let plain = extra_key(&Statement::Setting(
            "org.gnome.desktop/theme".into(),
            Options::default(),
        ))
        .expect("a setting is an extra");
        assert_eq!(plain.to_string(), "setting:org.gnome.desktop/theme");

        assert_ne!(
            scoped, plain,
            "one row for both scopes is how a system reset became a user reset"
        );
        // Both still round trip, and the scope stays in the subject rather than becoming a
        // second kind: the teardown dispatches on the kind and reads the rest.
        for key in [&scoped, &plain] {
            assert_eq!(key.kind, ResourceKind::Setting);
            assert_eq!(
                key.to_string().parse::<ExtraKey>().as_ref(),
                Ok(key),
                "`{key}` does not read back"
            );
        }
        // And deleting a system line leaves an unscoped declaration of the same key alone.
        let ledger_had = [scoped.to_string()].into_iter().collect();
        let mut ledger = ExtrasLedger::new();
        ledger.record(ledger_had);
        assert_eq!(
            ledger.drift(&[plain.to_string()].into_iter().collect()),
            vec![scoped.to_string()]
        );
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
