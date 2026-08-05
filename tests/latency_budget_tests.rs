//! How long a command is allowed to take.
//!
//! Nothing measured latency, which is how a 98-second `linix info cargo:ripgrep` shipped —
//! answering "not found" the whole time, while `linix search ripgrep` in the same tree found
//! it. The cause was that the `cargo:` qualifier was ignored and every backend on the machine
//! was asked for a package literally named `cargo:ripgrep`.
//!
//! **These are ceilings, not targets.** They are set well above what the commands actually
//! take here (a qualified `info` measures ~0.2s against a 5s budget) so that a normal machine
//! under normal load never goes red, and a regression of the *shape* that produced E14 — a
//! probe fanning out across every backend when it was told which one to ask — cannot pass.
//! A budget tight enough to fail on a busy CI runner is a budget people delete.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn fresh(name: &str) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_timed(dir: &Path, args: &[&str]) -> (Duration, i32) {
    let start = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_linix"))
        .args(args)
        .env("LINIX_CONFIG_DIR", dir.join("config"))
        .env("LINIX_DATA_DIR", dir.join("data"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    (start.elapsed(), out.status.code().unwrap_or(-1))
}

/// A ceiling, and only a ceiling.
///
/// Stated plainly because the first version of this comment claimed more than it could
/// deliver: it said asking every backend "cannot be done in five seconds", and then the test
/// passed at 4.34s with the defect deliberately reintroduced. On this machine the pre-fix path
/// measured 3.2-4.3s and the fixed one 0.22-0.36s, so **no wall-clock budget separates them
/// safely** — one tight enough to catch it would fail on a loaded runner.
///
/// So this catches the 98-second shape that was actually reported, and the *property* — that a
/// qualified `info` consults exactly one manager — is pinned by
/// `a_qualified_info_consults_only_the_named_backend` below, which counts the commands rather
/// than timing them.
#[test]
fn a_qualified_info_stays_under_its_ceiling() {
    let dir = fresh("latency-info");
    let (_, code) = run_timed(&dir, &["init"]);
    assert_eq!(code, 0, "init failed");

    let (elapsed, _) = run_timed(&dir, &["info", "cargo:ripgrep"]);
    assert!(
        elapsed < Duration::from_secs(5),
        "`linix info cargo:ripgrep` took {elapsed:?}. It names the manager to ask; taking this \
         long means it asked all of them, which is exactly how E14's 98 seconds happened."
    );
}

/// Commands that only read a file must not touch the machine at all.
#[test]
fn commands_that_only_read_the_config_are_immediate() {
    let dir = fresh("latency-read");
    let (_, code) = run_timed(&dir, &["init"]);
    assert_eq!(code, 0, "init failed");

    for args in [
        vec!["--version"],
        vec!["--help"],
        vec!["profile", "list"],
        vec!["module", "list"],
        vec!["path"],
    ] {
        let (elapsed, _) = run_timed(&dir, &args);
        assert!(
            elapsed < Duration::from_secs(5),
            "`linix {}` took {elapsed:?} and reads nothing but the config directory",
            args.join(" ")
        );
    }
}

/// The property the budget above cannot pin: a qualified `info` asks the manager it was told
/// to ask, and no other.
///
/// Counted, not timed. `linix info cargo:ripgrep` used to hand the whole string — colon and
/// all — to every backend on the machine, which is why it was slow *and* why it always
/// answered "not found": no manager has a package whose name contains a colon.
#[tokio::test]
async fn a_qualified_info_consults_only_the_named_backend() {
    use dashmap::DashMap;
    use linix::core::executor::MockExecutor;
    use linix::core::CommandExecutor;
    use std::sync::Arc;

    let dir = fresh("info-one-backend");
    let config_dir = dir.join("config");
    std::fs::create_dir_all(config_dir.join("modules")).unwrap();
    std::fs::create_dir_all(config_dir.join("profiles")).unwrap();
    std::fs::write(
        config_dir.join("priority"),
        "cargo
npm
pipx
go
",
    )
    .unwrap();
    std::fs::write(config_dir.join("active"), "").unwrap();

    // `sandboxed` so the registry, journal and snapshots land in the temp dir too — the
    // fixture helper that exists because forgetting `data_root` wrote into real user state.
    let config = linix::config::Config::sandboxed(&config_dir);

    let vfs = Arc::new(DashMap::new());
    let mock = Arc::new(MockExecutor::new(vfs.clone()));
    let exec =
        CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
    let app = linix::app::App::new_with_executor_and_state_path(
        config,
        exec,
        Some(dir.join("data").join("state.json")),
    )
    .await
    .expect("app");

    let _ = app.get_info("cargo:ripgrep").await;

    let calls = mock.get_calls().await;
    let strangers: Vec<&String> = calls
        .iter()
        .filter(|c| {
            let program = c.split_whitespace().next().unwrap_or("");
            !program.is_empty() && program != "cargo"
        })
        .collect();
    assert!(
        strangers.is_empty(),
        "`info cargo:ripgrep` names cargo and asked other managers too: {strangers:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("cargo:ripgrep")),
        "the qualifier was passed through as part of the package name: {calls:?}"
    );
}

// ---------------------------------------------------------------------------------------
// BUILDER round 6, W41 / R-9 — and a correction to the finding that asked for it.
//
// R-9 says "nothing measures latency". **This file already did**, committed in `02a4ec9` with
// W14's fix: two ceilings and one call-counting property test. What did not exist is what
// §8.1's A+ line actually names — a budget PER COMMAND CLASS, covering more than `info`, and a
// number reaching the user when one is crossed. That is what is added here, in this file
// rather than in a second one.
//
// Measured for this order on Windows, 24 ready backends, debug build, 111 adopted packages:
//
//     policy / vars / eval / check config        0.13 – 0.32 s
//     list                                       3.4  – 3.9  s
//     check health                               4.3  – 5.4  s
//     check                                      8.5  – 18.3 s
//
// The split is the finding, and it decides which classes carry a budget at all: a command that
// reads only files is fast on every machine, and one that asks every manager costs whatever
// the managers cost — a fact about the host. So `EveryBackend` and `Mutating` carry none, and
// saying that out loud is better than a number nobody can defend.
// ---------------------------------------------------------------------------------------

/// Every config-only command against its class budget — the class the two ceilings above did
/// not cover, and the one whose cost is LiNix's alone.
#[test]
fn every_config_only_command_stays_inside_its_class_budget() {
    use linix::core::latency::Class;
    let budget = Class::ConfigOnly.budget().expect("config-only carries one");
    let dir = fresh("latency-class-config");
    let (_, code) = run_timed(&dir, &["init"]);
    assert_eq!(code, 0, "init failed");

    let mut over = Vec::new();
    for args in [
        vec!["policy"],
        vec!["vars"],
        vec!["eval"],
        vec!["check", "config"],
        vec!["protected", "jq"],
    ] {
        let (elapsed, code) = run_timed(&dir, &args);
        // The control: a command that failed did not do the work, so its clock says nothing.
        assert!(
            code == 0 || code == 2,
            "`linix {}` exited {code}, so its timing measures nothing",
            args.join(" ")
        );
        if elapsed > budget {
            over.push(format!("`linix {}` took {elapsed:?}", args.join(" ")));
        }
    }

    assert!(
        over.is_empty(),
        "these read no manager and still crossed {}s:\n  {}\n\nMeasured at 0.13-0.32s when the \
         budget was set, so crossing it is an order of magnitude and not load.",
        budget.as_secs(),
        over.join("\n  ")
    );
}

/// The class table names subcommands, and a name is the thing that goes stale. `undo` sat in
/// two harness exemption lists after being renamed away because nothing validated the list.
#[test]
fn every_subcommand_the_class_table_names_still_exists() {
    let help = Command::new(env!("CARGO_BIN_EXE_linix"))
        .arg("--help")
        .output()
        .expect("the binary should run");
    let help = String::from_utf8_lossy(&help.stdout).into_owned();

    const NAMED: &[&str] = &[
        "policy",
        "vars",
        "eval",
        "why",
        "protected",
        "completions",
        "path",
        "history",
        "diff",
        "sbom",
        "export",
        "plan",
        "profile",
        "module",
        "edit",
        "config",
        "hooks",
        "schedule",
        "fleet",
        "info",
        "list",
        "search",
        "check",
        "adopt",
    ];

    let missing: Vec<&str> = NAMED
        .iter()
        .copied()
        .filter(|n| !help.contains(&format!("  {n} ")) && !help.contains(&format!("  {n}\n")))
        .collect();

    assert!(
        missing.is_empty(),
        "the latency class table names subcommands `--help` does not list: {missing:?}"
    );
}

/// The classification and the name it is looked up by, which is the half a wall-clock test
/// cannot pin without measuring the runner's mood instead of the program.
#[test]
fn the_class_of_a_command_is_read_off_its_own_variant() {
    use linix::core::latency::{subcommand_name, Class};

    // clap kebab-cases the variant; this reverses exactly that, so there is no second list of
    // sixty-six names to drift.
    #[derive(Debug)]
    #[allow(dead_code)]
    enum Fake {
        SelfUpgrade,
        Check { config: bool },
        PurgeUndeclared,
    }
    assert_eq!(subcommand_name(&Fake::SelfUpgrade), "self-upgrade");
    assert_eq!(subcommand_name(&Fake::Check { config: true }), "check");
    assert_eq!(subcommand_name(&Fake::PurgeUndeclared), "purge-undeclared");

    assert_eq!(Class::of("eval"), Class::ConfigOnly);
    assert_eq!(Class::of("info"), Class::OneBackend);
    assert_eq!(Class::of("list"), Class::EveryBackend);
    assert!(
        Class::of("list").budget().is_none(),
        "a host with forty managers is slow because it has forty managers"
    );
    assert_eq!(
        Class::of("linix-command-that-does-not-exist"),
        Class::Mutating,
        "a name the table does not know must fall to the class with NO budget, never to one \
         with a tight one — an unknown command is one nobody measured"
    );
}
