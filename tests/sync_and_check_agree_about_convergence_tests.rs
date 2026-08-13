//! When `sync` reports the machine converged, `check` must agree that it matches.
//!
//! `Exit::Converged` is the name of the code `sync` returns on success — not "the run finished",
//! not "no command failed", but *converged*. On a machine where a declaration was skipped, `sync`
//! returns it while `check`, run on the identical state one line later, reports drift:
//!
//! ```text
//! $ sudo -n env SHALL_CONFIG_DIR=… shall sync --yes ; echo $?
//!  WARN `ripgrep` is declared for `cargo`, which is not on this machine — skipping it.
//!  WARN `left-pad` is declared for `bun`, which is not on this machine — skipping it.
//!  WARN `cowsay` is declared for `uv`, which is not on this machine — skipping it.
//! 0
//! $ sudo -n env SHALL_CONFIG_DIR=… shall check ; echo $?
//! ->  drift  3 package(s) ...
//! 2
//! ```
//!
//! Three declarations, none installed, no transaction summary printed at all, and the exit code
//! a scheduled run reads says the machine matches its files.
//!
//! **This test picks no side, and deliberately.** Whether a skipped declaration should make
//! `sync` exit non-zero is `U21`'s question and the owner's to answer — `docs/spec/decisions.md`
//! owns it and `CLAUDE.md` forbids answering it here. What is not a question is that the two
//! commands cannot both be right about one machine at one moment. `target-state.md` §Q2 already
//! rules that shape for `check`'s own two views — *"two counts of one machine will disagree"* —
//! and this is that principle one command wider. Fixing either side satisfies this test.
//!
//! **Why the exit code is the thing that matters here.** `shall schedule` and `shall fleet` exist,
//! so an unattended `sync` is a supported use, and an unattended run is exactly the environment
//! where a manager goes missing: `sudo` ships `secure_path` without `~/.cargo/bin`, `~/.bun/bin`
//! or `~/.local/bin`, so `cargo`, `bun` and `uv` are invisible to anything run through it. The
//! warnings above are printed and useful to a person watching. Nothing is watching.
//!
//! **On running a real `sync`.** `GRADER.md` forbids validating a mutating path on a machine
//! someone uses, and this is not one: the fixture's only declaration is for a manager this host
//! does not have, and the test asserts `plan` reports **0 installs and 0 removals** before it
//! runs anything. A `sync` over an empty change set performs no work — that is the whole point,
//! and it is why it is safe to ask what it returns.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Managers to try as "in `priority`, not on this host", as the sibling fixtures do.
const CANDIDATES: &[&str] = &[
    "apt", "pacman", "apk", "xbps", "zypper", "dnf", "emerge", "choco", "winget", "brew", "nix",
];

fn shall() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_shall"))
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(shall())
        .args(args)
        .env("SHALL_CONFIG_DIR", dir)
        .env("SHALL_DATA_DIR", dir.join("data"))
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A repository whose only declaration names a manager `priority` lists and this host lacks.
///
/// Returns `None` when every candidate is installed here, in which case there is nothing on this
/// machine to measure.
fn one_declaration_this_host_cannot_serve(name: &str) -> Option<(PathBuf, String)> {
    for candidate in CANDIDATES {
        let dir = std::env::temp_dir().join(format!("shall-converged-{name}-{candidate}"));
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
        let existing = std::fs::read_to_string(&priority).unwrap_or_default();
        if existing.lines().any(|l| l.trim() == *candidate) {
            continue; // present here; nothing would be skipped
        }
        std::fs::write(&priority, format!("{existing}\n{candidate}\n")).unwrap();
        std::fs::write(
            dir.join("modules").join("starter.txt"),
            format!("{candidate}:tree\n"),
        )
        .unwrap();
        if text(&run(&dir, &["plan"])).contains("is not on this machine") {
            return Some((dir, (*candidate).to_string()));
        }
    }
    None
}

/// Does this plan carry an empty change set?
///
/// `plan` spells that two ways — `no actions` when there is nothing at all, and an explicit
/// `0 install(s), 0 removal(s)` when there are resources but no packages — and the safety guard
/// below has to accept both. It was written against only the second spelling and correctly
/// refused to run `sync`, which is the behaviour a guard whose reading is uncertain should have.
fn plans_nothing(out: &str) -> bool {
    out.contains("— no actions.") || out.contains("0 install(s), 0 removal(s)")
}

/// The safety precondition, asserted rather than assumed: this fixture plans no work at all.
///
/// If this ever stops holding, the tests below would be running a mutating `sync` with real
/// changes in it on whatever machine the suite is on, which `GRADER.md` forbids and which is not
/// what any of this is measuring.
#[test]
fn the_fixture_plans_no_changes_at_all() {
    let Some((dir, manager)) = one_declaration_this_host_cannot_serve("safety") else {
        return;
    };
    let out = text(&run(&dir, &["plan"]));
    assert!(
        plans_nothing(&out),
        "the fixture for `{manager}` plans real work, so the `sync` below would perform it. \
         This test must not run a mutating command with changes in it.\n{out}"
    );
}

/// The control: `check` genuinely reports drift here, so the disagreement below is one.
#[test]
fn check_reports_drift_on_this_machine() {
    let Some((dir, manager)) = one_declaration_this_host_cannot_serve("control") else {
        return;
    };
    let out = text(&run(&dir, &["check"]));
    let drift = out
        .lines()
        .find(|l| l.contains("drift"))
        .unwrap_or_default();
    assert!(
        !drift.starts_with("ok"),
        "`check` already considers this machine converged, so there is no disagreement for the \
         next test to report. `{manager}:tree` is declared and not installed.\nrow: {drift}"
    );
}

/// `sync` does not report convergence over a machine `check` calls drifted.
///
/// The two runs are back to back over one unchanged state. Either `sync` should not answer
/// `Converged`, or `check` should not answer drift — this test does not say which.
#[test]
fn sync_does_not_report_converged_while_check_reports_drift() {
    let Some((dir, manager)) = one_declaration_this_host_cannot_serve("agree") else {
        return;
    };
    // Safety again, at the point of use: the guard above is a separate test and a separate run.
    let planned = text(&run(&dir, &["plan"]));
    assert!(
        plans_nothing(&planned),
        "refusing to run `sync`: this fixture plans work.\n{planned}"
    );

    let synced = run(&dir, &["sync", "--yes"]);
    let sync_code = synced.status.code();
    let after = text(&run(&dir, &["check"]));
    let drift = after
        .lines()
        .find(|l| l.contains("drift"))
        .unwrap_or_default();

    assert!(
        sync_code != Some(0) || drift.starts_with("ok"),
        "`sync --yes` returned {sync_code:?} — `Exit::Converged` — and `check`, over the same \
         unchanged state, reports drift. `{manager}:tree` is declared and is not installed; \
         `sync` printed no transaction summary and skipped it.\n\
         sync output:\n{}\ncheck drift row: {drift}",
        text(&synced)
    );
}
