//! GRADER, 2026-07-28 — RED. The gate-parity check compares names, not gates.
//!
//! `scripts/harness-logic-test.sh` exists because a local gate weaker than CI turns a local GO
//! into a CI NO-GO (E3/E4). Its predicate greps `ci.yml` for `scripts/*.sh` and asserts each
//! basename appears somewhere in both release scripts.
//!
//! A basename is not a gate. CI runs `harness-mutation-test.sh` **twice** — once with no
//! argument, which measures `scripts/integration-windows.sh`, and once against
//! `docker/integration/run-in-container.sh`, which then carried `SURVIVOR_BUDGET=92`. Both release scripts run
//! it once, with no argument. So the container harness — four distros, every push, 133 checks —
//! is mutation-tested only in CI, and the parity check that exists to catch exactly this
//! reports parity, because the string `harness-mutation-test.sh` does appear in both files.
//!
//! Measured then: `bash scripts/harness-mutation-test.sh docker/integration/run-in-container.sh
//! --check apt jq` reported **90 survivors against the default budget of 86 and exited 1**. The
//! budget the container harness actually needed (92) lived in `ci.yml` and nowhere else, so the
//! documented local invocation of the script failed on a clean tree.
//!
//! Both halves are fixed: each harness's thresholds live in the script, and `ci.yml` overriding
//! one is what the second test below fails on. The thresholds are survival RATES now rather than
//! counts, for the reason `harness-mutation-test.sh`'s own header gives at length — a count
//! cannot tell a harness that grew from one that got weaker, and it stopped both harnesses for
//! growing.
//!
//! This is the same shape as every defect in READINESS §5.3: a checker that examines something
//! adjacent to the thing it is named for.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// Every distinct way `ci.yml` invokes a repo gate script: the script plus the arguments that
/// decide *what it measures*. Two invocations of one script with different targets are two
/// gates.
fn ci_invocations() -> Vec<String> {
    let ci = read(&repo().join(".github/workflows/ci.yml"));
    let mut out = Vec::new();
    for line in ci.lines() {
        let t = line.trim();
        let Some(at) = t.find("scripts/") else {
            continue;
        };
        if !t[at..].contains(".sh") {
            continue;
        }
        // From the script name to the end of the shell word list, minus CI's own quoting.
        let tail = t[at..]
            .trim_end_matches(['"', '\\', '\''])
            .trim()
            .to_string();
        out.push(tail);
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn every_gate_ci_runs_is_run_locally_with_the_same_target() {
    let ci = ci_invocations();
    // Without a floor this passes on a tree where the scan matched nothing — the G2 shape.
    assert!(
        ci.len() >= 3,
        "found only {} gate invocation(s) in ci.yml; this scan has stopped matching it",
        ci.len()
    );

    let locals: Vec<(String, String)> = ["scripts/release-check.sh", "scripts/release-check.ps1"]
        .iter()
        .map(|p| (p.to_string(), read(&repo().join(p))))
        .collect();

    let mut missing = Vec::new();
    for inv in &ci {
        // The target a gate measures, when it takes one: the harness path on its command line.
        let target = inv
            .split_whitespace()
            .find(|w| w.contains("integration") && w.ends_with(".sh"))
            .map(|w| w.to_string());
        let Some(target) = target else { continue };
        let script = inv.split_whitespace().next().unwrap_or("").to_string();
        if script.contains(&target) {
            continue; // the gate IS that harness, not a gate pointed at it
        }
        for (name, body) in &locals {
            let runs_it = body
                .lines()
                .filter(|l| l.contains(&script))
                .any(|l| l.contains(&target));
            if !runs_it {
                missing.push(format!(
                    "{name} never runs {script} against {target} — CI does:\n        {inv}"
                ));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "a local gate measures less than the CI gate of the same name:\n  {}\n\n\
         `the_review_apparatus_is_rust_tests` checks parity over gate SCRIPTS; a job whose \n         steps run a command directly is invisible to it, which is what this one is for.",
        missing.join("\n  ")
    );
}

/// The thresholds a harness needs travel with the harness, not with one caller.
///
/// `harness-mutation-test.sh` once defaulted to the Windows harness's numbers alone, and the
/// container harness's lived only as an `-e` flag in `ci.yml`. Anyone running the script the way
/// its own usage block documents got a red gate on a clean tree, which is how a gate learns to
/// be ignored.
///
/// **Stated as a prohibition on `ci.yml`, because the previous form went vacuous the moment it
/// started passing.** It read the override flags out of `ci.yml` and returned green when there
/// were none — so the fix that removed them also removed the gate, and the substring it checked
/// for (`92`) would have been satisfied by that number appearing in any comment. There is no
/// arrangement of this repository in which the assertion below is trivially true.
#[test]
fn the_survivor_thresholds_are_not_a_property_of_one_caller() {
    let script = read(&repo().join("scripts/harness-mutation-test.sh"));
    let ci = read(&repo().join(".github/workflows/ci.yml"));

    const TUNABLES: [&str; 4] = [
        "SURVIVOR_RATE",
        "CAUGHT_FLOOR",
        "FAIL_SURVIVOR_RATE",
        "FAIL_CAUGHT_FLOOR",
    ];

    let overrides: Vec<&str> = ci
        .lines()
        .filter(|l| TUNABLES.iter().any(|t| l.contains(&format!("{}=", t))))
        .map(|l| l.trim())
        .collect();
    assert!(
        overrides.is_empty(),
        "ci.yml sets a mutation threshold that belongs in the script:\n  {}\n\n\
         A threshold set by one caller is a threshold the script's own documented invocation \
         does not have, and that invocation then fails on a clean tree.",
        overrides.join("\n  ")
    );

    // And the script knows one set per harness, keyed on the harness it is pointed at.
    for var in [
        "DEFAULT_RATE",
        "DEFAULT_FLOOR",
        "DEFAULT_FAIL_RATE",
        "DEFAULT_FAIL_FLOOR",
    ] {
        let arms = script.matches(&format!("{}=", var)).count();
        assert!(
            arms >= 2,
            "scripts/harness-mutation-test.sh sets `{}` {} time(s). Each harness needs its own: \
             the container harness runs half again as many checks as the Windows one, so one \
             number cannot be right for both.",
            var,
            arms
        );
    }
}
