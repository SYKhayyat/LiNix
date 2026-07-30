//! GRADER round 4, 2026-07-30 — RED. `--dry-run` writes the managed-state registry, and the
//! gate that says "every subcommand" never looks at the directory it writes to.
//!
//! Measured on Windows, fresh `LINIX_CONFIG_DIR`/`LINIX_DATA_DIR`, one binary:
//!
//!     $ linix --dry-run adopt -y
//!     Adopted 112 package(s).                      <- past tense, no [DRY-RUN] marker
//!     Manifest:  …/cfg/modules/adopted.txt         <- and this file was NOT written
//!
//!     $ ls data/registry.json
//!     -rw-r--r-- 29645 registry.json               <- 112 packages recorded as managed
//!
//!     $ linix check
//!     ok  config      0 package(s) declared
//!     ->  drift       0 to install, 112 to remove, 0 to place, 0 to undo
//!                        run `linix sync`
//!
//! So a preview leaves the machine in the one state the model reads as *the user deleted every
//! line*: managed, undeclared. Driven end to end in a disposable data directory — one github
//! package installed, unmanaged, then `adopt --dry-run`, then the `sync` that `check` recommends:
//!
//!     Removals: 1     ✓ [github  ] sharkdp/hexyl        (70ms)
//!
//! The package was uninstalled. Above `max_removals` the count guard refuses first and offers
//! `--allow-mass-removal`; at or below it — any machine with fewer than 20 adopted packages —
//! nothing stands in the way.
//!
//! `adopt` routes its *manifest* write through `Writes::for_run(dry_run)` and then calls
//! `state_mut.add()` + `save()` with no dry-run test at all (`src/app/adopt.rs:236-255`). `hold`
//! and `unhold` do the same (`src/verbs/packages.rs:605,624`). The sibling that was fixed proves
//! the class was known — `src/verbs/cleanup.rs:467`:
//!
//!     // The registry is what LiNix believes it manages. A preview that persisted `forget`
//!     // would leave the package unmanaged for real while promising it had changed nothing.
//!     if !app.config.dry_run {
//!
//! **Why no gate caught it.** `tests/dry_run_every_verb_tests.rs` asserts that every subcommand
//! `--help` lists is exercised or exempted with a reason. Its `snapshot()` walks the **config**
//! directory only, and its exemption list reads:
//!
//!     ("hold",   "holds live in the data dir, not the config dir"),
//!     ("unhold", "holds live in the data dir, not the config dir"),
//!     ("adopt",  "needs installed packages to adopt"),
//!
//! Two of those are true statements that are not reasons: they name the instrument's blind spot
//! and treat it as the verb's alibi. The data directory holds `registry.json` — the file that
//! decides whether the next `sync` removes a package — and no dry-run assertion in this repo has
//! ever read it.
//!
//! These tests snapshot **both** directories, and each carries the control the same file
//! demands: the same command without the flag must change what the dry run left alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let f = Self { root };
        let (out, code) = f.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        f
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
            .current_dir(&self.root)
            .env("LINIX_CONFIG_DIR", self.root.join("config"))
            .env("LINIX_DATA_DIR", self.data())
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

    /// Only the state registry, by content. The rest of the data directory holds caches and a
    /// lock file that an ordinary read touches, and a snapshot of those would fail for reasons
    /// that are not this finding.
    fn registry(&self) -> Option<Vec<u8>> {
        std::fs::read(self.data().join("registry.json")).ok()
    }
}

/// A machine with a package no manager needs to be present for: `hold` records against the name
/// it is given.
fn declare(f: &Fixture, line: &str) {
    let m = f.root.join("config").join("modules").join("starter.txt");
    std::fs::write(&m, format!("{line}\n")).unwrap();
}

#[test]
fn dry_run_hold_does_not_record_a_hold() {
    let f = Fixture::new("grade3-dry-hold");
    declare(&f, "github:sharkdp/hexyl");

    let before = f.registry();
    let (out, code) = f.run(&["--dry-run", "hold", "github:sharkdp/hexyl"]);
    let after = f.registry();

    // The control: without the flag, the same command must change the registry. If it does not,
    // this fixture proves nothing and the assertion below could not have failed.
    let (ctl_out, _) = f.run(&["hold", "github:sharkdp/hexyl"]);
    assert!(
        f.registry() != before,
        "the control did not change the registry either, so this case cannot fail:\n{ctl_out}"
    );

    assert_eq!(
        after, before,
        "`linix --dry-run hold` wrote the managed-state registry. It also said `{}` in the past \
         tense, with no [DRY-RUN] marker.",
        out.lines().next().unwrap_or("").trim()
    );
    assert_eq!(code, 0, "{out}");
}

#[test]
fn dry_run_unhold_does_not_release_a_hold() {
    let f = Fixture::new("grade3-dry-unhold");
    declare(&f, "github:sharkdp/hexyl");
    f.run(&["hold", "github:sharkdp/hexyl"]);

    let before = f.registry();
    let (out, _) = f.run(&["--dry-run", "unhold", "github:sharkdp/hexyl"]);
    let after = f.registry();

    // Control: the real unhold changes it.
    f.run(&["unhold", "github:sharkdp/hexyl"]);
    assert!(
        f.registry() != before,
        "the control did not change the registry, so this case cannot fail"
    );

    assert_eq!(
        after, before,
        "`linix --dry-run unhold` wrote the managed-state registry, and printed `{}`.",
        out.lines().next().unwrap_or("").trim()
    );
}

/// The blocker. Not "a file changed" — *what the change means to the next command*.
///
/// `adopt` needs installed packages, so this test skips where there are none rather than passing
/// on an empty machine. A skip is honest; a pass on a fixture where the command was a no-op is
/// the failure mode `dry_run_every_verb_tests.rs` was written to end.
#[test]
fn dry_run_adopt_leaves_nothing_managed_and_undeclared() {
    let f = Fixture::new("grade3-dry-adopt");

    let (before_check, _) = f.run(&["check"]);
    assert!(
        before_check.contains("the machine matches your files"),
        "the fixture did not start converged, so the assertion below would be about something \
         else:\n{before_check}"
    );

    let (out, code) = f.run(&["--dry-run", "adopt", "-y"]);
    if out.contains("Nothing to adopt") {
        eprintln!("SKIPPED: no unmanaged package on this host for `adopt` to find");
        return;
    }
    assert_eq!(code, 0, "{out}");

    let registry_written = f.registry().is_some();
    let (after_check, _) = f.run(&["check"]);

    assert!(
        !registry_written,
        "`linix --dry-run adopt` wrote data/registry.json. It printed `{}` — past tense, no \
         [DRY-RUN] marker — and did NOT write the manifest it named, so every package it \
         discovered is now managed and undeclared. `check` says:\n{}\n\nThat is the state the \
         model reads as \"the user deleted every line\", and the `sync` this output recommends \
         acts on it.",
        out.lines()
            .find(|l| l.contains("Adopted"))
            .unwrap_or("")
            .trim(),
        after_check
            .lines()
            .filter(|l| l.contains("drift") || l.contains("config"))
            .collect::<Vec<_>>()
            .join("\n"),
    );

    assert!(
        !after_check.contains(" to remove,") || after_check.contains("0 to remove,"),
        "after a *preview*, `check` reports removals:\n{after_check}"
    );
}

/// The coverage property, stated as a test rather than as a note: the dry-run gate must read the
/// directory the registry lives in. Asserted through the gate's own exemption list, because that
/// list is where the blind spot is written down.
#[test]
fn the_dry_run_gate_does_not_excuse_a_verb_for_writing_where_it_cannot_see() {
    let src = include_str!("dry_run_every_verb_tests.rs");
    let excused_for_the_data_dir: Vec<&str> = src
        .lines()
        .filter(|l| l.contains("data dir") && l.trim_start().starts_with('('))
        .collect();
    assert!(
        excused_for_the_data_dir.is_empty(),
        "`dry_run_every_verb_tests.rs` exempts verbs *because* they write to the directory it \
         does not snapshot:\n  {}\n\nIts `snapshot()` walks the config directory only. \
         `data/registry.json` is the managed set — the file that decides whether the next \
         `sync` removes a package. An exemption that names the instrument's blind spot is not a \
         reason; it is the finding.",
        excused_for_the_data_dir.join("\n  ")
    );

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for l in src.lines() {
        if l.contains("LINIX_DATA_DIR") {
            *counts.entry("data dir wired into the fixture").or_default() += 1;
        }
        if l.contains("fn snapshot") {
            *counts.entry("snapshot defined").or_default() += 1;
        }
    }
    eprintln!("{counts:?}");
}
