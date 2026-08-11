//! `Y7a`, ruled 2026-08-03: a running service is adopted as a live line. Everything below was
//! found by running the command on a real Windows box, and none of it by a unit test.
//!
//!     $ shall adopt
//!     Adopted 460 declaration(s).
//!     $ shall check config
//!     Error: …/modules/adopted.txt:147: `$SQLEXPRESS` is not defined
//!
//! `MSSQL$SQLEXPRESS` is a real service, and `$` is a variable reference (IX.3) resolved *after*
//! the parse — so the round trip through the grammar agreed the line was writable and the file
//! still wedged every later command. Escaped `$$` by the one function that spells a line, which
//! is also the function packages go through: a package name can carry a `$` too.
//!
//! And with that fixed:
//!
//!     $ shall plan
//!     - it removes 154 packages, over the limit of 20
//!     - service:AppMgmt would be removed (…)
//!
//! `adopt` recorded every adopted name in the *package* registry, but a `service:` line resolves
//! to a resource statement and is never in the model's package set — so all 154 were managed with
//! nothing declaring them, and the first `plan` scheduled the machine's services for removal and
//! then refused itself. Resources enter the ledger when `sync` places them.
//!
//! **A host with no init system has no services to adopt**, so it is skipped and named rather
//! than passed green.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(args)
        .current_dir(dir)
        .env("SHALL_CONFIG_DIR", dir.join("config"))
        .env("SHALL_DATA_DIR", dir.join("data"))
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

/// A fixture whose `priority` also lists `service`. `init` writes the managers it finds, and it
/// does not list `service` — so a fixture without this line adopts no services and would pass
/// every assertion below without measuring one.
fn fixture(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let (out, code) = run(&root, &["init"]);
    assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
    let priority = root.join("config").join("priority");
    let mut p = std::fs::read_to_string(&priority).unwrap();
    p.push_str("\nservice\n");
    std::fs::write(&priority, p).unwrap();
    root
}

fn declared_in(module: &Path) -> Vec<String> {
    std::fs::read_to_string(module)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn an_adopted_service_is_a_live_line_carrying_the_state_it_was_found_in() {
    let root = fixture("grade5-adopt-services");
    let (out, code) = run(&root, &["adopt", "-y"]);
    assert_eq!(code, 0, "{out}");

    let lines = declared_in(&root.join("config").join("modules").join("adopted.txt"));
    let services: Vec<&String> = lines.iter().filter(|l| l.starts_with("service:")).collect();
    if services.is_empty() {
        eprintln!(
            "skipped: this host reported no running services, so an adopted service line \
             cannot be measured here"
        );
        return;
    }

    // Live, not commented — `declared_in` already dropped every `#` line, so reaching here at
    // all is the ruling. What each line says is the other half.
    for line in &services {
        assert!(
            line.ends_with("@status=running"),
            "a service is declared as the state it was found in, and the init only reports \
             running ones: `{line}`"
        );
        assert!(
            !line.contains("enabled="),
            "the start type was never looked at, so no adopted line may claim it: `{line}`"
        );
    }

    // A `$` in a name is a variable reference, and one that survives into the file wedges every
    // later command. This is the assertion `check config` below would fail on, stated here too
    // so the reason is named rather than inferred from an exit code.
    for line in &lines {
        let name = line.split_once(':').map_or(line.as_str(), |(_, n)| n);
        for (i, _) in name.match_indices('$') {
            assert!(
                name[i + 1..].starts_with('$') || name[..i].ends_with('$'),
                "`{line}` carries an unescaped `$`, which resolves as a variable reference"
            );
        }
    }

    // The manifest Shall wrote is one Shall can read. Parsing is not enough — `$` passes the
    // parser and fails the resolver, which is where the real defect lived.
    let (cfg, code) = run(&root, &["check", "config"]);
    assert_eq!(
        code, 0,
        "`adopt` wrote a config that does not resolve:\n{cfg}"
    );

    // And nothing it took is scheduled for removal. An adopted service recorded as a *package*
    // is managed with nothing declaring it, which reads as a removal of every service on the box.
    //
    // Either summary satisfies this: an empty plan does not print a count, and it is the
    // stronger outcome — the whole point of adopting is that the machine already matches. It
    // did not print that until `in_effect` learned to ask the init whether a declared service
    // is running, because before it every adopted service was `unverifiable`, and unverifiable
    // places.
    let (plan, code) = run(&root, &["plan"]);
    assert_eq!(code, 0, "{plan}");
    assert!(
        plan.contains("0 removal(s)") || plan.contains("already matches desired state"),
        "adopting a machine must never plan a removal on it:\n{plan}"
    );
    assert!(
        !plan.contains("would be removed"),
        "adopting a machine must never plan a removal on it:\n{plan}"
    );
}

/// The half of `Y7a` that made every later `install` fail, and the reason it took a real
/// machine to see: `adopt` writes `service:X@status=running` for every running service, and
/// until `in_effect` could answer for a `service:` those lines were placed on the *first* sync
/// whatever the machine looked like — 150 `sc start` calls on 150 already-running services,
/// each returning 1056, the first one failing the transaction.
///
/// Asserted on a plan rather than a sync so it costs no mutation: a machine that already matches
/// its adopted manifest has nothing to place.
#[test]
fn adopting_a_running_service_leaves_nothing_to_place() {
    let root = fixture("grade5-adopt-nothing-to-place");
    let (out, code) = run(&root, &["adopt", "-y"]);
    assert_eq!(code, 0, "{out}");

    let lines = declared_in(&root.join("config").join("modules").join("adopted.txt"));
    let services = lines.iter().filter(|l| l.starts_with("service:")).count();
    if services == 0 {
        eprintln!("skipped: this host reported no running services");
        return;
    }

    let (plan, code) = run(&root, &["plan"]);
    assert_eq!(code, 0, "{plan}");
    // A service that stopped between the two commands is real drift and is allowed to place —
    // trigger-start services do exactly that. What must not happen is the whole adopted set
    // queueing up, which is what "150 resource(s) to place" out of 150 adopted lines was.
    let placing = plan
        .split("resource(s) to place")
        .next()
        .and_then(|s| s.rsplit(' ').next())
        .and_then(|n| n.trim().parse::<usize>().ok())
        .unwrap_or(0);
    assert!(
        placing * 2 < services,
        "{placing} of {services} adopted services are queued to be applied — a machine that was \
         just adopted from is by definition already in the state it was adopted in:\n{plan}"
    );
}
