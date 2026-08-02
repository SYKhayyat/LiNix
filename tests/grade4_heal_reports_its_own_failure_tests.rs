//! BUILDER round 6, W36 / R-5's sibling R-6 — `heal` reported an operation it could not
//! recover at rc=0, in Rust's `Debug` syntax, with advice its own classifier contradicts.
//!
//! Measured by the round-5 grader, an `InProgress` install planted for a package that does not
//! exist:
//!
//!     ERROR could not recover npm:… — Some(CommandFailed { message: "…404…",
//!     retry: Permanent, absent_name: true }). The system may be in a partial state for this
//!     package; re-run `linix sync`.
//!      WARN 1 operation(s) could NOT be recovered: npm:… . Re-run `linix sync`.
//!     heal: reconciled locks/versions.json (1 entries)
//!     heal: refreshed backend metadata
//!     heal rc=0
//!
//! Three defects, and the behaviour underneath is right and must stay right: `heal` attempts
//! the recovery, fails, and **leaves the entry `InProgress`** rather than closing it. That is
//! the answer a "mark everything done" implementation gets wrong.
//!
//!   1. rc=0 after "1 operation(s) could NOT be recovered". `linix heal && echo ok` printed ok.
//!   2. `{:?}` on an `Option<Error>` printed at the user — `retry: Permanent`,
//!      `absent_name: true`, internal field names and all.
//!   3. The advice contradicted the struct it had just printed: `absent_name: true` means the
//!      name does not exist, and it said "re-run `linix sync`".
//!
//! **The fixture is `cargo uninstall` of a crate this machine does not have.** The grader
//! recorded that it could not automate this, because a planted `Install` entry needs a network
//! round-trip and a hand-written one omitting `options` lands in the corrupt-WAL branch instead
//! of the recovery branch. A planted **Remove** parses (the dry-run gate already relies on
//! that), and `cargo` is on every runner that builds LiNix, so its recovery fails locally,
//! deterministically, and with no network at all.
//!
//! `github:` was tried first and does **not** work: removing an artifact the lock has never
//! heard of succeeds, so there is nothing to fail. That is recorded because a fixture that
//! silently recovers would make every assertion below vacuous.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The package `heal` will try, and fail, to remove.
const ABSENT: &str = "linix-probe-not-installed-zzz";

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

/// A fixture whose journal records one interrupted removal that cannot succeed.
fn fixture(name: &str, backend: &str, package: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let (out, code) = run(&root, &["init"]);
    assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");

    std::fs::write(
        root.join("data").join("journal.jsonl"),
        format!(
            r#"{{"{backend}:{package}:wal": {{
                "id": "{backend}:{package}:wal",
                "action": {{"Remove": {{"name": "{package}", "backend": "{backend}"}}}},
                "status": "InProgress",
                "started_at_unix": 1000000,
                "finished_at_unix": null,
                "error": null,
                "staged_properties": {{}}
            }}}}"#
        ),
    )
    .unwrap();
    root
}

/// The control, and the reason every assertion below means something: the fixture really does
/// produce a recovery that fails. A host where `cargo uninstall` of an absent crate somehow
/// succeeds is **skipped and named**, never passed.
fn heal_that_could_not_recover(tag: &str) -> Option<(String, i32)> {
    let dir = fixture(tag, "cargo", ABSENT);
    let (out, code) = run(&dir, &["heal", "-y"]);
    if !out.contains("could not recover") {
        eprintln!(
            "skipped: this host recovered the planted entry, so there is no failure to report \
             on:\n{out}"
        );
        return None;
    }
    // This planted entry is recovered by shelling out to `cargo`, and cargo takes a lock on its
    // package cache. Under `cargo test` — which holds that lock — the child answers `Blocking
    // waiting for file lock on crate metadata` and fails on the lock rather than on the absent
    // crate, so what LiNix classified is a different failure from the one under test. Measured:
    // this target failed inside a full `cargo test --no-fail-fast` run and passed alone, on the
    // same tree, twice.
    //
    // Named and skipped rather than asserted-around: the assertions below are about the advice
    // LiNix gives for a PERMANENT failure, and a lock wait is not one. It cannot hide the
    // defect — the skip fires only on cargo's own lock sentence.
    if out.contains("waiting for file lock") {
        eprintln!(
            "skipped: another cargo holds the package-cache lock, so the recovery failed on the \
             lock rather than on the absent crate — a different failure from the one this \
             measures:\n{out}"
        );
        return None;
    }
    Some((out, code))
}

#[test]
fn heal_does_not_exit_zero_after_failing_to_recover() {
    let Some((out, code)) = heal_that_could_not_recover("grade4-heal-rc") else {
        return;
    };

    assert_ne!(
        code, 0,
        "`heal` said it could not recover an operation and then exited 0, so \
         `linix heal && echo ok` prints ok. U21 gave this program an exit vocabulary and the \
         recovery path was the last one not using it.\n\n{out}"
    );
}

/// The last two lines a user saw were successes. They still may be — the environment repairs
/// are real and should be reported — but the run as a whole must not read as one.
#[test]
fn heal_does_not_print_rust_debug_syntax_at_the_user() {
    let Some((out, _)) = heal_that_could_not_recover("grade4-heal-debug") else {
        return;
    };

    let leaked: Vec<&str> = [
        "Some(",
        "CommandFailed {",
        "retry:",
        "absent_name",
        "staged_properties",
        "InProgress",
    ]
    .into_iter()
    .filter(|needle| out.contains(needle))
    .collect();

    assert!(
        leaked.is_empty(),
        "`heal` printed internal vocabulary at the user: {leaked:?}\n\n`absent_name` is a \
         struct field the N-1 fix introduced; `Some(CommandFailed {{ … }})` is `{{:?}}` on an \
         `Option<Error>`. GRADER §4 asks for every place internal vocabulary leaks.\n\n{out}"
    );
}

/// The behaviour underneath, which is right and which this whole order must not disturb: the
/// entry is **left `InProgress`** rather than closed, so the next `heal` tries it again and
/// nothing claims the package is installed.
///
/// Deliberately does **not** take the package-cache-lock skip the helper carries: this asserts
/// that a failed recovery stays open and is retried, which is true whatever the recovery failed
/// on. Only the assertions about the ADVICE are invalidated by failing on a lock instead of on
/// the absent crate.
#[test]
fn a_failed_recovery_leaves_the_entry_open() {
    let dir = fixture("grade4-heal-open", "cargo", ABSENT);
    let (out, _) = run(&dir, &["heal", "-y"]);
    if !out.contains("could not recover") {
        eprintln!("skipped: this host recovered the planted entry");
        return;
    }

    let journal =
        std::fs::read_to_string(dir.join("data").join("journal.jsonl")).unwrap_or_default();
    assert!(
        journal.contains("InProgress"),
        "the failed recovery closed the entry anyway, which is the answer a \"mark everything \
         done\" implementation gives. The journal now reads:\n{journal}"
    );

    // And a second `heal` still finds it — the property that makes leaving it open worth
    // anything.
    let (again, code) = run(&dir, &["heal", "-y"]);
    assert!(
        again.contains("could not recover"),
        "the second `heal` did not retry the entry the first one left open:\n{again}"
    );
    assert_ne!(
        code, 0,
        "the second `heal` reported the same failure and exited 0:\n{again}"
    );
}

/// The advice must be driven by the classification the error already carries, not by one
/// sentence for every case. `cargo uninstall` of something not installed is `Permanent`, so
/// "re-run and it may work" is the wrong half.
#[test]
fn the_advice_follows_the_classification() {
    let Some((out, _)) = heal_that_could_not_recover("grade4-heal-advice") else {
        return;
    };

    let line = out
        .lines()
        .find(|l| l.contains("could not recover"))
        .expect("the control asserted this line exists");

    assert!(
        line.contains("will fail the same way") || line.contains("does not exist"),
        "a permanent failure was advised as though another attempt might help. LiNix classified \
         it and then did not consult the classification — which is R-3's defect in a third \
         place.\n\n{line}"
    );
}
