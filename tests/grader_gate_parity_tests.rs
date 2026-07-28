//! GRADER, 2026-07-28 — RED. The gate-parity check compares names, not gates.
//!
//! `scripts/harness-logic-test.sh` exists because a local gate weaker than CI turns a local GO
//! into a CI NO-GO (E3/E4). Its predicate greps `ci.yml` for `scripts/*.sh` and asserts each
//! basename appears somewhere in both release scripts.
//!
//! A basename is not a gate. CI runs `harness-mutation-test.sh` **twice** — once with no
//! argument, which measures `scripts/integration-windows.sh`, and once against
//! `docker/integration/run-in-container.sh` with `SURVIVOR_BUDGET=92`. Both release scripts run
//! it once, with no argument. So the container harness — four distros, every push, 133 checks —
//! is mutation-tested only in CI, and the parity check that exists to catch exactly this
//! reports parity, because the string `harness-mutation-test.sh` does appear in both files.
//!
//! Measured: `bash scripts/harness-mutation-test.sh docker/integration/run-in-container.sh
//! --check apt jq` reports **90 survivors against the default budget of 86 and exits 1**. The
//! budget the container harness actually needs (92) lives in `ci.yml` and nowhere else, so the
//! documented local invocation of the script fails on a clean tree.
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
         harness-logic-test.sh's parity predicate compares basenames, so it cannot see this.",
        missing.join("\n  ")
    );
}

/// The budget a harness needs must travel with the harness, not with one caller.
///
/// `harness-mutation-test.sh` defaults `SURVIVOR_BUDGET` to 86, which is the Windows harness's
/// measured number. The container harness needs 92, and that 92 exists only as an `-e` flag in
/// `ci.yml`. Anyone running the script the way its own usage block documents gets a red gate on
/// a clean tree, which is how a gate learns to be ignored.
#[test]
fn the_survivor_budget_is_not_a_property_of_one_caller() {
    let script = read(&repo().join("scripts/harness-mutation-test.sh"));
    let ci = read(&repo().join(".github/workflows/ci.yml"));

    let ci_budgets: Vec<&str> = ci
        .lines()
        .filter(|l| l.contains("SURVIVOR_BUDGET="))
        .map(|l| l.trim())
        .collect();
    if ci_budgets.is_empty() {
        return; // nothing overrides it; no split to find
    }

    // The script must know every harness's budget itself, so its documented invocation works.
    let knows_container = script.contains("run-in-container") && script.contains("92");
    assert!(
        knows_container,
        "ci.yml carries a per-harness survivor budget that scripts/harness-mutation-test.sh \
         does not know about:\n  {}\n\n\
         Measured: `bash scripts/harness-mutation-test.sh docker/integration/run-in-container.sh \
         --check apt jq` reports 90 survivors against the default budget of 86 and exits 1 on a \
         clean tree. The budget belongs beside the harness it measures.",
        ci_budgets.join("\n  ")
    );
}
