//! `dotfiles:` and `link:` given the identical job, side by side.
//!
//! `verbs/sync.rs` calls a dotfiles tree *"a pile of `link:` lines"* and applies it in the same
//! phase. Four documents say the two share a lifecycle:
//!
//! - `model/dotfiles.rs` — *"The cost is walking the tree each sync and one ledger row per file,
//!   and that is the cost worth paying."*
//! - `core/extras_lock.rs` — *"its files ARE keyed here, but individually by the tree applier —
//!   one ledger row per placed file (U22)."*
//! - `docs/spec/history.md` — *"each file keyed individually in the extras ledger so the teardown
//!   is the shared one."*
//! - `docs/spec/plan.md`, item 7n, marked **DONE 2026-07-24**, whose stated exit condition is
//!   *"a file deleted from the tree has its link removed by the same `extras_lock` teardown every
//!   other extra uses."*
//!
//! No code wrote that row. `apply/dotfiles.rs` contained no reference to `ExtrasLedger` at all, so
//! the tree got none of what `link:` has: no `<target>.shall-backup` before it replaced a file
//! (T6), no ledger entry, and therefore no teardown, no restore, and no removal guard.
//!
//! Each test below runs the two statements against the same bytes in the same tree and asserts
//! they answer the same. **The control is the `link:` half**: if it ever stops preserving the
//! user's file, these tests fail on that line first and say so, rather than passing because both
//! halves broke together.

use std::path::{Path, PathBuf};

use crate::harness::Fixture;

/// The shared root, plus what these tests need in it.
fn setup(name: &str) -> Fixture {
    let f = Fixture::new(name);
    std::fs::create_dir_all(f.root.join("tree")).unwrap();
    std::fs::create_dir_all(f.root.join("single")).unwrap();
    std::fs::create_dir_all(f.root.join("dest")).unwrap();
    let profile = f.cfg().join("profiles").join("Main");
    let mut p = std::fs::read_to_string(&profile).unwrap();
    p.push_str("\nuse extras\n");
    std::fs::write(&profile, p).unwrap();

    std::fs::write(f.root.join("tree").join("by_tree"), "managed\n").unwrap();
    std::fs::write(f.root.join("single").join("by_link"), "managed\n").unwrap();
    f.declare_both();
    f
}

/// A tree with one file, and a `link:` line naming a second file with the same content — the
/// same job stated the two ways. The fixture root is the child's `HOME`, so every destination is
/// inside home by construction rather than by where the checkout happens to sit.
impl Fixture {
    fn slash(p: &Path) -> String {
        p.display().to_string().replace('\\', "/")
    }

    /// Both statements, pointed at the same destination directory.
    fn declare_both(&self) {
        let lines = format!(
            "dotfiles:{} @target={}\nlink:{} @target={}\n",
            Self::slash(&self.root.join("tree")),
            Self::slash(&self.root.join("dest")),
            Self::slash(&self.root.join("single").join("by_link")),
            Self::slash(&self.root.join("dest").join("by_link")),
        );
        std::fs::write(self.cfg().join("modules").join("extras.txt"), lines).unwrap();
    }

    /// Only the `link:` line — the tree's declaration is gone, which is what removal means.
    fn declare_link_only(&self) {
        let lines = format!(
            "link:{} @target={}\n",
            Self::slash(&self.root.join("single").join("by_link")),
            Self::slash(&self.root.join("dest").join("by_link")),
        );
        std::fs::write(self.cfg().join("modules").join("extras.txt"), lines).unwrap();
    }

    fn dest(&self, name: &str) -> PathBuf {
        self.root.join("dest").join(name)
    }

    /// What is sitting at a destination, following no symlink: `None` when nothing is there.
    fn placed(&self, name: &str) -> Option<String> {
        let p = self.dest(name);
        if !p.exists() && !p.is_symlink() {
            return None;
        }
        Some(std::fs::read_to_string(&p).unwrap_or_else(|_| "<unreadable>".into()))
    }

    fn backup_of(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(format!("{}.shall-backup", self.dest(name).display())).ok()
    }
}

/// T6, asked of both statements: a file the user already had is preserved before Shall takes the
/// path over, so nobody is silently robbed of a config file they hand-wrote.
///
/// `--replace-existing` waives the *refusal* to overwrite. It has never meant "and throw the
/// original away" — the `link:` half of this fixture proves that, because it preserves the file
/// on the same run with the same flag.
#[test]
fn a_users_existing_file_is_preserved_by_both_statements() {
    let f = setup("dotfiles-preserves-the-original");

    // Two files the user wrote by hand, one at each destination.
    std::fs::write(f.dest("by_tree"), "the user wrote this\n").unwrap();
    std::fs::write(f.dest("by_link"), "the user wrote this\n").unwrap();

    let (out, code) = f.run(&["sync", "-y", "--replace-existing"]);
    assert_eq!(code, 0, "{out}");

    // The instrument, self-tested: if neither destination was taken over, no backup would be
    // expected of either and the assertions below could not fail.
    assert_eq!(
        f.placed("by_tree").as_deref(),
        Some("managed\n"),
        "the tree did not take its destination over, so this test proves nothing:\n{out}"
    );
    assert_eq!(
        f.placed("by_link").as_deref(),
        Some("managed\n"),
        "the link did not take its destination over, so the control below proves nothing:\n{out}"
    );

    // The control: `link:` has done this since T6.
    assert_eq!(
        f.backup_of("by_link").as_deref(),
        Some("the user wrote this\n"),
        "the `link:` control lost the user's file. T6 is the behaviour this whole test compares \
         against, so fix that first:\n{out}"
    );

    assert_eq!(
        f.backup_of("by_tree").as_deref(),
        Some("the user wrote this\n"),
        "`dotfiles:` destroyed a file the user hand-wrote. The identical `link:` line on the \
         same run preserved its own as `<target>.shall-backup`; the tree called `remove_file` \
         and symlinked over the top, so the original is not recoverable from anywhere:\n{out}"
    );
}

/// S20, asked of both statements: deleting the declaration removes the thing, and puts back what
/// was there before it.
///
/// The tree's declaration is deleted outright, which is 7n's stated exit condition. The `link:`
/// line stays declared, so it is the control for "this sync ran and the teardown was reached".
#[test]
fn deleting_the_declaration_undoes_it_and_restores_what_was_there() {
    let f = setup("dotfiles-teardown");

    std::fs::write(f.dest("by_tree"), "the user wrote this\n").unwrap();
    let (out, code) = f.run(&["sync", "-y", "--replace-existing"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        f.placed("by_tree").as_deref(),
        Some("managed\n"),
        "nothing was placed, so removing the declaration would prove nothing:\n{out}"
    );
    assert_eq!(
        f.placed("by_link").as_deref(),
        Some("managed\n"),
        "the control line placed nothing:\n{out}"
    );

    // The tree's line goes away. `link:` keeps its own, so a quiet run cannot be mistaken for
    // a teardown that ran.
    f.declare_link_only();
    let (out, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "{out}");

    assert_eq!(
        f.placed("by_link").as_deref(),
        Some("managed\n"),
        "the still-declared `link:` was torn down, which is the teardown reaching too far:\n{out}"
    );
    assert_eq!(
        f.placed("by_tree").as_deref(),
        Some("the user wrote this\n"),
        "the `dotfiles:` line was deleted and its file stayed. S20 is that removing a line \
         removes the thing, and 7n's exit condition says a departed tree file is undone by the \
         same `extras_lock` teardown every other extra uses — so the user's original should be \
         back at that path:\n{out}"
    );
}

/// The same question one level down: a file deleted from *inside* a still-declared tree.
///
/// This is the case the tree exists for — you delete a dotfile from your repo and expect it to
/// leave the machine — and the one a per-tree ledger key could never answer, which is why 7n
/// specified a row per placed file rather than a row per tree.
#[test]
fn a_file_deleted_from_the_tree_leaves_the_machine() {
    let f = setup("dotfiles-file-departs");

    std::fs::write(f.root.join("tree").join("second"), "also managed\n").unwrap();
    let (out, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        f.placed("second").as_deref(),
        Some("also managed\n"),
        "the second tree file was never placed, so deleting it proves nothing:\n{out}"
    );

    std::fs::remove_file(f.root.join("tree").join("second")).unwrap();
    let (out, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "{out}");

    assert_eq!(
        f.placed("by_tree").as_deref(),
        Some("managed\n"),
        "the file still in the tree was removed too:\n{out}"
    );
    assert_eq!(
        f.placed("second"),
        None,
        "a file deleted from the dotfiles tree left its link on the machine. That is 7n's \
         exit condition, and the row that would let the shared teardown see it was never \
         written:\n{out}"
    );
}
