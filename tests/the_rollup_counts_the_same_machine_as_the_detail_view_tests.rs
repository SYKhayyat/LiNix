//! `shall check` and `shall check health` must not report different machines.
//!
//! A manager that is not on PATH probes as `Absent`. `check health` then promotes it
//! (`check.rs`): if `priority` names it, the user told Shall to use it, so absent is a
//! **failure** and the row reads *"`apt` is not on PATH ... — and `priority` lists it, so Shall
//! was told to use it"*. The `check` rollup consumes the same probe and never promotes
//! anything — `HealthStatus::Absent => {}` (`check.rs`) — so its `critical` count is zero
//! where the detail view's is eight.
//!
//! Measured on an Ubuntu container whose `priority` names eleven managers, run with an empty
//! PATH so none of them can be reached:
//!
//! ```text
//! $ shall check health
//! Backends: 4 OK, 0 degraded, 8 critical, 44 not installed (of 56 total).
//!   [FAIL] apt — `apt` is not on PATH ... — and `priority` lists it, so Shall was told to use it
//!
//! $ shall check ; echo $?
//! ok  health      4 backend(s) ready
//! Nothing needs you.
//! 0
//! ```
//!
//! **This is a ruled case, not a judgement call.** `target-state.md` §Q2: *"The rollup and the
//! detail view read the same tally, because two counts of one machine will disagree."* And the
//! same table defines the promotion the rollup skips: **critical** is *"it is installed, **or
//! `priority` names it**, and it cannot work"*; **absent** is *"it is not installed here **and
//! nothing asked for it**"*. `priority` asked for it.
//!
//! **It has been fixed here once already, and grew back by another road.** `check.rs`:
//! *"This rollup used to skip `critical` entirely ... the rollup said `25 backend(s) ready`
//! while `check health` called the same machine `23 critical`."* That cure taught the rollup to
//! count `Critical`. It did not move the `Absent`-to-`Critical` promotion out of the one caller
//! that performs it, so the rollup still cannot see the failures that only exist after it.
//!
//! **And the comment that says this cannot happen is on the shared function.**
//! `probe_all_health` (`check.rs`): *"They share this one now, which is also what keeps the
//! two views from disagreeing about the same machine."* Sharing the probe was mistaken for
//! sharing the verdict. The promotion is the verdict, and it lives in `check health` alone.
//!
//! The consequence is not cosmetic: `check` is the command a script branches on, and it prints
//! *"Nothing needs you"* and exits 0 on a machine where every manager Shall was told to use is
//! unreachable. That is fail-loud inverted — the run that can do nothing is the run that
//! reports nothing wrong.
//!
//! The fix belongs in `probe_all_health`, which is where the two views already meet.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Managers to try as "named by `priority`, absent from this host". Enough spread that at
/// least one is absent on any single machine; the fixture verifies its choice.
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

/// A config whose `priority` names a manager this machine does not have.
///
/// `init` writes `priority` from what it finds, so the file never names an absent manager on
/// the day it is written. Appending one is what a machine looks like after the manager leaves —
/// uninstalled, or unreachable because the PATH a scheduled run inherits is not the PATH the
/// user has.
fn priority_names_a_manager_that_is_not_here(name: &str) -> Option<(PathBuf, String)> {
    for candidate in CANDIDATES {
        let dir = std::env::temp_dir().join(format!("shall-rollup-{name}-{candidate}"));
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
        let priority = dir.join("priority");
        let mut text = std::fs::read_to_string(&priority).unwrap_or_default();
        if text.lines().any(|l| l.trim() == *candidate) {
            continue; // `init` found it, so it is here
        }
        text.push_str(&format!("\n{candidate}\n"));
        std::fs::write(&priority, text).unwrap();

        // The detail view is the instrument; if it does not call this critical the fixture has
        // not produced the state under test, whatever the reason.
        if run(&dir, &["check", "health"]).contains("`priority` lists it") {
            return Some((dir, (*candidate).to_string()));
        }
    }
    None
}

/// `Backends: 4 OK, 0 degraded, 8 critical, 44 not installed (of 56 total).`
fn critical_in_detail_view(text: &str) -> Option<usize> {
    let line = text.lines().find(|l| l.starts_with("Backends:"))?;
    let at = line.find(" critical")?;
    line[..at]
        .rsplit(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())?
        .parse()
        .ok()
}

/// The rollup's health row, whichever of its three shapes it took.
fn health_row(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.contains("health") && (l.contains("ready") || l.contains("cannot run")))
        .map(|l| l.trim().to_string())
}

/// The control and the precondition in one: the detail view gets this right.
///
/// It names the manager, says `priority` asked for it, and counts it critical. Everything below
/// is the claim that the other view of the same machine does not agree — which is only a
/// finding while this passes.
#[test]
fn the_detail_view_calls_a_priority_named_absent_manager_critical() {
    let Some((dir, manager)) = priority_names_a_manager_that_is_not_here("detail") else {
        return; // every candidate is installed here; nothing to measure
    };
    let out = run(&dir, &["check", "health"]);
    assert!(
        out.contains(&format!("[FAIL] {manager}")),
        "`check health` no longer fails a `priority`-named absent manager, so the instrument \
         these tests depend on is gone:\n{out}"
    );
    assert!(
        critical_in_detail_view(&out).unwrap_or(0) > 0,
        "`check health` names it and counts zero critical:\n{out}"
    );
}

/// The rollup does not call the same machine healthy.
#[test]
fn the_rollup_does_not_report_ok_health_over_a_manager_that_cannot_run() {
    let Some((dir, manager)) = priority_names_a_manager_that_is_not_here("rollup") else {
        return;
    };
    let out = run(&dir, &["check"]);
    let row = health_row(&out).unwrap_or_else(|| "<no health row>".to_string());
    assert!(
        !row.starts_with("ok"),
        "`priority` names `{manager}` and it cannot run — `check health` says so — and the \
         rollup reports:\n  {row}\n\nfull output:\n{out}"
    );
}

/// And the two agree on the number, which is the rule rather than the instance.
///
/// Asserting only on the `ok` prefix would be satisfied by a rollup that reported one critical
/// out of eight. The spec's sentence is about the tally.
#[test]
fn both_views_count_the_same_number_of_managers_that_cannot_run() {
    let Some((dir, _)) = priority_names_a_manager_that_is_not_here("tally") else {
        return;
    };
    let detail = run(&dir, &["check", "health"]);
    let expected = critical_in_detail_view(&detail).unwrap_or(0);

    let summary = run(&dir, &["check"]);
    let row = health_row(&summary).unwrap_or_else(|| "<no health row>".to_string());
    // `N ready, M cannot run`, or `N backend(s) ready` when the rollup found none.
    let reported = match row.find(" cannot run") {
        Some(at) => row[..at]
            .rsplit(|c: char| !c.is_ascii_digit())
            .find(|s| !s.is_empty())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        None => 0,
    };

    assert_eq!(
        reported, expected,
        "`check health` counts {expected} manager(s) that cannot run; the rollup row is \
         `{row}`, which counts {reported}. Two counts of one machine.\n\ndetail:\n{detail}"
    );
}
