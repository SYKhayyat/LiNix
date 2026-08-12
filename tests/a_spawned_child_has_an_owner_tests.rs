//! **Every process Shall starts belongs to Shall.**
//!
//! There are three doors. `core::supervise::supervised_output` (and its `_fed` sibling)
//! captures the streams, bounds the silence, and stops the child on the way out.
//! `supervised_status` hands the terminal over for a program a person is watching, unbounded
//! because a shell at a prompt is not a hung command — but still owned. The third,
//! `core::blocking::command_output`, is for a `std::process::Command`, whose hazard is the
//! opposite one: it cannot be abandoned, so it holds a runtime worker until the child exits.
//! Everything inside `core::executor` itself is the mechanism, so it is exempt by construction.
//!
//! **This gate exists because seventeen sites had neither**, ten of which a hand-written grep
//! missed and this walk found on its first run. Awaiting `Command::output()` and then
//! dropping that future does not kill the process — tokio detaches it — so a `generate:` command
//! outlived the sync that asked for it, a hook outlived the node that fired it, and a secret
//! decrypt outlived its own timeout under a comment promising it would not. None of them was
//! bounded either, so one that blocked on a prompt blocked every sync on that machine forever,
//! silently.
//!
//! Fixing seventeen sites fixes seventeen. This is what stops the eighteenth: a `Command::new`
//! that reaches `spawn`/`output`/`status` outside the executor fails here until it either goes
//! through a door or explains, in a sentence, why it is not a child anybody can abandon.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::ledger::Ledger;

/// Sites that start a process without a door, and why that is right there.
///
/// A short reason is not a reason: the ledger enforces a length, because "special case" is what
/// every one of the seventeen would have said.
const UNOWNED_BY_DESIGN: &[(&str, &str)] = &[
    (
        "src/app/ui/history.rs",
        "the TUI has already left the alternate screen and handed the terminal to this command, \
         and the whole screen is already blocked on the person: the surrounding `run()` is one \
         synchronous event loop, and `core::on_the_terminal` at its call site is what keeps the \
         entire loop — this command included — off a runtime worker.",
    ),
    (
        "src/bin/shim.rs",
        "a separate binary with no async runtime at all: a shim's whole job is to become the \
         program its line named and wait for it, so there is no worker to park and no future to \
         drop. It links the library for the launcher, not for the executor.",
    ),
    (
        "src/backends/link_teardown_test.rs",
        "a test file that happens to live under `src/`, exercising `icacls` against a real \
         temporary directory. A fixture's child lasts as long as one test process, which is a \
         different problem from a package manager left running on somebody's laptop.",
    ),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Whether this file's own code — not its tests — starts a process.
///
/// The `#[cfg(test)]` tail is cut first. Test modules spawn helper binaries on purpose and are
/// not the subject: a fixture that leaks a child leaks it for the length of one test process,
/// which is a different problem from a package manager left running on somebody's laptop.
fn body_of(source: &str) -> &str {
    match source.find("\n#[cfg(test)]") {
        Some(i) => &source[..i],
        None => source,
    }
}

/// The offending shape: a `Command` that is actually started. Building one and handing it back
/// is not a spawn — `app/sandbox.rs` constructs four of them for a caller to run, and counting
/// those would train whoever reads this failure to ignore it.
fn starts_a_process(body: &str) -> Option<String> {
    if !body.contains("Command::new") {
        return None;
    }
    body.lines()
        .map(str::trim)
        .find(|line| {
            !line.starts_with("//")
                && !line.starts_with("///")
                && (line.contains(".spawn()")
                    || line.contains(".output()")
                    || line.contains(".status()")
                    || line.contains(".interact()"))
        })
        .map(str::to_string)
}

/// Whether the file gets its children through one of the three doors.
///
/// Two are for `tokio` children, whose hazard is detachment; the third is for
/// `std::process::Command`, whose hazard is the opposite — the call cannot be abandoned, so it
/// holds a runtime worker until the child exits.
fn uses_a_door(body: &str) -> bool {
    body.contains("supervised_output")
        || body.contains("supervised_output_fed")
        || body.contains("supervised_status")
        || body.contains("blocking::command_output")
        || body.contains("blocking::command_status")
}

#[test]
fn every_spawned_child_goes_through_a_door_or_says_why_not() {
    let root = repo_root().join("src");
    let mut files = Vec::new();
    rust_sources(&root, &mut files);

    let mut scanned = 0usize;
    let mut unowned: BTreeSet<String> = BTreeSet::new();
    let mut lines: Vec<(String, String)> = Vec::new();

    for path in &files {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        scanned += 1;
        let relative = path
            .strip_prefix(repo_root())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        // The executor is the mechanism. It is the one place allowed to hold a raw child.
        if relative == "src/core/executor.rs" {
            continue;
        }
        let body = body_of(&source);
        if let Some(line) = starts_a_process(body) {
            if !uses_a_door(body) {
                unowned.insert(relative.clone());
                lines.push((relative, line));
            }
        }
    }

    let detail = |site: &str| {
        lines
            .iter()
            .find(|(s, _)| s == site)
            .map(|(_, line)| format!("        {line}"))
    };

    Ledger::of("a process started with no owner", "UNOWNED_BY_DESIGN")
        .pairs(UNOWNED_BY_DESIGN)
        // Well under the ~250 files in `src/`, and far above the handful a broken walk reads.
        .scanning_at_least(120)
        .remedy(
            "Route it through `core::supervise::supervised_output` (captured, bounded, stopped) \
             or `supervised_status` (terminal handed over, unbounded, still stopped). A \
             `std::process::Command` goes through `core::blocking::command_output` instead: it \
             cannot be abandoned, but it holds a runtime worker until the child exits.",
        )
        .detailing(detail)
        .audit(scanned, &unowned);
}

/// **And the doors themselves are still there.** The gate above passes trivially if the names it
/// looks for stop existing — every site would read as "does not spawn", the walk would find
/// nothing, and a vacuous pass looks exactly like a clean one.
#[test]
fn the_doors_exist_and_own_what_they_start() {
    let executor = crate::harness::executor_source();
    for door in [
        "pub async fn supervised_output",
        "pub async fn supervised_output_fed",
        "pub async fn supervised_status",
    ] {
        assert!(
            executor.contains(door),
            "{door} is gone; the gate is vacuous"
        );
    }
    let blocking = std::fs::read_to_string(repo_root().join("src/core/blocking.rs"))
        .expect("the third door lives in core::blocking");
    for door in ["pub fn command_output", "pub fn command_status"] {
        assert!(
            blocking.contains(door),
            "{door} is gone; the gate is vacuous"
        );
    }
    // Ownership is the point of both, and `Stopping` is what provides it. `wait_watched` is
    // the shared body: if a door stopped going through it, the child would be raw again.
    assert!(
        executor.contains("struct Stopping"),
        "the guard that stops a child is gone"
    );
    assert!(
        executor.matches("Stopping::new").count() >= 2,
        "both doors must own their child, and only {} does",
        executor.matches("Stopping::new").count()
    );
}

/// **A package manager is never killed outright as a first resort.** SIGKILL cannot be caught,
/// so nothing gets to clean up: dpkg's database is left mid-write and pacman's `db.lck` is left
/// on disk, which is the wedged machine `heal` exists to unwedge. The order is the fix, so the
/// order is what is pinned.
#[test]
fn a_child_is_asked_to_stop_before_it_is_killed() {
    let executor = crate::harness::executor_source();
    // The ordering that matters is inside `stop`, not across the file: the grace constant is
    // declared far above it, so comparing first-occurrences in the whole file would pass or
    // fail on where a `const` happens to sit.
    let from = executor
        .find("async fn stop(")
        .expect("`Stopping::stop` is what sequences the two signals, and it is gone");
    let body = &executor[from..];
    let end = body.find("\n    }").expect("the end of `stop`");
    let body = &body[..end];

    let ask = body
        .find("request_stop")
        .expect("a catchable signal must be sent first, and `stop` sends none");
    let grace = body
        .find("TERMINATION_GRACE")
        .expect("a grace period, or the SIGTERM is decoration in front of a SIGKILL");
    let kill = body
        .find("start_kill")
        .expect("and a kill, for a child that will not stop");
    assert!(
        ask < grace && grace < kill,
        "SIGTERM must be asked first, then waited out, then escalated — `stop` has them at \
         ask {ask}, grace {grace}, kill {kill}:\n{body}"
    );
    assert!(
        executor.contains("libc::SIGTERM"),
        "`request_stop` must send SIGTERM specifically; SIGKILL cannot be caught, so a package \
         manager killed with it never unlinks its lock"
    );
}
