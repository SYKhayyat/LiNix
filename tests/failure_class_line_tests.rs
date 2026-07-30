//! BUILDER round 6, W35's second half — the classification LiNix computes has to reach the
//! caller that needs it.
//!
//! `Error::retryability()` answers "could a second attempt differ" from the backend's own exit
//! policy, and until this line existed **only two places in the program could see the answer**.
//! Everyone downstream re-derived it. The sweep harness re-derived it by RETRYING THE INSTALL
//! IMMEDIATELY, which is a proxy that is wrong for exactly the failures the classification gets
//! right: a GitHub rate limit with 1236 seconds left on the window cannot succeed one second
//! later, so it scored `defect`, the macOS leg went red, and the real-lifecycle ratchet fell
//! 8 -> 7 and went red behind it (R-3).
//!
//! So a failing command prints one line, on stderr, in a shape a script reads without grepping
//! an English sentence:
//!
//! ```text
//! linix-failure-class: permanent
//! ```
//!
//! **This file exists so the token is the stable part.** `scripts/integration-windows.sh` and
//! `docker/integration/run-in-container.sh` both read it; every other sentence LiNix prints
//! about a failure is free to be reworded, and this one is not.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = run(&dir, &["init"]);
    assert!(out.1 == 0, "the fixture's own `init` failed:\n{}", out.0);
    dir
}

fn run(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_linix"))
        .args(args)
        .current_dir(dir)
        .env("LINIX_CONFIG_DIR", dir.join("config"))
        .env("LINIX_DATA_DIR", dir.join("data"))
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

/// The line the harnesses read, in the shape they read it.
#[test]
fn a_failing_command_names_its_failure_class() {
    let dir = fixture("failure-class-permanent");

    // A crate that does not exist: cargo says so, the policy calls it permanent, and no retry
    // anywhere can change that answer.
    let (out, code) = run(&dir, &["install", "cargo:linix-no-such-crate-zzz", "-y"]);
    assert_ne!(
        code, 0,
        "the fixture succeeded, so nothing was classified:\n{out}"
    );

    let class = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("linix-failure-class: "))
        .unwrap_or_else(|| {
            panic!(
                "a failing command printed no `linix-failure-class:` line. Both sweep harnesses \
                 read it and score a defect when it is missing, because its absence means the \
                 binary under test is not the tree that was built.\n{out}"
            )
        });

    assert_eq!(
        class.trim(),
        "permanent",
        "`cargo install <no-such-crate>` is permanent — a retry cannot make the crate exist. \
         Reporting it any other way sends the sweep harness back to retrying it.\n{out}"
    );
}

/// The vocabulary, whole. A token the harness does not expect is as bad as no token: its
/// `case` arms would fall through to the branch that scores a defect.
#[test]
fn the_class_vocabulary_is_the_four_the_harnesses_know() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(repo.join("src").join("main.rs")).unwrap();

    for token in ["transient", "permanent", "exhausted", "unknown"] {
        assert!(
            source.contains(&format!("\"{token}\"")),
            "`{token}` is a `Retryability` the program can produce and `main.rs` has no token \
             for it"
        );
    }

    // And both harnesses read the line, or the classification is computed for nobody again.
    for harness in [
        repo.join("scripts").join("integration-windows.sh"),
        repo.join("docker")
            .join("integration")
            .join("run-in-container.sh"),
    ] {
        let body = std::fs::read_to_string(&harness).unwrap();
        assert!(
            body.contains("linix-failure-class"),
            "{} does not read the failure class, so it is still guessing by retrying",
            harness.display()
        );
        // Comment lines skipped, and that is not pedantry: the first version of this check
        // matched a COMMENT quoting the historical output as an example, and reported a
        // harness that had already been fixed. A check that examines the wrong thing and
        // reports a finding is the same defect as one that reports success.
        let live = |needle: &str| {
            body.lines()
                .map(str::trim_start)
                .any(|l| !l.starts_with('#') && l.contains(needle))
        };
        assert!(
            !live("failed twice — a defect, not ecosystem variance"),
            "{} still scores a plain repeat as a defect. That sentence is what turned a rate \
             limit into a red macOS leg.",
            harness.display()
        );
        assert!(
            live("linix-failure-class"),
            "{} mentions the class only in prose — it has to read it",
            harness.display()
        );
    }
}

/// A command that succeeded must not print it. A line that appears on every run is a line a
/// script cannot use to tell a failure from a success.
#[test]
fn a_successful_command_prints_no_class_at_all() {
    let dir = fixture("failure-class-success");
    let (out, code) = run(&dir, &["eval"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !out.contains("linix-failure-class"),
        "`eval` succeeded and still announced a failure class:\n{out}"
    );
}

/// A refusal is not a failure, and it already has exit 3 to say so (U21). Printing a failure
/// class for it would give a script two answers to one question.
#[test]
fn a_refusal_is_not_given_a_failure_class() {
    let dir = fixture("failure-class-refusal");
    let (out, code) = run(&dir, &["reset"]);
    assert_eq!(
        code, 3,
        "`reset` with no terminal should refuse at 3; this fixture measures nothing \
         otherwise:\n{out}"
    );
    assert!(
        !out.contains("linix-failure-class"),
        "a refusal was given a failure class. Exit 3 already says what this is, and the \
         harnesses branch on it before they ever look for a class:\n{out}"
    );
}
