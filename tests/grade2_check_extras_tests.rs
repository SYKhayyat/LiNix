//! GRADER round 3, 2026-07-29 — RED. `check drift` says the machine matches while a declared
//! resource is missing from it.
//!
//! `linix check` is the command whose whole job is the question *does the machine match your
//! files?* Measured on Windows, with three `link:` lines declared and nothing placed yet:
//!
//!     $ linix check
//!     ok  config      0 package(s) declared
//!     ok  drift       the machine matches your files
//!
//! Then, after `sync -y` placed all three, with one deleted behind LiNix's back:
//!
//!     $ rm dest/s2 && linix check
//!     ok  drift       the machine matches your files
//!
//! Control, same fixture, same binary — drift is not broken in general:
//!
//!     $ echo 'scoop:linix-no-such-pkg-zzz' > modules/starter.txt && linix check
//!     ok  config      1 package(s) declared
//!     ->  drift       1 to install, 0 to remove
//!
//! and with **both** a missing package and a missing `link:` declared, it still reports exactly
//! `1 to install`. The extras family is not counted, not compared, and not reported.
//!
//! This is one family with `grade2_plan_extras_tests.rs`, and stating it once: **`link:`,
//! `service:`, `setting:`, `shim:`, `schedule:` and `repo:` are outside the model everywhere
//! except the apply loop itself.** `sync` places them (and prints `already up to date` while
//! doing it), `--dry-run sync` previews them, and the round-2 guard now counts them for a
//! refusal — but `check`, `plan` and `apply` cannot see them in either direction.
//!
//! Round 2 closed G-1 at the guard. The guard was one of three failures that finding listed;
//! this is the other two — "the removal is invisible" and "nothing names it" — in the commands
//! a user runs *before* they get anywhere near a refusal.
//!
//! Severity is not cosmetic: a green `check` is what a user reads to decide the machine is
//! converged. On this tree it is green over a dotfile that is not there.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("dest")).unwrap();
        let f = Self { root };
        let (out, code) = f.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        std::fs::create_dir_all(f.cfg().join("dotfiles")).unwrap();
        f
    }

    fn cfg(&self) -> PathBuf {
        self.root.join("config")
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
            .current_dir(&self.root)
            .env("LINIX_CONFIG_DIR", self.cfg())
            .env("LINIX_DATA_DIR", self.root.join("data"))
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

    fn decl(p: &Path) -> String {
        p.to_string_lossy().replace('\\', "/")
    }

    /// One `link:` line whose target does not exist, and the target path.
    fn declare_one_link(&self) -> PathBuf {
        let src = self.cfg().join("dotfiles").join("d1");
        let dst = self.root.join("dest").join("d1");
        std::fs::write(&src, "content\n").unwrap();
        std::fs::write(
            self.cfg().join("modules/starter.txt"),
            format!("link:{} @target={}\n", Self::decl(&src), Self::decl(&dst)),
        )
        .unwrap();
        dst
    }
}

/// The drift line is the one sentence a user reads to decide whether to run `sync`.
#[test]
fn check_reports_drift_for_a_declared_resource_that_is_not_there() {
    let f = Fixture::new("grade2-check-drift");
    let target = f.declare_one_link();
    assert!(!target.exists(), "the fixture's target should start absent");

    // Control: drift itself works. A missing *package* is reported, on this same fixture, so a
    // green run below cannot be explained by "check is broken" or "the module did not parse".
    std::fs::write(
        f.cfg().join("modules/starter.txt"),
        "scoop:linix-no-such-pkg-zzz\n",
    )
    .unwrap();
    // U21: a read-only command that looked and found work exits 2, so 0 and 2 both mean it ran.
    let (control, code) = f.run(&["check"]);
    assert!(
        code == 0 || code == 2,
        "`check` failed ({code}):\n{control}"
    );
    assert!(
        control.contains("to install"),
        "the control failed — `check` did not report a missing package as drift, so this test \
         would prove nothing:\n{control}"
    );

    let target = f.declare_one_link();
    let (out, code) = f.run(&["check"]);
    assert!(code == 0 || code == 2, "`check` failed ({code}):\n{out}");
    assert!(
        !out.contains("the machine matches your files"),
        "`check` reported the machine matches, while {} is declared and is not on disk. \
         `sync` on the same tree places it.\n{out}",
        target.display()
    );
}

/// The same question after convergence: something LiNix placed is deleted behind its back.
/// This is `rebuild`'s and `heal`'s reason to exist, and `check` is how a user finds out.
#[test]
fn check_reports_drift_when_a_placed_resource_is_deleted_behind_linixs_back() {
    let f = Fixture::new("grade2-check-drift-after");
    let target = f.declare_one_link();

    let (out, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "the fixture's own `sync` failed:\n{out}");
    assert!(
        target.exists(),
        "setup did not place {}:\n{out}",
        target.display()
    );

    std::fs::remove_file(&target).unwrap();
    let (out, code) = f.run(&["check"]);
    assert!(code == 0 || code == 2, "`check` failed ({code}):\n{out}");
    assert!(
        !out.contains("the machine matches your files"),
        "a file LiNix placed was deleted and `check` still reports the machine matches your \
         files. The one command whose job is that question is green over the gap.\n{out}"
    );
}

/// `sync` placed three files and said `already up to date`. The per-item lines are printed by
/// the apply loop; the summary counts packages only, so the sentence a user reads last is the
/// one that is wrong.
#[test]
fn sync_does_not_report_no_change_while_placing_resources() {
    let f = Fixture::new("grade2-sync-summary");
    let target = f.declare_one_link();

    let (out, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "`sync` failed:\n{out}");
    assert!(
        target.exists(),
        "the control failed — `sync` placed nothing, so there is no claim to contradict:\n{out}"
    );
    assert!(
        !out.contains("already up to date"),
        "`sync` placed {} and reported `already up to date`.\n{out}",
        target.display()
    );
}
