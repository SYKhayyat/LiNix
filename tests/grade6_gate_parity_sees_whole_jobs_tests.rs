//! GRADER round 6, 2026-07-31 — RED. The gate-parity check sees **shell scripts**, and the claim
//! it guards is about **gates**. A CI job that runs no `scripts/*.sh` is invisible to it.
//!
//! Round 2 found that the parity predicate compared *basenames*, and it was fixed to compare the
//! script plus its target (`grader_gate_parity_tests.rs`). That fix inherited the original scope:
//! both `scripts/harness-logic-test.sh`'s predicate and its Rust successor scan `ci.yml` for
//! lines containing `scripts/…​.sh` and ignore every line that does not.
//!
//! **Proven by making it happen** (GRADER §0.1). Two brand-new hard gates appended to `ci.yml`,
//! neither of them a shell script, neither run by any local gate:
//!
//! ```yaml
//!   deny-unsafe:
//!     steps:
//!     - run: cargo build --release --config 'build.rustflags=["-Funsafe_code"]'
//!     - run: cargo install cargo-audit && cargo audit --deny warnings
//! ```
//!
//! ```text
//! $ bash scripts/harness-logic-test.sh
//! == every gate CI runs is also run by the local release scripts
//!   ok    both release scripts run all 3 gate script(s) CI runs, against the same harnesses
//! ```
//!
//! Three asymmetries are live behind that `ok` **right now**, with nothing appended:
//!
//! | CI does | the local gate does | why parity cannot see it |
//! |---|---|---|
//! | `cargo test --release --no-fail-fast` | `cargo test --release` | not a `scripts/*.sh` line |
//! | job `storage` — btrfs/lvm/zfs on loopback devices, **every push** | nothing | runs `docker run` directly |
//! | `containers` matrix includes `opensuse` + `void` | `DISTROS="ubuntu fedora arch alpine tools gentoo"` | matrix membership, not a script name |
//!
//! `--no-fail-fast` is not cosmetic: `ci.yml` carries a comment explaining that without it cargo
//! stops at the first failing test *target* and the rest of the suite goes unmeasured. The local
//! gate — the one a developer runs before pushing — has exactly the defect CI documented and
//! fixed.
//!
//! The B bar in `READINESS` §8.1 is "Local gates match CI exactly." This is the check that is
//! supposed to establish it.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

fn local_gates() -> String {
    read(&repo().join("scripts/release-check.sh"))
        + &read(&repo().join("scripts/release-check.ps1"))
}

/// `cargo test` is the single most important gate in the repo. CI runs it one way and the local
/// gates run it another, and the difference decides how much of the suite is measured at all.
#[test]
fn the_local_gate_runs_cargo_test_the_way_ci_does() {
    let ci = read(&repo().join(".github/workflows/ci.yml"));
    let ci_runs_no_fail_fast = ci
        .lines()
        .any(|l| l.contains("cargo test") && l.contains("--no-fail-fast"));
    assert!(
        ci_runs_no_fail_fast,
        "this test's premise is gone: ci.yml no longer runs `cargo test --no-fail-fast`. \
         Re-derive the comparison rather than deleting the check."
    );

    let locals = [
        (
            "scripts/release-check.sh",
            read(&repo().join("scripts/release-check.sh")),
        ),
        (
            "scripts/release-check.ps1",
            read(&repo().join("scripts/release-check.ps1")),
        ),
    ];
    let weaker: Vec<&str> = locals
        .iter()
        .filter(|(_, body)| {
            body.lines()
                .filter(|l| l.contains("cargo test"))
                .all(|l| !l.contains("--no-fail-fast"))
        })
        .map(|(name, _)| *name)
        .collect();

    assert!(
        weaker.is_empty(),
        "CI runs `cargo test --release --no-fail-fast`; {} runs it without the flag.\n\n\
         ci.yml's own comment: \"cargo stops at the first test TARGET that fails, and this suite \
         has dozens. Run 30503630610 reported exactly one failing target per OS and never ran \
         the rest.\" A local gate that reports fewer failures than CI is a local GO that CI \
         turns into a NO-GO.",
        weaker.join(", ")
    );
}

/// Every distro CI drives on **every push** must be drivable by the local matrix, or a developer
/// cannot reproduce a red leg without pushing.
#[test]
fn the_local_matrix_covers_every_distro_ci_runs_on_every_push() {
    let ci = read(&repo().join(".github/workflows/ci.yml"));
    // The per-push `containers` matrix rows: `- { distro: NAME, …`.
    let mut ci_distros: Vec<String> = ci
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- { distro:"))
        .filter_map(|t| t.split(',').next())
        .map(|d| d.trim().to_string())
        .collect();
    ci_distros.sort();
    ci_distros.dedup();
    assert!(
        ci_distros.len() >= 4,
        "found only {} distro(s) in ci.yml's matrix; this scan has stopped matching it: {:?}",
        ci_distros.len(),
        ci_distros
    );

    let sh = read(&repo().join("scripts/release-check.sh"));
    let default_distros = sh
        .lines()
        .find(|l| l.contains("DISTROS:-"))
        .unwrap_or_default()
        .to_string();

    // `tools` and `gentoo` are nightly-only in CI, so the local gate running them is *stronger*,
    // which is fine. The failure this asserts is the other direction.
    let nightly_only = ["tools", "gentoo"];
    let missing: Vec<&String> = ci_distros
        .iter()
        .filter(|d| !nightly_only.contains(&d.as_str()))
        .filter(|d| !default_distros.contains(d.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "CI drives {:?} on every push; scripts/release-check.sh's default matrix is:\n  {}\n\
         missing: {:?}\n\n\
         Those legs exist because `zypper` and `xbps` were registered backends with no real \
         lifecycle anywhere. A developer running the repo's own ship gate still never drives them.",
        ci_distros,
        default_distros.trim(),
        missing
    );
}

/// The general form, and the one that keeps this from going stale: **every CI job must be
/// reachable from a local gate.** A job whose steps contain no `scripts/*.sh` is exactly the kind
/// the current parity predicate cannot see, and `storage` — the only job that touches real block
/// devices — is one.
#[test]
fn every_ci_job_has_something_local_that_runs_it() {
    let ci = read(&repo().join(".github/workflows/ci.yml"));
    let local = local_gates();

    // Top-level job ids: two-space-indented `name:` keys under `jobs:`.
    let jobs: Vec<String> = ci
        .lines()
        .skip_while(|l| l.trim() != "jobs:")
        .filter(|l| {
            l.starts_with("  ")
                && !l.starts_with("   ")
                && l.trim_end().ends_with(':')
                && !l.trim_start().starts_with('#')
        })
        .map(|l| l.trim().trim_end_matches(':').to_string())
        .collect();
    assert!(
        jobs.len() >= 5,
        "found only {} job(s) in ci.yml; this scan has stopped matching it: {jobs:?}",
        jobs.len()
    );

    // Reachability is judged by what a job actually *drives*, never by its name — otherwise this
    // check makes the same mistake it is about. Each entry says how a local gate reaches that job,
    // and every one below was verified by reading the local script:
    //
    //   build           cargo fmt/clippy/test/build + harness-logic-test.sh — both scripts
    //   release         publishes artifacts on a tag; drives nothing
    //   containers      docker/integration/run.sh, via release-check.sh's DISTROS
    //   slow-containers the same run.sh — `tools` and `gentoo` are in the default DISTROS
    //   macos-native    release-check.sh's Darwin branch: integration-windows.sh brew wget
    //   windows-native  release-check.ps1: integration-windows.sh $Backend $Package
    //   argv-drift      `cargo test` builds and runs tests/argv_drift_tests.rs
    //   harness-mutation harness-mutation-test.sh — both scripts, both targets
    //   storage         release-check.sh's DISTROS names the image; run.sh gives it --privileged
    //
    // `storage` had no row when this was written, and that was the finding: the only job that
    // touches real block devices, on every push, driven by nothing a developer can run. Its
    // needle is the image name rather than `run.sh`, because `run.sh` appears in this file
    // twice already and would have made the row true without making the job reachable.
    let reached_by: &[(&str, &str)] = &[
        ("build", "cargo build"),
        ("containers", "docker/integration/run.sh"),
        ("slow-containers", "docker/integration/run.sh"),
        ("macos-native", "integration-windows.sh"),
        ("windows-native", "integration-windows.sh"),
        ("harness-mutation", "harness-mutation-test.sh"),
        ("argv-drift", "cargo test"),
        ("storage", "storage"),
    ];
    let drives_nothing = ["release"];

    let unreachable: Vec<String> = jobs
        .iter()
        .filter(|j| !drives_nothing.contains(&j.as_str()))
        .filter(|j| match reached_by.iter().find(|(job, _)| job == j) {
            // Claimed reachable — and the claim is checked, not trusted.
            Some((_, needle)) => !local.contains(needle),
            // No claim at all: nothing local drives this job.
            None => true,
        })
        .cloned()
        .collect();

    assert!(
        unreachable.is_empty(),
        "{} CI job(s) are driven by nothing in either release script: {:?}\n\nof jobs: {:?}\n\n\
         `storage` builds `docker/integration/Dockerfile.storage` and drives btrfs/lvm/zfs on \
         real loopback devices with --privileged, on every push. No local gate runs it, and the \
         parity predicate reports `ok` because that job's steps contain no `scripts/*.sh` — it \
         compares script names, and the claim it guards is about gates.",
        unreachable.len(),
        unreachable,
        jobs
    );
}
