//! The README's install section promises a published binary. That promise must agree with
//! whether a release exists.
//!
//! `scripts/install.sh` tells the reader to pipe it from the web, and it resolves to the newest
//! `v*` release. No tag has ever been pushed, so today it resolves to nothing, notices, and falls
//! back to building from source — fifteen minutes and a Rust toolchain, where the section above it
//! says "no toolchain, no compiler". A note in the README says so, and its last line is an
//! instruction to delete it in the commit that pushes the first tag.
//!
//! **An instruction in prose is the thing that does not happen.** The note is honest only while
//! there is no release; the moment one exists it becomes the lie it was written to prevent, and it
//! is on the first screen every new user reads. So the two are checked against each other.
//!
//! Tags rather than the GitHub API: a test must not need the network, and a tag is what the
//! release job triggers on (`startsWith(github.ref, 'refs/tags/v')`) — so it is the same fact the
//! workflow acts on, not a proxy for it.

use std::path::PathBuf;
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The sentence the note opens with, and the only part of it this test pins.
const NOTE: &str = "No release has been tagged yet";

/// Whether this clone knows of a `v*` tag.
///
/// `None` when git cannot answer, and `None` when the clone knows of no tags at all. The second
/// case is not a repository without releases, it is a checkout that was never told about them:
/// `actions/checkout` fetches no tags by default, so `git tag --list v*` comes back empty and
/// successful on a tree whose release is tagged. Read as "no release exists" that turned this
/// gate upside down and failed every test job on the commit after the 0.8.0 tag. Unknown is
/// reported, never guessed.
fn has_a_version_tag() -> Option<bool> {
    let tags = git(&["tag", "--list"])?;
    if tags.lines().all(|l| l.trim().is_empty()) {
        return None;
    }
    Some(
        git(&["tag", "--list", "v*"])?
            .lines()
            .any(|l| !l.trim().is_empty()),
    )
}

/// `None` when git cannot answer at all.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn the_no_release_yet_note_is_present_exactly_while_there_is_no_release() {
    let readme = std::fs::read_to_string(repo().join("README.md"))
        .expect("README.md")
        .replace("\r\n", "\n");
    let note_present = readme.contains(NOTE);

    let Some(tagged) = has_a_version_tag() else {
        eprintln!(
            "install promise: SKIPPED — this clone knows of no tags, so whether a release exists is \
             unknown. The README currently {} the note.",
            if note_present { "carries" } else { "omits" }
        );
        return;
    };

    // The self-test: if the sentence is ever reworded, this gate silently becomes "assert a
    // string nobody writes", which passes for ever on a tagged repo.
    assert!(
        note_present || tagged,
        "README.md does not contain the sentence {NOTE:?} and no `v*` tag exists. Either the \
         note was deleted before a release was published — which makes the install instructions \
         promise binaries that are not there — or it was reworded, in which case update this \
         test rather than deleting the assertion."
    );

    assert!(
        !(tagged && note_present),
        "a `v*` tag exists and README.md still says {NOTE:?}. That note is on the first screen \
         of the project's front door and is now false; the commit that tagged the release was \
         supposed to remove it."
    );
}
