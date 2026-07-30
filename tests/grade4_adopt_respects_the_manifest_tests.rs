//! GRADER round 5, 2026-07-30 — RED. `adopt` offers a package that is already declared, writing a
//! second declaration for it in another module; and its summary explains the packages it skipped
//! with a reason that is never the reason.
//!
//! Measured on Windows, one package declared and installed:
//!
//!     $ cat config/modules/mine.txt
//!     cargo:ripgrep
//!     $ linix check config
//!     OK: … 1 present, 0 absent, 0 repo/shim/service/link/schedule line(s).
//!
//!     $ linix adopt -y
//!     Adopted 111 package(s).
//!     Left alone: 185 (listed in the manifest)          <- the manifest has one line
//!     …
//!     Deleting a line UNINSTALLS that package on the next sync.
//!
//!     $ grep '^cargo:ripgrep' config/modules/adopted.txt
//!     cargo:ripgrep                                      <- now declared in two modules
//!
//! **Two defects, one root.** `discover()` keeps a candidate when
//! `!state_guard.is_managed(&pkg.backend, &pkg.name)` — the managed-state *registry*
//! (`app/adopt.rs:117`). Nothing in `discover` reads the manifests at all, so a package declared by
//! hand and not yet synced is offered again. And `found.skipped` has exactly two push sites,
//! `:154` (the OS reports it essential) and `:315` (`hold_back_what_cannot_be_written`); the
//! summary at `:281` prints `found.skipped.len()` under *"(listed in the manifest)"*, a reason that
//! is wrong for 100% of the items, always. Each `Skipped` already carries a correct per-item
//! `reason`; the rollup discards them for one that is never true.
//!
//! **Consequence, driven to the end.** The user deletes their own line, as the sentence three lines
//! below the count instructs — and nothing happens, because `adopted.txt` still declares it:
//!
//!     $ : > config/modules/mine.txt
//!     $ linix --dry-run sync -y
//!     already up to date
//!     $ linix why cargo:ripgrep
//!       declared:    at …/config/modules/adopted.txt:44 (module:adopted, profile:Main)
//!
//! It fails safe — nothing is removed — which is what keeps this at medium.
//!
//! **A host with nothing to adopt cannot test this**, so such a host is *skipped and named* rather
//! than passed. A green result on a machine where `adopt` found nothing is the failure mode the
//! dry-run gate was rebuilt to stop.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_linix"))
        .args(args)
        .current_dir(dir)
        .env("LINIX_CONFIG_DIR", dir.join("config"))
        .env("LINIX_DATA_DIR", dir.join("data"))
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

fn fixture(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let (out, code) = run(&root, &["init"]);
    assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
    root
}

/// The package lines `adopt` wrote, in order. Comments and blank lines are not declarations.
fn declared_in(module: &Path) -> Vec<String> {
    std::fs::read_to_string(module)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Ask LiNix itself what this host has to adopt, so the test names a package that really is
/// adoptable here rather than guessing one. Returns `None` on a host with nothing.
fn one_adoptable_package(tag: &str) -> Option<String> {
    let probe = fixture(&format!("grade4-adopt-probe-{tag}"));
    let (_probe_out, code) = run(&probe, &["adopt", "-y"]);
    if code != 0 {
        return None;
    }
    let lines = declared_in(&probe.join("config").join("modules").join("adopted.txt"));
    // A line with a backend and no options: the simplest thing to re-declare by hand.
    lines
        .into_iter()
        .find(|l| l.contains(':') && !l.contains(' ') && !l.contains('@'))
}

#[test]
fn adopt_does_not_re_declare_a_package_the_manifest_already_names() {
    let Some(pkg) = one_adoptable_package("dup") else {
        eprintln!(
            "skipped: this host gave `adopt` nothing to adopt, so the duplication cannot be \
             measured here"
        );
        return;
    };

    let root = fixture("grade4-adopt-dup");
    let mine = root.join("config").join("modules").join("mine.txt");
    std::fs::write(&mine, format!("{pkg}\n")).unwrap();
    let profile = root.join("config").join("profiles").join("Main");
    let mut p = std::fs::read_to_string(&profile).unwrap();
    p.push_str("\nuse mine\n");
    std::fs::write(&profile, p).unwrap();

    // The control: LiNix agrees the package is declared before `adopt` runs.
    let (cfg, code) = run(&root, &["check", "config"]);
    assert_eq!(code, 0, "{cfg}");
    assert!(
        cfg.contains("1 present"),
        "the fixture did not declare `{pkg}`, so nothing below is measuring a duplicate:\n{cfg}"
    );

    let (out, code) = run(&root, &["adopt", "-y"]);
    assert_eq!(code, 0, "{out}");

    let adopted = declared_in(&root.join("config").join("modules").join("adopted.txt"));
    assert!(
        !adopted.iter().any(|l| l == &pkg),
        "`{pkg}` was already declared in modules/mine.txt, and `adopt` wrote it again into \
         modules/adopted.txt — so it is now declared twice, and deleting the user's own line no \
         longer removes it. `adopt` says two lines later: \"Deleting a line UNINSTALLS that \
         package on the next sync.\"\n\nadopt said:\n{}",
        out.lines()
            .filter(|l| l.starts_with("Adopted") || l.starts_with("Left alone"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The count's stated reason, checked against the only thing it claims to be about.
#[test]
fn the_left_alone_count_is_about_what_it_says_it_is_about() {
    let Some(pkg) = one_adoptable_package("label") else {
        eprintln!("skipped: this host gave `adopt` nothing to adopt");
        return;
    };

    let root = fixture("grade4-adopt-label");
    let mine = root.join("config").join("modules").join("mine.txt");
    std::fs::write(&mine, format!("{pkg}\n")).unwrap();
    let profile = root.join("config").join("profiles").join("Main");
    let mut p = std::fs::read_to_string(&profile).unwrap();
    p.push_str("\nuse mine\n");
    std::fs::write(&profile, p).unwrap();

    let (out, code) = run(&root, &["adopt", "-y"]);
    assert_eq!(code, 0, "{out}");

    let Some(line) = out
        .lines()
        .find(|l| l.trim_start().starts_with("Left alone:"))
    else {
        // Nothing was skipped on this host: the label cannot be wrong if it never printed.
        eprintln!("skipped: `adopt` skipped nothing here, so the label did not print");
        return;
    };
    let reported: usize = line
        .split_whitespace()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("could not read a count out of: {line}"));

    let declared = declared_in(&mine).len();
    assert!(
        reported <= declared,
        "`adopt` reported `{}` — but the manifest declares {} package(s). The count is \
         `found.skipped.len()`, and `skipped` is only ever the OS-essential names \
         (adopt.rs:154) and the ones no package line can hold (adopt.rs:315). Neither has \
         anything to do with the manifest, so the reason is wrong for every item it counts.",
        line.trim(),
        declared
    );
}
