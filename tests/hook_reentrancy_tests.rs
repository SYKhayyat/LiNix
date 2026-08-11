//! A hook Shall installs into a package manager must stand down when Shall started that
//! manager — **every** such hook, not the one somebody looked at.
//!
//! `main` stood down for `hook-reconcile` alone, under a comment naming the hazard exactly:
//! *"it holds the lock this process would wait two minutes for."* That is what apt, dnf,
//! zypper, apk, xbps, portage and eopkg invoke. It is not what pacman invokes — Shall installs
//! `hook-record` as pacman's `PostTransaction` hook (`app/pm_hooks.rs`) — and it is not what
//! the shell wrappers invoke, which is `hook-observe`. Both are writers by `Commands::writes`,
//! so both take the 120-second exclusive `DataLock` *inside* the sync that is holding it:
//!
//! ```text
//! shall sync  (holds DataLock)
//!   └─ pacman -S foo                    ← spawned with SHALL_INSIDE set
//!        └─ PostTransaction hook
//!             └─ shall hook-record …    ← ignored SHALL_INSIDE, blocked for the full 120s
//! ```
//!
//! pacman waits on its own hook for the whole two minutes, nothing kills it, and the record it
//! was going to write is lost at the end of it anyway.
//!
//! **The assertions here are enumerations, not a list of three names.** One walks clap for
//! every `hook-*` subcommand and drives it; the other walks `app/pm_hooks.rs` for every hook
//! body Shall writes into a manager's configuration and checks the subcommand it names is one
//! of those. A fourth hook subcommand fails both until it inherits the guard, and a hook body
//! that calls something else fails the second.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `hook-*` subcommand clap knows about, as an argv that parses.
///
/// Built from clap's own metadata rather than typed out, because a hand-written argv per
/// subcommand is the list this test exists to stop trusting. Required options get a
/// placeholder value; everything else is left off.
fn hook_invocations() -> Vec<Vec<String>> {
    let cmd = <shall::cli::Cli as clap::CommandFactory>::command();
    let mut out = Vec::new();
    for sub in cmd.get_subcommands() {
        let name = sub.get_name().to_string();
        if !name.starts_with("hook-") {
            continue;
        }
        let mut argv = vec![name];
        for arg in sub.get_arguments() {
            if !arg.is_required_set() {
                continue;
            }
            let Some(long) = arg.get_long() else { continue };
            argv.push(format!("--{long}"));
            if matches!(
                arg.get_action(),
                clap::ArgAction::Set | clap::ArgAction::Append
            ) {
                argv.push("harness".to_string());
            }
        }
        out.push(argv);
    }
    assert!(
        out.len() >= 3,
        "clap reports {} hook subcommands; the three this guard was written for are \
         hook-record, hook-reconcile and hook-observe, so the walk is looking at the wrong thing",
        out.len()
    );
    out
}

/// A data directory whose lock this test already holds, and a config directory to read.
///
/// The lock is taken here rather than by a spawned `shall` on purpose. No verb holds it for its
/// whole lifetime any more — that was its own defect, and `watch` was the worst instance — so a
/// fixture built out of a long-running command would be measuring which verb still over-locks
/// rather than whether a hook stands down. `DataLock` is the thing under test; take it directly.
struct HeldLock {
    dir: PathBuf,
    _lock: shall::core::datalock::DataLock,
}

fn hold_the_data_lock(name: &str) -> HeldLock {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config/modules")).unwrap();
    std::fs::create_dir_all(dir.join("config/profiles")).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    // No backends in `priority`, so a hook that does NOT stand down has nothing to ask and
    // still blocks — the wait is on the lock, before any manager is reached.
    std::fs::write(dir.join("config/priority"), "\n").unwrap();
    std::fs::write(dir.join("config/active"), "").unwrap();

    let lock = shall::core::datalock::DataLock::acquire(
        &dir.join("data"),
        "the hook re-entrancy fixture",
        Duration::from_secs(5),
    )
    .expect("a fresh data directory's lock is free");
    HeldLock { dir, _lock: lock }
}

/// Run one argv with `SHALL_INSIDE` set, against a data directory somebody else is holding.
///
/// Bounded well under the 120-second lock timeout: a run that stands down answers in under a
/// second, and a run that waits is the defect. Returns `None` if it was still going at the
/// bound, which is the failure the assertion reports.
fn run_bounded(dir: &std::path::Path, argv: &[String], bound: Duration) -> Option<(i32, Duration)> {
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(argv)
        .env("SHALL_CONFIG_DIR", dir.join("config"))
        .env("SHALL_DATA_DIR", dir.join("data"))
        .env("SHALL_INSIDE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the binary should run");
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some((status.code().unwrap_or(-1), started.elapsed())),
            Ok(None) => {}
            Err(e) => panic!("waiting on the child failed: {e}"),
        }
        if started.elapsed() >= bound {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn every_hook_subcommand_stands_down_when_shall_started_the_manager() {
    let held = hold_the_data_lock("hook-reentrancy");
    // Ten seconds. The lock's own timeout is 120s, so anything that waits on it fails this
    // bound by a wide margin, and a stand-down does not reach a lock at all.
    let bound = Duration::from_secs(10);
    let mut waited: Vec<String> = Vec::new();
    for argv in hook_invocations() {
        match run_bounded(&held.dir, &argv, bound) {
            Some((code, took)) if took < bound => {
                assert_eq!(
                    code,
                    0,
                    "`shall {}` stood down but exited {code}; standing down is not a failure — \
                     the run that spawned this one is already recording what it installed",
                    argv.join(" ")
                );
            }
            _ => waited.push(argv.join(" ")),
        }
    }
    assert!(
        waited.is_empty(),
        "{} hook subcommand(s) blocked on the data lock instead of standing down under \
         SHALL_INSIDE:\n  {}\n\nEach one is what a package manager Shall is driving invokes \
         mid-transaction, so this is 120 seconds of silence per transaction and a lost record. \
         The guard is `Commands::is_manager_hook`.",
        waited.len(),
        waited.join("\n  ")
    );
}

/// Every subcommand `app/pm_hooks.rs` writes into a manager's hook file is one the guard covers.
///
/// The other direction of the same rule. `is_manager_hook` answers from clap's name, so this is
/// what stops a hook body being written that calls `shall sync` — or a hook subcommand being
/// renamed to something that no longer matches — while the enumeration above still passes.
#[test]
fn every_hook_shall_installs_stands_down() {
    let src = std::fs::read_to_string(repo().join("src/app/pm_hooks.rs"))
        .expect("src/app/pm_hooks.rs is gone; re-derive this finding");
    let known: Vec<String> = <shall::cli::Cli as clap::CommandFactory>::command()
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();

    // Every `{shall_bin} <word>` in a hook body: the binary placeholder is how each of those
    // format strings names Shall, so this finds the invocation without knowing the shape of
    // the file the manager wants.
    let mut invoked: Vec<String> = Vec::new();
    for (_, rest) in src
        .match_indices("{shall_bin} ")
        .map(|(i, m)| (i, &src[i + m.len()..]))
    {
        let word: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !word.is_empty() && !invoked.contains(&word) {
            invoked.push(word);
        }
    }
    assert!(
        !invoked.is_empty(),
        "no `{{shall_bin}} <subcommand>` was found in pm_hooks.rs; this extraction is looking \
         at the wrong file"
    );

    // **Not every `{shall_bin}` in this file is a hook body.** The shell-integration snippet
    // opens with a comment telling the reader how to source it — `eval "$(shall hooks
    // shell-init bash)"` — and that line is run by a person's rc file, not by a package
    // manager. Listed rather than filtered by shape, because "it looked like a comment" is the
    // kind of heuristic that quietly stops matching; a second non-hook invocation has to be
    // added here on purpose, with a reason.
    const NOT_A_HOOK_BODY: &[(&str, &str)] = &[(
        "hooks",
        "the shell-integration snippet's own header comment, telling the user which command \
         emits it — evaluated by their rc file, never by a manager",
    )];

    let mut strays: Vec<String> = Vec::new();
    for word in &invoked {
        if NOT_A_HOOK_BODY.iter().any(|(w, _)| w == word) {
            continue;
        }
        if !known.contains(word) {
            strays.push(format!("{word} (not a subcommand at all)"));
        } else if !word.starts_with("hook-") {
            strays.push(format!("{word} (a subcommand, and not a hook one)"));
        }
    }
    assert!(
        strays.is_empty(),
        "pm_hooks.rs writes {:?} into a package manager's hook file, and nothing there is \
         covered by `Commands::is_manager_hook`. A hook fired by a manager Shall is driving \
         must stand down, or it blocks 120s on a lock its own parent holds.\n  \
         everything the file invokes: {:?}",
        strays,
        invoked
    );

    // And the other direction: a hook subcommand clap knows and no manager is ever told to
    // call is a subcommand nothing installs — either dead, or a hook body that was forgotten.
    let installed: Vec<&String> = invoked.iter().filter(|w| w.starts_with("hook-")).collect();
    let uninstalled: Vec<&String> = known
        .iter()
        .filter(|k| k.starts_with("hook-"))
        .filter(|k| !installed.contains(k))
        .collect();
    assert!(
        uninstalled.is_empty(),
        "{uninstalled:?} are hook subcommands and `pm_hooks.rs` never writes them into any \
         manager's hook file. Either a hook body is missing, or the subcommand is dead and the \
         guard above is protecting something nothing invokes."
    );
}
