//! Does every subcommand LiNix invokes still exist in the tool it invokes it on?
//!
//! `pixi global upgrade-all` was removed upstream and stayed in this tree, invisible, because
//! the only thing testing it was a plan-smoke — and **a plan-smoke proves an argv was
//! *constructed*, never that it is *correct***. The argv was built perfectly every time. Nobody
//! ever asked pixi.
//!
//! So this asks. It drives every registered backend through a mock executor, reads the argv
//! LiNix would really have run, and walks each subcommand chain against that manager's own
//! `--help` on this machine. It converts silent upstream drift into a named failure, which is
//! the difference between fixing `pixi` today and fixing its successor automatically.
//!
//! **It reads the argv from the code, not from a list.** A written-down table of "what LiNix
//! invokes" is a second copy of the truth, and the second copy is what goes stale — which is
//! this defect's own shape, one level up.
//!
//! Only managers actually installed here can be checked. The rest are reported as skipped, by
//! name, because a gate that silently covers eight of fifty and prints a pass is the thing
//! `READINESS` is about.
//!
//! A manager that IS installed and cannot be asked is a **failure**, not a skip. That
//! distinction is the whole difference between a gate and a report: `scoop`, `npm`, `gem`,
//! `pipx` and `yarn` are all on this machine and were all skipped as "its help could not be
//! read", because this file launched them with a raw `Command` while LiNix launches them
//! through an interpreter — they are `.cmd`/`.ps1` shims and `Command::new` cannot execute one.
//! Five installed managers, silently uncovered, in the gate written to stop exactly that.
//! Everything here now goes through [`linix::core::executor::effective_command`], the same
//! function the product uses.

use std::collections::BTreeSet;
use std::process::Command;

/// Programs whose `--help` cannot answer this question, with the reason.
///
/// Listed rather than silently skipped: an exemption is a claim, and an unexamined exemption
/// list is where coverage goes to disappear (E29).
fn help_cannot_answer(program: &str) -> Option<&'static str> {
    match program {
        // Not package managers — LiNix runs these as plain programs with no subcommand.
        "sh" | "bash" | "sudo" | "env" | "tee" | "cp" | "mv" | "rm" | "ln" | "chmod" => {
            Some("not a package manager; LiNix runs it as a plain program")
        }
        // PowerShell takes a script, not a subcommand; its "argv" is a -Command string.
        "powershell" | "pwsh" => Some("takes a -Command script, not a subcommand"),
        // Emacs is handed an Emacs Lisp form after `--eval`. Every word in it looks like a
        // subcommand to a flat argv reader and none of them is one.
        "emacs" => Some("takes an Emacs Lisp form after --eval, not a subcommand"),
        // Windows service control dispatches on argv[1] by hand with no getopt — `sc query
        // --help` tries to query a service literally named `--help`. `core::argv` already
        // records the same fact for the option terminator.
        "sc" => Some("dispatches on argv[1] by hand; it has no help to ask"),
        // A plugin host. `kubectl krew` works through a `kubectl-krew` binary and no
        // `kubectl --help` lists it, so absence from that help means nothing either way.
        // Whether krew is usable is what the krew backend's own `probes()` answers.
        "kubectl" => Some("a plugin host; its help cannot list plugin subcommands"),
        _ => None,
    }
}

/// Does the tool itself reject this subcommand as unknown or removed?
///
/// Absence from `--help` is a signal, not a verdict, and treating it as one produced a false
/// positive on the first real run: `bun pm ls` is missing from `bun pm --help` — which now
/// documents `bun list` instead — but still runs. Undocumented is not removed, and a gate that
/// cannot tell the difference is a gate people learn to ignore.
///
/// So the finding is confirmed by asking the tool. `--help` is the probe because it is the one
/// argument no package manager acts on: a live subcommand prints its help, a dead one says so.
fn tool_rejects(program: &str, chain: &[String], tok: &str) -> Option<String> {
    // Through `run`, like `help_text`: a shimmed manager that cannot be launched answers
    // nothing, and nothing contains none of the phrases below — so a raw `Command` here would
    // silently clear every subcommand of every shimmed manager as "not rejected".
    let args: Vec<String> = chain
        .iter()
        .cloned()
        .chain([tok.to_string(), "--help".to_string()])
        .collect();
    let text = run(program, &args)?.to_lowercase();
    for phrase in [
        "has been removed",
        "unknown command",
        "unrecognized subcommand",
        "unrecognised subcommand",
        "invalid command",
        "is not a recognized",
        "no such command",
        "unknown subcommand",
        "did you mean",
    ] {
        if text.contains(phrase) {
            return Some(
                text.lines()
                    .find(|l| l.contains(phrase))
                    .unwrap_or(phrase)
                    .trim()
                    .to_string(),
            );
        }
    }
    None
}

/// A token that is a subcommand rather than a flag, an operand or a fragment of a script.
///
/// Deliberately strict, because **a gate with false positives is worse than no gate**: it
/// trains people to ignore it, which is this codebase's disease in a new costume. The first
/// draft flagged `go env GOPATH` (an operand), `kubectl krew` (a plugin, which no
/// `kubectl --help` lists) and `emacs (progn (require 'package)` (Emacs Lisp) as upstream
/// drift. Not one of them was.
///
/// A subcommand is lowercase, starts with a letter, and carries no punctuation beyond `-`,
/// `_` and `.` — which is what every manager in this tree actually names its verbs. `GOPATH`
/// fails on case; `(progn` and `'package)` fail on punctuation.
fn is_subcommand_token(tok: &str) -> bool {
    let mut chars = tok.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && tok
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
        && tok != "jq"
}

fn on_path(program: &str) -> bool {
    which::which(program).is_ok()
}

/// Run a manager the way LiNix runs it.
///
/// `Command::new("scoop")` cannot launch anything on Windows: `scoop` is a `.ps1`/`.cmd` shim,
/// and only an interpreter can execute it. `which::which` still finds it, so this gate said
/// "installed" and then failed to read a word of its help — and skipped it. LiNix's own
/// executor has always wrapped shims; the gate did not, so it launched a different program
/// from the one that ships and covered four installed managers less than it claimed.
fn run(program: &str, args: &[String]) -> Option<String> {
    let (prog, argv) = linix::core::executor::effective_command(program, args);
    let out = Command::new(prog).args(&argv).output().ok()?;
    Some(format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

/// The manager's own help for a subcommand chain, or None if it could not be asked.
fn help_text(program: &str, chain: &[String]) -> Option<String> {
    for flag in ["--help", "help", "-h"] {
        // `go help`, `mix help` — the verb form, which some tools use exclusively.
        let args: Vec<String> = if flag == "help" {
            std::iter::once("help".to_string())
                .chain(chain.iter().cloned())
                .collect()
        } else {
            chain
                .iter()
                .cloned()
                .chain(std::iter::once(flag.to_string()))
                .collect()
        };
        if let Some(text) = run(program, &args) {
            // A help page is long. A one-line "unknown flag" is not, and must not be mistaken
            // for one that simply does not mention our subcommand.
            if text.len() > 40 {
                return Some(text.to_lowercase());
            }
        }
    }
    None
}

#[tokio::test]
async fn every_subcommand_linix_invokes_still_exists_upstream() {
    use dashmap::DashMap;
    use linix::core::executor::MockExecutor;
    use linix::core::{CommandExecutor, PackageSpec};
    use std::sync::Arc;

    let vfs = Arc::new(DashMap::new());
    let mock = Arc::new(MockExecutor::new(vfs.clone()));
    let exec =
        CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
    let config = linix::config::Config::default();
    let registry = linix::backends::create_default_registry(
        exec,
        &config,
        Arc::new(linix::app::hooks::LuaHooks::new(&config).expect("hooks")),
    )
    .await;

    // Drive every capability that builds an argv. Failures are irrelevant — the mock runs
    // nothing; what matters is the argv each call *would* have used.
    // `available()`, not `all()`. A backend whose program is not on this machine cannot have
    // its subcommands checked against anything, and asking anyway is how `kubectl krew` got
    // reported as drift when krew simply is not installed here.
    for backend in registry.available() {
        let spec = PackageSpec {
            name: "jq".into(),
            backend: backend.name().into(),
            ..Default::default()
        };
        if let Some(i) = backend.as_installable() {
            let _ = i.install(std::slice::from_ref(&spec), false).await;
            let _ = i.remove(&["jq".to_string()], false).await;
        }
        if let Some(q) = backend.as_queryable() {
            let _ = q.list_installed().await;
        }
        if let Some(u) = backend.as_upgradable() {
            let _ = u.upgrade(false).await;
            let _ = u.update(false).await;
        }
        if let Some(s) = backend.as_searchable() {
            let _ = s.search("jq").await;
        }
    }

    let calls = mock.get_calls().await;
    // `DRIFT_DUMP=1 cargo test --test argv_drift_tests -- --nocapture` prints every argv this
    // gate examined. Kept because the first version of this test passed while never reaching
    // pixi at all, and the only way to see that was to look at what it had actually collected.
    if std::env::var("DRIFT_DUMP").is_ok() {
        for c in &calls {
            eprintln!("CALL: {c}");
        }
    }
    assert!(
        !calls.is_empty(),
        "no backend produced an argv — this gate would pass by testing nothing"
    );

    let mut drifted: Vec<String> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    let mut checked: BTreeSet<String> = BTreeSet::new();
    let mut skipped: BTreeSet<String> = BTreeSet::new();

    for call in &calls {
        let mut toks = call.split_whitespace();
        let Some(program) = toks.next() else { continue };
        if let Some(why) = help_cannot_answer(program) {
            skipped.insert(format!("{program} ({why})"));
            continue;
        }
        if !on_path(program) {
            skipped.insert(format!("{program} (not installed on this machine)"));
            continue;
        }

        // Walk the chain: `pixi` must list `global`, then `pixi global` must list `update`.
        let mut chain: Vec<String> = Vec::new();
        for tok in toks {
            if tok == "--" {
                break;
            }
            if !is_subcommand_token(tok) {
                continue;
            }
            let Some(help) = help_text(program, &chain) else {
                // NOT a skip. The program is on this machine — `on_path` said so above — so
                // an unreadable help is this gate failing to ask, not the manager
                // declining to answer, and every subcommand behind it goes unchecked while
                // the run still reports a pass. On Windows that was `scoop`, `npm`, `gem` and
                // `pipx`: four installed managers, silently uncovered.
                unreadable.push(format!(
                    "{} {} — installed, but this gate could not read its help",
                    program,
                    chain.join(" ")
                ));
                break;
            };
            let shown = format!("{} {}", program, {
                let mut c = chain.clone();
                c.push(tok.to_string());
                c.join(" ")
            });
            if help.contains(&tok.to_lowercase()) {
                checked.insert(shown);
            } else if let Some(said) = tool_rejects(program, &chain, tok) {
                drifted.push(format!("`{shown}` — the tool says: {said}"));
            } else {
                // Missing from the help and the tool still takes it: undocumented, not gone.
                // Worth knowing, never worth failing on.
                skipped.insert(format!(
                    "{shown} (undocumented in its own help, but the tool still accepts it)"
                ));
                checked.insert(shown);
            }
            chain.push(tok.to_string());
        }
    }

    // Reported always, pass or fail: a gate that covers eight of fifty managers and prints
    // nothing about the other forty-two is describing a machine, not a product.
    eprintln!(
        "argv drift: {} subcommand(s) verified against a real tool on this machine",
        checked.len()
    );
    for s in &skipped {
        eprintln!("  skipped: {s}");
    }

    assert!(
        unreadable.is_empty(),
        "these managers are installed here and this gate could not ask them anything, so every          subcommand LiNix runs on them is unverified:
  {}",
        unreadable.join("
  ")
    );
    assert!(
        drifted.is_empty(),
        "these subcommands no longer exist in the tool LiNix runs them on:\n  {}",
        drifted.join("\n  ")
    );
    assert!(
        checked.len() >= 5,
        "only {} subcommand(s) could be checked on this machine — too few for this to mean \
         anything. Run it where the managers are installed.",
        checked.len()
    );
}
