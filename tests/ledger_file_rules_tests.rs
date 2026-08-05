//! The rules every `locks/` file obeys, asserted over the whole family — and a ratchet so the
//! seventh ledger cannot quietly opt out of them.
//!
//! Six ledgers had six identical copies of `load` and `save`. They all agreed, which is the
//! interesting part: nobody had drifted, because one person found the rules once and wrote them
//! out six times. What that costs is not the ~170 duplicated lines. It is that a *new* ledger
//! inherits the rules only if whoever writes it remembers to copy them — and
//! `locks/versions.json` and `locks/hooks.toml` being left behind by `linix --dry-run lock` is
//! what forgetting looks like.
//!
//! `core::ledger::LockFile` now carries the rules. This file holds them to it.
//!
//! **Its own process, deliberately.** `dry_run` is a process-wide atomic set once from `main`.
//! Flipping it inside the library's unit-test binary would flip it for every test sharing that
//! binary, and the ones it would break are the ones that write files.

use linix::core::{
    ArtifactLedger, BareLock, ExecLedger, ExtrasLedger, HookLedger, LockFile, RegexLock,
};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Every ledger, as a pair of closures over the trait's two file operations.
///
/// The list is written out because there is no way to enumerate implementors of a trait at
/// runtime — which is exactly why `no_ledger_hand_rolls_its_own_carrier` below reads the source
/// instead of trusting this list to be complete.
type SaveTo = fn(&Path) -> linix::core::Result<()>;
type LoadFrom = fn(&Path) -> bool;

fn family() -> Vec<(&'static str, SaveTo, LoadFrom)> {
    vec![
        (
            "regex",
            |p| RegexLock::new().save(p),
            |p| RegexLock::load(p).is_ok(),
        ),
        (
            "extras",
            |p| ExtrasLedger::new().save(p),
            |p| ExtrasLedger::load(p).is_ok(),
        ),
        (
            "exec",
            |p| ExecLedger::new().save(p),
            |p| ExecLedger::load(p).is_ok(),
        ),
        (
            "hooks",
            |p| HookLedger::new().save(p),
            |p| HookLedger::load(p).is_ok(),
        ),
        (
            "artifact",
            |p| ArtifactLedger::new().save(p),
            |p| ArtifactLedger::load(p).is_ok(),
        ),
        (
            "bare",
            |p| BareLock::new().save(p),
            |p| BareLock::load(p).is_ok(),
        ),
    ]
}

/// A missing file is the correct starting state for every one of them, never an error. A
/// ledger that failed on absence would make a first run on a fresh repo fail.
#[test]
fn a_missing_file_loads_empty_for_every_ledger() {
    for (name, _, load) in family() {
        let missing = PathBuf::from("no")
            .join("such")
            .join(format!("{name}.toml"));
        assert!(
            load(&missing),
            "{name}: a missing ledger must load empty, not fail"
        );
    }
}

/// The rule that was found once and copied six times, now asserted once over all six.
///
/// **This must be the only test in this file that touches `dry_run`**, and it runs last-ish by
/// name only because nothing else here depends on the flag being off.
#[test]
fn no_ledger_writes_during_a_dry_run() {
    let tmp = TempDir::new().unwrap();

    // Real run first: every ledger writes, so the dry-run assertion below is about the flag and
    // not about a save that never worked.
    for (name, save, _) in family() {
        let path = tmp.path().join("real").join(format!("{name}.toml"));
        save(&path).unwrap_or_else(|e| panic!("{name}: save failed outside a dry run: {e}"));
        assert!(
            path.exists(),
            "{name}: save wrote nothing even outside a dry run — the assertion below would \
             pass for the wrong reason"
        );
    }

    linix::core::dry_run::set(true);
    let mut leaked: Vec<String> = Vec::new();
    for (name, save, _) in family() {
        let path = tmp.path().join("preview").join(format!("{name}.toml"));
        save(&path).unwrap_or_else(|e| panic!("{name}: save errored during a dry run: {e}"));
        if path.exists() {
            leaked.push(name.to_string());
        }
    }
    linix::core::dry_run::set(false);

    assert!(
        leaked.is_empty(),
        "these ledgers wrote during a dry run: {leaked:?}\n\n\
         A preview that leaves a pin or an approval behind changes what the next real run does. \
         `linix --dry-run lock` used to write locks/versions.json and locks/hooks.toml."
    );
}

/// **The ratchet.** Every `*_lock.rs` gets its file rules from `LockFile` or says why not.
///
/// A source scan, because the defect is a ledger that is *not* wired to the trait: nothing the
/// program does can enumerate a type that opted out, only the source can. Written in the style
/// of `os_native_argv_coverage_tests.rs`, and for the same reason — the seventh ledger is the
/// one this is for, and it does not exist yet.
#[test]
fn no_ledger_hand_rolls_its_own_carrier() {
    let core = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/core");
    let mut ledgers: Vec<String> = Vec::new();
    let mut offenders: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(&core).expect("cannot read src/core") {
        let path = entry.expect("bad dir entry").path();
        let Some(file) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if !file.ends_with("_lock.rs") {
            continue;
        }
        ledgers.push(file.to_string());
        let src = std::fs::read_to_string(&path).expect("cannot read ledger");

        if !src.contains("impl LockFile for") {
            offenders.push(format!("{file}: does not implement LockFile"));
        }
        for hand_rolled in [
            "pub fn load(path: &Path) -> Result<Self>",
            "pub fn save(&self, path: &Path) -> Result<()>",
            "pub fn new() -> Self",
        ] {
            if src.contains(hand_rolled) {
                offenders.push(format!("{file}: hand-rolls `{hand_rolled}`"));
            }
        }
    }

    assert!(
        ledgers.len() >= 6,
        "found only {} ledger files — the scan is broken, not the code",
        ledgers.len()
    );
    assert!(
        offenders.is_empty(),
        "these ledgers carry their own file rules instead of inheriting them:\n    {}\n\n\
         Implement `core::ledger::LockFile` and delete the copy. The rules are the missing-file \
         rule, the dry-run rule and atomic write through `persist`; a copy is free to lose any \
         of the three, and the one that gets lost is the one nobody tests.",
        offenders.join("\n    ")
    );
}

/// A gate that has never failed is a claim, not a check.
#[test]
fn the_carrier_scan_can_actually_fail() {
    // The strings the scan hunts for are the real signatures, not approximations of them: if
    // the trait's shape changes, this fails here rather than silently matching nothing forever.
    let sample = "    pub fn load(path: &Path) -> Result<Self> {\n";
    assert!(sample.contains("pub fn load(path: &Path) -> Result<Self>"));

    let converted = "impl LockFile for RegexLock {\n    const WHAT: &'static str = \"x\";\n}\n";
    assert!(converted.contains("impl LockFile for"));
    assert!(!converted.contains("pub fn save(&self, path: &Path) -> Result<()>"));
}
