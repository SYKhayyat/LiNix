//! No command whose duration is a person's or a loop's holds the exclusive data lock for its
//! whole run.
//!
//! `Commands::writes` decides who takes the 120-second exclusive `DataLock` for the length of
//! the process, and its own doc comment records this bug being found and fixed **three times**:
//! `edit` blocks on `$EDITOR` and "once stopped every other Shall on the machine for as long as
//! somebody read a manifest in vim"; `fleet` "took the 120-second exclusive lock for a purely
//! remote report"; `history` "opens a TUI a person reads for as long as they like". Three
//! siblings with the identical shape were never touched:
//!
//! | command | how long it held the lock |
//! |---|---|
//! | `watch` | **forever** — an unbounded `loop`; "Ctrl-C to stop" |
//! | `shell` | the length of an interactive `$SHELL` session |
//! | `run`   | the length of an arbitrary user command |
//!
//! `watch` is the sharp one. It is the GitOps daemon and the documented deployment is to leave
//! it running — so `shall install`, `shall sync` and the `hook-reconcile` a hand-typed `apt
//! install` fires all waited 120 seconds and then failed, for as long as the daemon was up. A
//! user who followed the documentation disabled their own CLI.
//!
//! The fix is `LockScope`: a command answers `Reader`, `Writer` or `Deferred`, exhaustively, so
//! a seventh instance does not compile. These tests are the other half — that the answer
//! `Deferred` is honoured rather than merely declared.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::Parser;
use shall::cli::{Cli, LockScope};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every subcommand, with the scope it declares.
///
/// Walks clap for the names and `Cli::parse_from` for the variant, so the two cannot drift:
/// a name typed here that stops existing fails to parse, which is the failure mode two harness
/// exemption lists had for months while naming a subcommand called `undo` that was deleted.
fn scopes() -> Vec<(String, LockScope)> {
    let cmd = <Cli as clap::CommandFactory>::command();
    let mut out = Vec::new();
    for sub in cmd.get_subcommands() {
        let name = sub.get_name().to_string();
        if let Some(scope) = scope_of(&name) {
            out.push((name, scope));
        }
    }
    out
}

/// The scope of one subcommand, parsed through clap.
///
/// **Filler arguments, and the reason is `run`.** The first draft parsed each name bare and
/// skipped whatever would not parse — so `shall run`, which takes a command, was silently absent
/// from the walk, and the assertion that its name still exists passed against a set that did not
/// contain it. A table checked against a set that quietly excluded its subject is the exact
/// failure this file is about, one layer up.
fn scope_of(name: &str) -> Option<LockScope> {
    for filler in 0..3 {
        let mut argv = vec!["shall".to_string(), name.to_string()];
        argv.extend((0..filler).map(|i| format!("filler{i}")));
        if let Ok(cli) = Cli::try_parse_from(&argv) {
            return Some(cli.command.lock_scope());
        }
    }
    None
}

/// The commands whose duration is decided by something other than the package work they do.
///
/// A person at a keyboard, a loop with no end, or a program Shall does not own. This is a
/// statement about the *shape* of a verb, so it is written down; every entry is then checked
/// against clap (it must still exist) and against `lock_scope` (it must not be a whole-run
/// writer). Adding a seventh unbounded verb without answering `Deferred` or `Reader` fails here.
const UNBOUNDED: &[(&str, &str)] = &[
    (
        "watch",
        "an unbounded loop — the GitOps daemon, meant to be left running",
    ),
    ("shell", "launches $SHELL and awaits it"),
    ("run", "runs a command Shall neither wrote nor bounds"),
    (
        "history",
        "opens a TUI a person reads for as long as they like",
    ),
    ("edit", "blocks on $EDITOR (AU6)"),
    ("repl", "an interactive prompt"),
];

#[test]
fn no_unbounded_command_holds_the_lock_for_its_whole_run() {
    let scopes = scopes();
    let mut offenders = Vec::new();
    for (name, why) in UNBOUNDED {
        let scope = scopes.iter().find(|(n, _)| n == name).map(|(_, s)| *s);
        let Some(scope) = scope else {
            panic!(
                "`{name}` is named here as an unbounded command and the walk over clap's \
                 subcommands did not produce it; this table has rotted, which is the exact \
                 failure it exists to prevent"
            )
        };
        if scope == LockScope::Writer {
            offenders.push(format!("{name} — {why}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "{} command(s) take the 120-second exclusive data lock for their whole run, and their \
         run has no bound:\n  {}\n\nEvery other writing Shall command on the machine waits two \
         minutes behind them and then fails. Answer `LockScope::Deferred` and take the lock at \
         the mutating action, the way `history` does.",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// A `Deferred` answer is a promise that the command takes the lock somewhere else.
///
/// Without this, `Deferred` is indistinguishable from `Reader` — a command could be moved out
/// of the whole-run lock set and simply never lock at all, which is not the fix, it is the
/// other bug. Each deferred subcommand's name must appear in a `DataLock::for_one_step` call
/// in `src/`.
#[test]
fn every_deferred_command_takes_the_lock_somewhere() {
    let deferred: Vec<String> = scopes()
        .into_iter()
        .filter(|(_, s)| *s == LockScope::Deferred)
        .map(|(n, _)| n)
        .collect();
    assert!(
        !deferred.is_empty(),
        "no command answers `LockScope::Deferred`; the scope this file is about has been \
         deleted, so re-derive the finding rather than deleting the test"
    );

    // Every `for_one_step("…")` argument in the tree, read from source. The call is the only
    // way a deferred command reaches the lock, so its literal is the ledger.
    let mut steps: Vec<String> = Vec::new();
    for entry in walk(&repo().join("src")) {
        let Ok(src) = std::fs::read_to_string(&entry) else {
            continue;
        };
        for (i, m) in src.match_indices("for_one_step(\"") {
            let rest = &src[i + m.len()..];
            if let Some(end) = rest.find('"') {
                steps.push(rest[..end].to_string());
            }
        }
    }
    assert!(
        !steps.is_empty(),
        "no `DataLock::for_one_step(\"…\")` call was found in src/; this extraction is looking \
         at the wrong thing"
    );

    let unlocked: Vec<&String> = deferred
        .iter()
        .filter(|d| {
            !steps
                .iter()
                .any(|s| s == *d || s.starts_with(&format!("{d} ")))
        })
        .collect();
    assert!(
        unlocked.is_empty(),
        "{:?} answer `LockScope::Deferred` and never take the lock at all. Deferred means the \
         lock moves to the write, not that it disappears — a writer with no lock is two writers \
         making a removal out of a race (II.8).\n  for_one_step call sites: {:?}",
        unlocked,
        steps
    );
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// The measurement, not the declaration: `watch` is running, and a second Shall can still write.
///
/// This is the finding reproduced. Before the fix, taking the data lock while `watch` was up
/// waited the full 120 seconds and then failed with "the Shall data directory is locked by
/// shall watch". It now succeeds between ticks.
#[test]
fn a_running_watch_does_not_stop_the_next_writer() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("watch-lock-scope");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config/modules")).unwrap();
    std::fs::create_dir_all(dir.join("config/profiles")).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    // No backends: a tick has nothing to ask and nothing to install, so what is measured is the
    // lock's scope and not how long a package manager takes.
    std::fs::write(dir.join("config/priority"), "\n").unwrap();
    std::fs::write(dir.join("config/active"), "").unwrap();

    let mut watch = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(["-y", "watch", "--interval", "1"])
        .env("SHALL_CONFIG_DIR", dir.join("config"))
        .env("SHALL_DATA_DIR", dir.join("data"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the binary should run");

    // Give it long enough to have reconciled at least once, so this is not measuring a daemon
    // that has not started yet.
    let started = Instant::now();
    let mut took: Option<Duration> = None;
    while started.elapsed() < Duration::from_secs(45) {
        std::thread::sleep(Duration::from_millis(200));
        if matches!(watch.try_wait(), Ok(Some(_))) {
            break;
        }
        // A short wait on purpose: the question is whether the lock is free *between* ticks,
        // and a two-minute wait would answer "eventually" for a daemon that never releases it.
        let t0 = Instant::now();
        if shall::core::datalock::DataLock::acquire(
            &dir.join("data"),
            "the lock-scope measurement",
            Duration::from_secs(3),
        )
        .is_ok()
        {
            took = Some(t0.elapsed());
            break;
        }
    }
    let _ = watch.kill();
    let _ = watch.wait();

    assert!(
        took.is_some(),
        "`shall watch` was running and the data lock never came free in 45 seconds. That is the \
         defect: the documented GitOps deployment disables every other writing Shall command on \
         the machine for as long as the daemon is up. `watch` must take the lock per tick \
         (`LockScope::Deferred`), not for its lifetime."
    );
}
