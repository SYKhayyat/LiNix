//! `--dry-run` performs nothing — asserted against the real binary, per command.
//!
//! This repo's flagship historical bug was a `--dry-run` that performed the removal. It came
//! back in a quieter place: `--dry-run uninstall` printed `remove 1` and deleted the
//! declaration for real, leaving the package installed and undeclared — drift the next sync
//! would act on, produced by the command that promises to change nothing.
//!
//! The flag was consulted per-verb, so the coverage question is which verbs remembered. Each
//! config-mutating command gets a case here rather than the one that was reported, and each
//! case is paired with a control that performs the same command *without* the flag: an
//! assertion that a file did not change is worthless if the setup could never have changed it.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Fresh {
    dir: PathBuf,
}

impl Fresh {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fresh = Self { dir };
        let (out, code) = fresh.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        fresh
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
            .env("LINIX_CONFIG_DIR", self.dir.join("config"))
            .env("LINIX_DATA_DIR", self.dir.join("data"))
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

    fn config(&self, rel: &str) -> PathBuf {
        self.dir.join("config").join(rel)
    }

    /// Every file in the config repo and its contents, so a preview can be asserted against
    /// the whole repo rather than the one file the test author thought of.
    fn snapshot(&self) -> Vec<(PathBuf, String)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if let Ok(body) = std::fs::read_to_string(&p) {
                    out.push((p, body));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.dir.join("config"), &mut out);
        out.sort();
        out
    }
}

/// `module create` wrote the module during a preview.
#[test]
fn a_previewed_module_create_creates_nothing() {
    let fresh = Fresh::new("dry-run-module-create");
    let before = fresh.snapshot();

    let (out, code) = fresh.run(&["--dry-run", "module", "create", "editors"]);
    assert_eq!(code, 0, "preview failed:\n{out}");
    assert_eq!(
        fresh.snapshot(),
        before,
        "a preview changed the config repo"
    );
    assert!(
        out.contains("DRY-RUN"),
        "a preview that changes nothing must say so:\n{out}"
    );

    // The control: the same command without the flag really does write the file.
    let (out, code) = fresh.run(&["module", "create", "editors"]);
    assert_eq!(code, 0, "the control failed:\n{out}");
    assert!(
        fresh.config("modules/editors.txt").exists(),
        "the fixture cannot create a module at all, so it proves nothing about the preview"
    );
}

/// `schedule add` wrote the `schedules` file during a preview, and `sync` would then have
/// provisioned a real system timer for it.
#[test]
fn a_previewed_schedule_add_schedules_nothing() {
    let fresh = Fresh::new("dry-run-schedule-add");
    let before = fresh.snapshot();

    let (out, code) = fresh.run(&[
        "--dry-run",
        "schedule",
        "add",
        "nightly",
        "--cron",
        "0 3 * * *",
        "--run",
        "linix sync",
    ]);
    assert_eq!(code, 0, "preview failed:\n{out}");
    assert_eq!(
        fresh.snapshot(),
        before,
        "a preview changed the config repo"
    );

    // The control asserts the file, not the exit code. `schedule add` also provisions the host
    // scheduler, and registering a Windows task needs an elevated shell — a fact about the
    // runner, not about whether this test can tell a preview from an act.
    let (out, _) = fresh.run(&[
        "schedule",
        "add",
        "nightly",
        "--cron",
        "0 3 * * *",
        "--run",
        "linix sync",
    ]);
    assert!(
        fresh.config("schedules").exists(),
        "the fixture cannot add a schedule at all, so it proves nothing about the preview:\n{out}"
    );
}

/// The reported case, end to end: a preview of `uninstall` leaves the declaration alone.
///
/// The line is written by a real `linix install` rather than by hand — a hand-written module
/// no active profile reaches is a setup where `undeclare` finds no files and the assertion
/// passes without ever reaching the code it is about.
#[test]
fn a_previewed_uninstall_undeclares_nothing() {
    let fresh = Fresh::new("dry-run-uninstall");

    // `link:` needs no package manager on the runner, and its target need not exist for the
    // declaration to be written: `install` writes the line first and converges second (P1).
    let (out, _) = fresh.run(&["install", "link:none/at/all", "-y"]);
    let manifest = fresh.config("modules/imperative.txt");
    let Ok(before) = std::fs::read_to_string(&manifest) else {
        panic!("the fixture never wrote a manifest:\n{out}");
    };
    assert!(
        before.contains("link:none/at/all"),
        "the fixture declared nothing, so there is nothing to preview removing:\n{out}"
    );

    let (out, _) = fresh.run(&["--dry-run", "uninstall", "link:none/at/all"]);
    assert_eq!(
        std::fs::read_to_string(&manifest).unwrap(),
        before,
        "a preview removed the declaration:\n{out}"
    );
    assert!(
        out.contains("DRY-RUN"),
        "a preview must say what it would have done:\n{out}"
    );

    // The control.
    let (out, _) = fresh.run(&["uninstall", "link:none/at/all", "-y"]);
    assert!(
        !std::fs::read_to_string(&manifest)
            .unwrap()
            .contains("link:none/at/all"),
        "the fixture cannot remove the line at all, so it proves nothing about the preview:\n{out}"
    );
}

/// `unmanage` dropped the declaration *and* rewrote the registry during a preview — the same
/// bug one file over, and the one that also persists LiNix's own state.
#[test]
fn a_previewed_unmanage_forgets_nothing() {
    let fresh = Fresh::new("dry-run-unmanage");

    let (out, _) = fresh.run(&["install", "link:none/at/all", "-y"]);
    let manifest = fresh.config("modules/imperative.txt");
    let Ok(before) = std::fs::read_to_string(&manifest) else {
        panic!("the fixture never wrote a manifest:\n{out}");
    };
    assert!(
        before.contains("link:none/at/all"),
        "fixture declared nothing:\n{out}"
    );

    let (out, _) = fresh.run(&["--dry-run", "unmanage", "link:none/at/all"]);
    assert_eq!(
        std::fs::read_to_string(&manifest).unwrap(),
        before,
        "a preview removed the declaration:\n{out}"
    );

    let (out, _) = fresh.run(&["unmanage", "link:none/at/all"]);
    assert!(
        !std::fs::read_to_string(&manifest)
            .unwrap()
            .contains("link:none/at/all"),
        "the fixture cannot unmanage at all, so it proves nothing about the preview:\n{out}"
    );
}
