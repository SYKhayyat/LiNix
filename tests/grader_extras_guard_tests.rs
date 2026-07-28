//! GRADER, 2026-07-28 — RED. A removal path with no guard, no count, and no plan line.
//!
//! `readme.md:266` — "**every path that removes anything** goes through one guard".
//! `src/app/sync/guard.rs:9` — "*Every* path that deletes is guarded, not just the reviewed
//! ones. A guard on one command is a guard on nothing: the bug that motivated this arrived
//! through `prune`, which nobody thought to check."
//!
//! `src/app/apply/extras.rs:105` deletes and the word `guard` does not appear in that file.
//! It is the undo path for every non-package resource LiNix manages — `link:`, `service:`,
//! `setting:`, `shim:`, `schedule:`, `repo:`. When a declaration leaves the model, this loop
//! tears the resource down directly through `inst.remove(...)`, outside the transaction, and
//! outside `guard::enforce`.
//!
//! Measured, with the guard configured and confirmed effective by `linix protected`
//! (`max_removals = 1`, `protected_packages = ["f3"]`):
//!
//!     $ linix sync -y          # five link: declarations applied
//!     $ : > modules/starter.txt
//!     $ linix --dry-run sync
//!     already up to date
//!     $ linix sync -y
//!     already up to date
//!     $ ls <targets>
//!     (empty)
//!
//! Five managed files deleted. `--dry-run` did not mention them, `sync` reported no changes,
//! the count of five never met `max_removals = 1`, and `f3` was removed while protected.
//!
//! Three separate failures, in the order that matters:
//!   1. the removal is invisible — nothing in `plan`, `--dry-run`, or the summary names it;
//!   2. the count is never computed, so `max_removals` cannot apply to it;
//!   3. `protected_packages` does not reach it, and V.26 says protection is a refusal that
//!      nothing overrides.
//!
//! `link:` is the mildest member of the family — the source stays in the config repo, so the
//! bytes are recoverable. `service:`, `setting:` and `shim:` have no such net.

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
        f
    }

    fn cfg(&self) -> PathBuf {
        self.root.join("config")
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
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

/// Forward slashes: the grammar reads `\` as an escape, and a Windows path written raw into a
/// module does not survive the parse.
fn decl(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[test]
fn undeclaring_managed_extras_goes_through_the_removal_guard() {
    let f = Fixture::new("extras-guard");
    let sources = f.cfg().join("dotfiles");
    std::fs::create_dir_all(&sources).unwrap();

    // A guard that must refuse this: five removals against a limit of one, and one of the
    // five named protected.
    std::fs::write(
        f.cfg().join("preferences.toml"),
        "[guard]\nmax_removals = 1\nprotected_packages = [\"f3\"]\n",
    )
    .unwrap();
    let (rules, _) = f.run(&["protected"]);
    assert!(
        rules.contains("f3") && rules.contains("1"),
        "the fixture's own guard config did not take effect, so this test would prove \
         nothing:\n{rules}"
    );

    let mut module = String::new();
    let mut targets = Vec::new();
    for i in 1..=5 {
        let src = sources.join(format!("f{i}"));
        let dst = f.root.join("dest").join(format!("f{i}"));
        std::fs::write(&src, format!("content-{i}\n")).unwrap();
        module.push_str(&format!("link:{} @target={}\n", decl(&src), decl(&dst)));
        targets.push(dst);
    }
    std::fs::write(f.cfg().join("modules/starter.txt"), &module).unwrap();

    let (out, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "applying the link declarations failed:\n{out}");
    for t in &targets {
        assert!(t.exists(), "setup did not apply {}:\n{out}", t.display());
    }

    // Undeclare all five.
    std::fs::write(f.cfg().join("modules/starter.txt"), "").unwrap();

    let (preview, _) = f.run(&["--dry-run", "sync"]);
    let (applied, rc) = f.run(&["sync", "-y"]);
    let gone: Vec<String> = targets
        .iter()
        .filter(|t| !t.exists())
        .map(|t| t.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert!(
        gone.is_empty(),
        "`sync` deleted {} managed file(s) — {:?} — with the guard set to refuse more than \
         one removal and to protect `f3`.\n\
         It exited {rc} and reported no changes.\n\
         --- linix --dry-run sync ---\n{}\n--- linix sync -y ---\n{}\n\n\
         src/app/apply/extras.rs:105 calls `inst.remove(...)` directly; the word `guard` does \
         not appear in that file. readme.md:266 says every path that removes anything goes \
         through one guard.",
        gone.len(),
        gone,
        preview.trim(),
        applied.trim(),
    );
}

/// Even before the guard, a deletion the user cannot see coming is the wrong shape.
#[test]
fn a_sync_that_tears_down_extras_says_so_before_it_does_it() {
    let f = Fixture::new("extras-visible");
    let sources = f.cfg().join("dotfiles");
    std::fs::create_dir_all(&sources).unwrap();

    let src = sources.join("vimrc");
    let dst = f.root.join("dest").join("vimrc");
    std::fs::write(&src, "set nocompatible\n").unwrap();
    std::fs::write(
        f.cfg().join("modules/starter.txt"),
        format!("link:{} @target={}\n", decl(&src), decl(&dst)),
    )
    .unwrap();

    let (out, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "applying the link failed:\n{out}");
    assert!(dst.exists(), "setup did not apply the link:\n{out}");

    std::fs::write(f.cfg().join("modules/starter.txt"), "").unwrap();
    let (preview, _) = f.run(&["--dry-run", "sync"]);

    assert!(
        preview.contains("vimrc") || preview.contains("remove 1"),
        "`--dry-run sync` was about to delete {} and said:\n  {}\n\n\
         A preview that reports `already up to date` before removing a managed file is the \
         same event as a plan that under-reports a removal.",
        dst.display(),
        preview.trim()
    );
}
