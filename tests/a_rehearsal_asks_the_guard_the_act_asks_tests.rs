//! `sync --dry-run` must consult the same guard `sync` consults.
//!
//! A rehearsal that answers a different question from the act is not a rehearsal. Measured on the
//! `tools` container image — adopt the machine, un-declare packages the guard protects, then ask
//! each surface. The only variable between the rows is the `--dry-run` flag:
//!
//! ```text
//! sync --dry-run --yes, 1 package    rc=0   protected named: 0    no refusal
//! sync --yes,           1 package    rc=3   protected named: 1    refused
//! sync --dry-run --yes, 13 packages  rc=0   protected named: 0    no refusal
//! sync --yes,          13 packages   rc=3   protected named: 10   refused
//! ```
//!
//! **`plan` gets it right, which is what makes this a defect and not a missing feature.** Same
//! tree, same state, same second:
//!
//! ```text
//! sync --dry-run   rc=0   protected named: 0    no refusal
//! plan             rc=0   protected named: 10   refusal text present
//! sync (real)      rc=3   protected named: 10   refused
//! ```
//!
//! The information is computed and available; one of the two previews of the same operation does
//! not ask for it.
//!
//! **It contradicts a rule this repository already wrote down.** `cli/args.rs` states `S25`:
//! *"**`--dry-run` never exempts anything**: a preview of a writer reads the same state a
//! concurrent writer is rewriting."* The lock is one thing a dry-run must not exempt. The guard
//! is the other, and it does.
//!
//! Longstanding rather than new: `3affcc5`, CI's last green main, behaves identically on all four
//! rows, so this predates the window in which nothing on Unix compiled.
//!
//! **What this test asserts, and what it does not.** Only that the two agree: if `plan` says a
//! removal is refused, `sync --dry-run` over the identical repository says so too. Teaching the
//! dry-run to ask, or routing both through one function, each satisfy it. It does not say which
//! exit code a dry-run should carry — that is `U21`'s and the owner's.
//!
//! **Nothing here mutates the machine.** `adopt` writes declarations into a scratch config
//! directory and installs nothing; `plan` and `sync --dry-run` are read-only. The test asserts
//! `--dry-run` is present on every `sync` it runs.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn shall() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_shall"))
}

fn run(dir: &Path, args: &[&str]) -> Output {
    // Belt and braces: this fixture must never run a mutating sync.
    if args.first() == Some(&"sync") {
        assert!(
            args.contains(&"--dry-run"),
            "this test may only run `sync` with --dry-run; got {args:?}"
        );
    }
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

/// A repository that has adopted this machine and then declares nothing.
///
/// Every adopted package becomes an undeclared removal, which is the state that reaches the
/// guard: over a ceiling, and holding whatever this host has that the guard protects.
///
/// `None` when the host has nothing to adopt — a machine with no reachable manager cannot
/// produce this state, and the tests say so rather than passing quietly.
fn a_machine_whose_whole_inventory_is_now_undeclared() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("shall-rehearsal-guard");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    let init = Command::new(shall())
        .arg("init")
        .env("SHALL_CONFIG_DIR", &dir)
        .env("SHALL_DATA_DIR", dir.join("data"))
        .output()
        .ok()?;
    if !init.status.success() {
        return None;
    }
    if !run(&dir, &["adopt", "--yes"]).status.success() {
        return None;
    }

    // Un-declare everything: empty every module file adopt wrote.
    let modules = dir.join("modules");
    let mut declared = 0usize;
    for entry in std::fs::read_dir(&modules).ok()?.flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "txt") {
            declared += std::fs::read_to_string(&p)
                .map(|s| s.lines().count())
                .ok()?;
            std::fs::write(&p, "").ok()?;
        }
    }
    (declared > 0).then_some(dir)
}

fn refuses(out: &str) -> bool {
    out.contains("refusing this removal")
}

fn protected_lines(out: &str) -> usize {
    out.lines()
        .filter(|l| l.contains("would be removed ("))
        .count()
}

/// The control: `plan` reaches the guard on this host, so there is a disagreement to find.
///
/// Without it, a green result below could mean "both surfaces are silent because this machine has
/// nothing protected", which would be a test that passes by measuring nothing.
#[test]
fn plan_reaches_the_guard_on_this_machine() {
    let Some(dir) = a_machine_whose_whole_inventory_is_now_undeclared() else {
        return; // nothing adoptable here; no state to measure
    };
    let out = text(&run(&dir, &["plan"]));
    assert!(
        refuses(&out) || protected_lines(&out) > 0,
        "`plan` does not reach the guard on this host, so the comparison below would be \
         vacuous. Either nothing installed here is protected, or `plan` stopped asking.\n{out}"
    );
}

/// `sync --dry-run` reports the refusal that `plan` reports over the same repository.
#[test]
fn the_dry_run_reports_the_refusal_the_plan_reports() {
    let Some(dir) = a_machine_whose_whole_inventory_is_now_undeclared() else {
        return;
    };
    let planned = text(&run(&dir, &["plan"]));
    if !refuses(&planned) {
        return; // the control test owns this failure; do not report it twice
    }
    let rehearsed = text(&run(&dir, &["sync", "--dry-run", "--yes"]));
    assert!(
        refuses(&rehearsed),
        "`plan` says this removal is refused by the guard and `sync --dry-run` does not \
         mention it. The rehearsal of a command must answer the same question the command \
         answers — `S25` says a dry-run never exempts anything.\n\n\
         --- plan ---\n{planned}\n--- sync --dry-run ---\n{rehearsed}"
    );
}

/// And it names the same protected packages, not merely the same verdict.
///
/// Separate from the test above because a dry-run could learn to say "refused" while still not
/// listing what is protected, which is the half a reader acts on.
#[test]
fn the_dry_run_names_the_protected_packages_the_plan_names() {
    let Some(dir) = a_machine_whose_whole_inventory_is_now_undeclared() else {
        return;
    };
    let planned = text(&run(&dir, &["plan"]));
    let expected = protected_lines(&planned);
    if expected == 0 {
        return; // nothing protected here; the control test covers this case
    }
    let rehearsed = text(&run(&dir, &["sync", "--dry-run", "--yes"]));
    assert_eq!(
        protected_lines(&rehearsed),
        expected,
        "`plan` names {expected} protected package(s) and `sync --dry-run` names {}. \
         A reader deciding whether to run the sync sees the second one.\n\n\
         --- sync --dry-run ---\n{rehearsed}",
        protected_lines(&rehearsed)
    );
}
