//! GRADER round 4, 2026-07-30 — RED. `sync` re-places every declared resource on every run, and
//! the second run backs up the file LiNix itself placed.
//!
//! Round 3's N-2 put the extras family into `check` and `plan`. Both now read the machine: with
//! three `link:` lines placed, `check` says *the machine matches your files* and `plan` says
//! *0 resource(s) to place*. The apply loop does not consult either. Measured, same fixture,
//! three consecutive runs:
//!
//!     $ linix sync -y      # nothing placed yet
//!      WARN Link: Cross-drive fallback to COPY for …/src/s1   (×3)
//!     $ ls dest/           s1 s2 s3
//!
//!     $ linix sync -y      # everything already in place
//!      WARN Link: Cross-drive fallback to COPY for …/src/s1   (×3)
//!     already up to date
//!     $ ls dest/           s1 s1.linix-backup s2 s2.linix-backup s3 s3.linix-backup
//!
//! The `.linix-backup` files are backups of LiNix's own copies, created in the user's directory,
//! by a run that reported `already up to date`. A user's genuine pre-existing file is backed up
//! once and then never overwritten, which is the one thing that goes right here.
//!
//! Two properties are asserted below, and the second is the one with teeth: the *work* is
//! unconditional. `ExtrasManager::in_effect` already answers "is this resource in place?" — the
//! probe `check` and `plan` were given in round 3 — and the loop that places them never asks.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("dest")).unwrap();
        let f = Self { root };
        let (out, code) = f.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        let profile = f.cfg().join("profiles").join("Main");
        let mut p = std::fs::read_to_string(&profile).unwrap();
        p.push_str("\nuse extras\n");
        std::fs::write(&profile, p).unwrap();

        let mut lines = String::new();
        for i in 0..3 {
            let src = f.root.join("src").join(format!("s{i}"));
            let dest = f.root.join("dest").join(format!("s{i}"));
            std::fs::write(&src, format!("content-{i}\n")).unwrap();
            lines.push_str(&format!(
                "link:{} @target={}\n",
                src.display().to_string().replace('\\', "/"),
                dest.display().to_string().replace('\\', "/")
            ));
        }
        std::fs::write(f.cfg().join("modules").join("extras.txt"), lines).unwrap();
        f
    }

    fn cfg(&self) -> PathBuf {
        self.root.join("config")
    }

    fn dest_listing(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.root.join("dest"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
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
}

#[test]
fn a_second_sync_leaves_no_backup_of_linixs_own_file() {
    let f = Fixture::new("grade3-resource-idempotency");

    let (first, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "{first}");
    let after_first = f.dest_listing();
    assert_eq!(
        after_first,
        vec!["s0", "s1", "s2"],
        "the fixture's own first sync did not place exactly three files:\n{first}"
    );

    let (second, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "{second}");

    assert_eq!(
        f.dest_listing(),
        after_first,
        "a second `sync` with nothing to do added files to the user's directory. `check` reports \
         `the machine matches your files` and `plan` reports `0 resource(s) to place` for this \
         same state; the apply loop asks neither, re-copies all three, and backs up the copies it \
         made itself. sync #2 said:\n{}",
        second.trim()
    );
}

/// The work itself, not just its litter: a run with nothing to do must not report doing the
/// placement. `check` and `plan` both say there is nothing to place.
#[test]
fn a_second_sync_does_not_re_place_what_is_already_in_place() {
    let f = Fixture::new("grade3-resource-work");
    let (first, _) = f.run(&["sync", "-y"]);
    let placements_first = first.matches("Link:").count();
    assert!(
        placements_first >= 3,
        "the fixture's own first sync placed nothing, so a quiet second run would prove \
         nothing:\n{first}"
    );

    let (check, _) = f.run(&["check"]);
    assert!(
        check.contains("the machine matches your files"),
        "`check` does not consider this converged, so `sync` repeating the work is not yet a \
         disagreement:\n{check}"
    );

    let (second, _) = f.run(&["sync", "-y"]);
    assert_eq!(
        second.matches("Link:").count(),
        0,
        "`sync` re-placed {} resource(s) that `check` had just called converged, under a summary \
         reading `already up to date`:\n{}",
        second.matches("Link:").count(),
        second.trim()
    );
}
