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
