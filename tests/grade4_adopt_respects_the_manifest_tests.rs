//! GRADER round 5, 2026-07-30 — RED. `adopt` offers a package that is already declared, writing a
//! second declaration for it in another module; and its summary explains the packages it skipped
//! with a reason that is never the reason.
//!
//! Measured on Windows, one package declared and installed:
//!
//!     $ cat config/modules/mine.txt
//!     cargo:ripgrep
//!     $ shall check config
//!     OK: … 1 present, 0 absent, 0 repo/shim/service/link/schedule line(s).
//!
//!     $ shall adopt -y
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
//!     $ shall --dry-run sync -y
//!     already up to date
//!     $ shall why cargo:ripgrep
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

/// Ask Shall itself what this host has to adopt, so the test names a package that really is
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

    // The control: Shall agrees the package is declared before `adopt` runs.
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

    // BUILDER round 6, re-aimed at the same subject. The original assertion was
    // `reported <= declared_in(&mine).len()` — the only check available while the whole count
    // wore one sentence, *"(listed in the manifest)"*. That sentence is gone; the count is now
    // broken down by the reason each item actually carries, so the question "is this count
    // about what it says it is about" is answerable directly instead of by proxy.
    //
    // Two assertions, and the first is what keeps this honest: a breakdown that does not add
    // up to its own total is the same defect wearing a longer message. Pre-fix there is no
    // breakdown at all, so the sum is 0 and this goes red exactly as the original did.
    let breakdown: Vec<(usize, String)> = out
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("Left alone:"))
        .skip(1)
        .map_while(|l| {
            let t = l.trim_start();
            let (n, reason) = t.split_once("  ")?;
            Some((n.trim().parse::<usize>().ok()?, reason.trim().to_string()))
        })
        .collect();

    let summed: usize = breakdown.iter().map(|(n, _)| n).sum();
    assert_eq!(
        summed,
        reported,
        "`{}` is explained by reasons adding up to {}:\n{:#?}\n\nA rollup whose parts do not \
         account for its whole is the defect this test is about — the old version printed the \
         whole under one reason (`found.skipped.len()` labelled \"listed in the manifest\") \
         that was true for none of its inputs.",
        line.trim(),
        summed,
        breakdown
    );

    // And the original proxy, now applied where it belongs: only the manifest reason may be
    // measured against the manifest.
    let declared = declared_in(&mine).len();
    let blamed_on_the_manifest: usize = breakdown
        .iter()
        .filter(|(_, r)| r.contains("already declare"))
        .map(|(n, _)| n)
        .sum();
    assert!(
        blamed_on_the_manifest <= declared,
        "{} package(s) were left alone because they are already declared, but only {} are \
         declared:\n{:#?}",
        blamed_on_the_manifest,
        declared,
        breakdown
    );
}
