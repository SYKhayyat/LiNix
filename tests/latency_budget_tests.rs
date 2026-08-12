//! How long a command is allowed to take.
//!
//! Nothing measured latency, which is how a 98-second `shall info cargo:ripgrep` shipped —
//! answering "not found" the whole time, while `shall search ripgrep` in the same tree found
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
    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(args)
        .env("SHALL_CONFIG_DIR", dir.join("config"))
        .env("SHALL_DATA_DIR", dir.join("data"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    (start.elapsed(), out.status.code().unwrap_or(-1))
}

/// The fastest of three runs, and the exit code of the last.
///
/// **A wall clock in a parallel test suite measures the suite, not the command.** Every one of
/// these budgets is a claim about what the command *inherently* costs, and it is checked while
/// five hundred other tests — many of them spawning processes of their own — compete for the
/// same cores. `every_config_only_command_stays_inside_its_class_budget` crossed a five-second
/// budget on one such run and measured **111-131 ms** on the same binary a minute later, which
/// is the box being busy, not `shall vars` being slow.
///
/// The floor is the honest statistic for the question being asked. A command that got slower by
/// the order of magnitude these budgets exist to catch is slower in *every* run, so taking the
/// best one loses nothing the test was ever able to see — while a single sample loses the whole
/// gate the first time a runner is loaded, and a budget that goes red for reasons nobody
/// controls is a budget people delete. The comment at the top of this file says exactly that
/// about the *number*; this says it about the *method*.
fn run_timed_floor(dir: &Path, args: &[&str]) -> (Duration, i32) {
    let mut best = Duration::MAX;
    let mut code = -1;
    for _ in 0..3 {
        let (elapsed, rc) = run_timed(dir, args);
        best = best.min(elapsed);
        code = rc;
        // A run already inside the budget answers the question; the other two would only
        // confirm it, and every sample here is a whole process launch.
        if best < Duration::from_secs(1) {
            break;
        }
    }
    (best, code)
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

    let (elapsed, _) = run_timed_floor(&dir, &["info", "cargo:ripgrep"]);
    assert!(
        elapsed < Duration::from_secs(5),
        "`shall info cargo:ripgrep` took {elapsed:?}. It names the manager to ask; taking this \
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
        let (elapsed, _) = run_timed_floor(&dir, &args);
        assert!(
            elapsed < Duration::from_secs(5),
            "`shall {}` took {elapsed:?} and reads nothing but the config directory",
            args.join(" ")
        );
    }
}

/// The property the budget above cannot pin: a qualified `info` asks the manager it was told
/// to ask, and no other.
///
/// Counted, not timed. `shall info cargo:ripgrep` used to hand the whole string — colon and
/// all — to every backend on the machine, which is why it was slow *and* why it always
/// answered "not found": no manager has a package whose name contains a colon.
#[tokio::test]
async fn a_qualified_info_consults_only_the_named_backend() {
    use dashmap::DashMap;
    use shall::core::executor::MockExecutor;
    use shall::core::CommandExecutor;
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
    let config = shall::config::Config::sandboxed(&config_dir);

    let vfs = Arc::new(DashMap::new());
    let mock = Arc::new(MockExecutor::new(vfs.clone()));
    let exec =
        CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
    let app = shall::app::App::new_with_executor_and_state_path(
        config,
        exec,
        Some(dir.join("data").join("state.json")),
    )
    .await
    .expect("app");

    let _ = app.inventory().await.get_info("cargo:ripgrep").await;

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
/// not cover, and the one whose cost is Shall's alone.
#[test]
fn every_config_only_command_stays_inside_its_class_budget() {
    use shall::core::latency::Class;
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
        let (elapsed, code) = run_timed_floor(&dir, &args);
        // The control: a command that failed did not do the work, so its clock says nothing.
        assert!(
            code == 0 || code == 2,
            "`shall {}` exited {code}, so its timing measures nothing",
            args.join(" ")
        );
        if elapsed > budget {
            over.push(format!("`shall {}` took {elapsed:?}", args.join(" ")));
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
    let help = Command::new(env!("CARGO_BIN_EXE_shall"))
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
    use shall::core::latency::{subcommand_name, Class};

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
        Class::of("shall-command-that-does-not-exist"),
        Class::Mutating,
        "a name the table does not know must fall to the class with NO budget, never to one \
         with a tight one — an unknown command is one nobody measured"
    );
}

/// The fan-out commands keep their shape.
///
/// **The budget the wall clock cannot express, driven for real.** `Class::EveryBackend` carries
/// no ceiling in seconds, and correctly so: `list` costs whatever the managers on the host cost.
/// But that left the commands users run most with nothing measuring them at all, and the
/// regression it could not see is the one that matters — a change that serialises the fan-out
/// drops overlap from 6.3× to 1.2×, the wall clock stays inside a budget of `None`, and it stays
/// there for ever.
///
/// `--timings` has computed the overlap ratio and the wave count since it was written and
/// nothing read either. This reads them.
///
/// **Skipped, loudly, on a host with too few managers.** The ratio is not a measurement of
/// anything below four child commands, so a bare runner says so rather than passing silently —
/// a gate that skips without saying so reads as a pass.
#[test]
fn the_fan_out_commands_still_fan_out() {
    let dir = fresh("latency-shape");
    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(["init"])
        .env("SHALL_CONFIG_DIR", dir.join("config"))
        .env("SHALL_DATA_DIR", dir.join("data"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    assert!(out.status.success(), "init failed");

    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(["--timings", "list"])
        .env("SHALL_CONFIG_DIR", dir.join("config"))
        .env("SHALL_DATA_DIR", dir.join("data"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    // The breakdown goes to stderr on purpose — `shall eval --timings | jq` must still get
    // JSON — so that is where it is read from.
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    let line = report
        .lines()
        .find(|l| l.starts_with("Timings:"))
        .unwrap_or_else(|| {
            panic!("`shall --timings list` printed no `Timings:` line; the instrument this gate reads is gone:\n{report}")
        });

    // `Timings: 3.75s wall · 23 child command(s) summing to 23.67s · 6.3x overlap · 2 wave(s)`
    let number_before = |unit: &str| -> Option<f64> {
        let at = line.find(unit)?;
        line[..at]
            .rsplit(|c: char| !(c.is_ascii_digit() || c == '.'))
            .find(|t| !t.is_empty())?
            .parse()
            .ok()
    };
    let Some(children) = number_before(" child command(s)") else {
        // The no-children form is a different sentence entirely, and it is a real answer on a
        // machine with no managers.
        eprintln!("latency shape: SKIPPED — no child commands on this host:\n  {line}");
        return;
    };
    let children = children as usize;
    if children < 4 {
        eprintln!(
            "latency shape: SKIPPED — {children} child command(s) on this host, which is too \
             few for an overlap ratio to mean anything:\n  {line}"
        );
        return;
    }

    let overlap = number_before("x overlap").expect("the summary line prints an overlap ratio");
    let waves = number_before(" wave(s)").expect("the summary line prints a wave count") as usize;
    // The numerator, read so the gate below can be two-sided and so a human diffing two CI runs
    // can see it move. `… 23 child command(s) summing to 23.67s · …`
    let summed: Option<f64> = line.split_once("summing to ").and_then(|(_, rest)| {
        rest.split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .find(|t| !t.is_empty())?
            .parse()
            .ok()
    });

    // **The bounds come from `Shape`, not from a second copy of them here.** The first draft
    // wrote `>= 2.0` and `<= 2` in both places, taken from one host, and ubuntu-latest reported
    // 2.0x over 16 children with 3 waves — inside neither. Two copies of a guessed constant is
    // two places to be wrong; asking the type is one.
    let shape = shall::core::latency::Class::of("list")
        .shape()
        .expect("`list` asks every manager, so it carries a shape budget");
    let ceiling = shape.wave_ceiling(children);

    assert!(
        overlap >= shape.min_overlap,
        "`shall list` asked {children} managers and overlapped them only {overlap:.1}x, under \
         the {:.1}x floor — close to asking them one at a time. The seconds a fan-out costs \
         belong to the host; the scheduling does not.\n  {line}",
        shape.min_overlap
    );
    // **The other side of the same number, because a floor alone can be satisfied by making
    // things worse.** The ratio is `sum(child time) / wall` and contention inflates the
    // numerator: measured over the same 23 children, width 20 spent 676.6s of child time
    // against width 4's 182.5s — 3.7× the total work — and the ratio *rose* from 1.6× to 8.3×
    // for it. Wall clock genuinely improved, so the design is sound and this is not a budget on
    // the design; it is a check that the instrument is measuring concurrency at all.
    //
    // The ceiling is arithmetic rather than tuned: `sum/wall` is the mean number of children in
    // flight, and a run cannot average more children in flight than it has. A reading above
    // that is not a fast run, it is a numerator counting something that is not overlap —
    // queued time, a retry, a child recorded twice. A tighter ceiling would have to know each
    // command's concurrency cap, and a guessed one is a flaky gate, which is worse than none.
    //
    // The summed child time is in the message either way, so a human diffing two CI runs can
    // see the numerator move even where no assertion fires — the regression this pair still
    // cannot catch is every child getting slower at constant concurrency, which moves `sum` and
    // `wall` together and leaves the ratio exactly where it was.
    assert!(
        overlap <= children as f64,
        "`shall list` reports {overlap:.1}x overlap over {children} child command(s)\
         {}. A run cannot average more children in flight than it has, so the numerator is \
         counting something that is not concurrency.\n  {line}",
        summed
            .map(|s| format!(" summing to {s:.2}s"))
            .unwrap_or_default()
    );

    assert!(
        waves <= ceiling,
        "`shall list` went quiet {} time(s) mid-run ({waves} waves over {children} children, \
         ceiling {ceiling}). A serial run has one wave per child, and this is close enough to it \
         to be a fan-out that stopped fanning out.\n  {line}",
        waves.saturating_sub(1)
    );
}
