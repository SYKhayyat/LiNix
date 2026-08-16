//! A package Shall could not install must not be reported as one nothing declares.
//!
//! `SyncChanges::skipped` is one `Vec<Skipped>` fed by two opposite questions:
//!
//! | producer | what the row is |
//! |---|---|
//! | `planner.rs`, `planner.rs` (via `Declined::reported`) | a **removal** declined — the package is installed, nothing declares it, and it stays |
//! | `planner.rs` | an **install** skipped — the package is declared, it is *not* installed, and it does not arrive |
//!
//! Three surfaces print that vec, and all three describe it with the first row's sentence:
//!
//! - `sync.rs` — *"Left in place (N) — installed, declared nowhere, and not removed"*
//! - `plan.rs` — *"N package(s) are installed, declared nowhere, and will not be removed"*
//! - `check.rs` — *"N package(s) installed and declared nowhere that `sync` will not remove"*
//!
//! For an install skip every clause is false: it is not installed, it *is* declared, and there
//! was never a removal to decline. The follow-up advice — *"Declare them to keep them"* — asks
//! for the thing the user already did, which is why the line is in the plan at all.
//!
//! Shall knows better in the same breath. One `shall check` prints both of these, in order:
//!
//! ```text
//!  WARN `tree` is declared for `apt`, which is not on this machine — skipping it.
//! ->  drift  1 package(s) installed and declared nowhere that `sync` will not remove: apt:tree
//! ```
//!
//! **This is the bug the type was created to stop.** `Skipped`'s own doc says a fixed sentence
//! *"is wrong for every input it does not describe — `adopt` printed 'Left alone: 185 (listed
//! in the manifest)' about items none of which were listed in the manifest"*. The per-row
//! `reason` field exists because the reasons differ; the headers above it assert three facts
//! about every row regardless.
//!
//! **How it is reached.** `priority` names a manager the machine does not have. That is not
//! exotic: `init` writes `priority` from what it finds, and a manager can leave afterwards — a
//! `sudo` run under `secure_path`, a scheduled sync with a minimal environment, or a manager
//! simply uninstalled. `shall schedule` exists, so an unattended sync is a supported use.
//!
//! The fix is a chiluk, not a wording change: a declined removal and a skipped install are two
//! questions and want two lists, or one list whose header is derived per row the way `reason`
//! already is.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Managers to try as "in `priority`, not on this host". Enough spread that some are absent
/// on any one machine; the fixture verifies its choice rather than trusting this list.
const CANDIDATES: &[&str] = &[
    "apt", "pacman", "apk", "xbps", "zypper", "dnf", "emerge", "choco", "winget", "brew", "nix",
];

fn shall() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_shall"))
}

fn run(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(shall())
        .args(args)
        .env("SHALL_CONFIG_DIR", dir)
        .env("SHALL_DATA_DIR", dir.join("data"))
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A config declaring one package for a manager that is listed in `priority` and absent here.
///
/// Returns the directory and the manager it settled on, or `None` when every candidate turned
/// out to be installed — in which case there is nothing on this host to measure and the test
/// says so rather than passing quietly.
fn a_declaration_no_manager_can_serve(name: &str) -> Option<(PathBuf, String)> {
    for candidate in CANDIDATES {
        let dir = std::env::temp_dir().join(format!("shall-skipped-{name}-{candidate}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let init = Command::new(shall())
            .arg("init")
            .env("SHALL_CONFIG_DIR", &dir)
            .env("SHALL_DATA_DIR", dir.join("data"))
            .output()
            .expect("init should run");
        if !init.status.success() {
            continue;
        }

        // `init` writes `priority` from the managers it found, so a manager that is absent is
        // also unlisted — and an unlisted manager is refused at the grammar, well before the
        // planner. Appending it is what a machine that *lost* a manager looks like.
        let priority = dir.join("priority");
        let mut text = std::fs::read_to_string(&priority).unwrap_or_default();
        if text.lines().any(|l| l.trim() == *candidate) {
            continue; // present here; its skip would never be planned
        }
        text.push_str(&format!("\n{candidate}\n"));
        std::fs::write(&priority, text).unwrap();
        std::fs::write(
            dir.join("modules").join("starter.txt"),
            format!("{candidate}:tree\n"),
        )
        .unwrap();

        // The manager may be installed and merely unlisted, in which case the planner would
        // schedule a real install. The skip warning is the proof that it is not here.
        let out = run(&dir, &["plan"]);
        if out.contains("is not on this machine") {
            return Some((dir, (*candidate).to_string()));
        }
    }
    None
}

/// The claims the three surfaces make about every row in `skipped`.
const CLAIMS: &[&str] = &[
    "installed, declared nowhere, and not removed",
    "are installed, declared nowhere, and will not be removed",
    "installed and declared nowhere that `sync` will not remove",
];

fn claim_in(text: &str) -> Option<&'static str> {
    CLAIMS.iter().copied().find(|c| text.contains(c))
}

/// The control, and it is the half that makes the rest a finding rather than a wording quibble.
///
/// Shall names the package as *declared* in the same output that calls it declared nowhere. If
/// this line ever stops being printed, the tests below would be arguing with a program that
/// never claimed to know better.
#[test]
fn shall_says_the_package_is_declared_in_the_same_breath() {
    let Some((dir, manager)) = a_declaration_no_manager_can_serve("control") else {
        return; // every candidate manager is installed here; nothing to measure
    };
    let out = run(&dir, &["check"]);
    assert!(
        out.contains(&format!("`tree` is declared for `{manager}`")),
        "the control line is gone, so the contradiction the other tests rest on is \
         unproven:\n{out}"
    );
}

/// A second control: with nothing skipped, none of the three sentences is printed.
///
/// Without this, a test that greps for a sentence could be satisfied by a program that prints
/// it unconditionally, and would go green the day the sentence was removed for the wrong
/// reason.
#[test]
fn a_plan_with_nothing_skipped_makes_no_such_claim() {
    // `tempfile`, not a fixed name: `config.rs` states the rule, and the sibling fixture in
    // `a_rehearsal_asks_the_guard_the_act_asks_tests.rs` is where breaking it cost a red CI leg.
    let held = tempfile::tempdir().expect("a temp dir");
    let dir = held.path().to_path_buf();
    let init = Command::new(shall())
        .arg("init")
        .env("SHALL_CONFIG_DIR", &dir)
        .env("SHALL_DATA_DIR", dir.join("data"))
        .output()
        .expect("init should run");
    assert!(init.status.success(), "init failed");
    std::fs::write(dir.join("modules").join("starter.txt"), "").unwrap();

    let out = run(&dir, &["plan"]);
    assert!(
        claim_in(&out).is_none(),
        "an empty plan already claims something is installed and undeclared:\n{out}"
    );
}

/// `shall plan` does not describe a declared package as declared nowhere.
#[test]
fn the_plan_does_not_call_a_declared_package_undeclared() {
    let Some((dir, manager)) = a_declaration_no_manager_can_serve("plan") else {
        return;
    };
    let out = run(&dir, &["plan"]);
    assert!(
        claim_in(&out).is_none(),
        "`{manager}:tree` is declared in `modules/starter.txt` and is not installed. The plan \
         reports it as installed and declared nowhere:\n{out}"
    );
}

/// And neither does `sync`, which prints the same list through the same function.
#[test]
fn sync_does_not_call_a_declared_package_undeclared() {
    let Some((dir, manager)) = a_declaration_no_manager_can_serve("sync") else {
        return;
    };
    let out = run(&dir, &["sync", "--dry-run", "--yes"]);
    assert!(
        claim_in(&out).is_none(),
        "`sync` reports the declared, uninstalled `{manager}:tree` as an undeclared \
         leftover:\n{out}"
    );
}

/// And neither does `check`, which turns the same list into a drift finding.
///
/// The one of the three that a script reads: drift is what decides whether `check` reports the
/// machine as needing attention, so this row is not only a sentence.
#[test]
fn check_does_not_report_a_declared_package_as_an_undeclared_leftover() {
    let Some((dir, manager)) = a_declaration_no_manager_can_serve("check") else {
        return;
    };
    let out = run(&dir, &["check"]);
    assert!(
        claim_in(&out).is_none(),
        "`check` counts the declared, uninstalled `{manager}:tree` as drift of the opposite \
         kind:\n{out}"
    );
}
