//! Every relative link in the documentation points at a file that exists — **with that exact
//! spelling**.
//!
//! Windows and macOS open `README.md` when the file is `readme.md`, so a link written with the
//! wrong case works on the machine it was written on, renders as a 404 on GitHub, and panics in
//! a test that opens it on Linux. The root readme was tracked lower case while the working tree
//! held it upper case, and in one sitting that produced four wrong-case links, one wrong-case
//! `read()`, and a shell link-check over all of them reporting all clear — because it asked the
//! filesystem, and the filesystem was the lenient party. The two names have since been made one
//! (`README.md`, matching every other document at the root), which removes that instance and
//! none of the hazard: any link, in any direction, is one careless keystroke from the same bug.
//!
//! **The authority is git's index, and the first draft of this test got that wrong too.** It
//! compared each link against `read_dir`, which reports what *this working tree* is storing —
//! and the tree and the index disagreed. A fresh clone gets the index's spelling, GitHub renders
//! the index's spelling, and CI checks out the index's spelling, so the working tree's opinion
//! is the one answer that matters least. That draft failed the four links that were correct and
//! passed the ones that were not.
//!
//! `scripts/unix-check.sh` cannot catch any of this either: it compiles the tree for Linux, and a
//! broken path in a Markdown file compiles perfectly.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Markdown files that are the repository's own documentation.
///
/// `target/` holds dependency sources with their own broken links, and `docs/attic/` is kept as
/// residue rather than maintained — its own header says it is read once, by a person.
fn documentation_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let skip: HashSet<&str> = ["target", ".git", "node_modules", "attic", "mutants.out"]
        .into_iter()
        .collect();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !skip.contains(name.as_str()) {
                documentation_files(&path, found);
            }
        } else if name.ends_with(".md") {
            found.push(path);
        }
    }
}

/// The inline links of a Markdown file: the `target` of every `[text](target)`.
///
/// Anchors, external schemes and bare fragments are dropped — this is a question about files.
fn links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("](") {
        rest = &rest[open + 2..];
        let Some(close) = rest.find(')') else { break };
        let target = &rest[..close];
        rest = &rest[close + 1..];
        let target = target.split('#').next().unwrap_or("");
        if target.is_empty()
            || target.starts_with("http")
            || target.starts_with("mailto:")
            || target.starts_with('/')
            // A link containing whitespace is a title or prose, not a path.
            || target.contains(' ')
        {
            continue;
        }
        out.push(target.to_string());
    }
    out
}

/// Every path git tracks, spelled as the index spells it — which is what a clone receives.
///
/// `None` when git cannot answer: this test is about what ships, and without the index it has no
/// way to ask. It says so rather than falling back to the filesystem, whose disagreement with the
/// index is the entire subject.
fn tracked_paths() -> Option<HashSet<String>> {
    let out = Command::new("git")
        .args(["ls-files"])
        .current_dir(repo())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let listed: HashSet<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    (!listed.is_empty()).then_some(listed)
}

/// `docs/./../README.md` -> `README.md`, without touching the disk.
///
/// Lexical on purpose: `canonicalize` would resolve through the filesystem and hand back its
/// spelling, which is the one thing this must not consult.
fn normalise(rel: &str) -> String {
    let slashed = rel.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for part in slashed.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

#[test]
fn every_relative_link_in_the_documentation_resolves_with_the_case_it_was_written_in() {
    let root = repo();
    let mut files = Vec::new();
    documentation_files(&root, &mut files);

    // The self-test. A scan that found nothing would make the assertion below vacuous, which is
    // the failure this repository writes down most often.
    assert!(
        files.len() > 10,
        "found {} markdown files under {}; the scan is broken, not the docs",
        files.len(),
        root.display()
    );

    let Some(tracked) = tracked_paths() else {
        eprintln!(
            "documentation links: SKIPPED — `git ls-files` did not answer, and the working tree \
             is not a substitute for the index here. Nothing was checked."
        );
        return;
    };
    // A link may point at a directory, which git lists only through its contents.
    let dirs: HashSet<String> = tracked
        .iter()
        .flat_map(|p| {
            p.match_indices('/')
                .map(|(i, _)| p[..i].to_string())
                .collect::<Vec<_>>()
        })
        .collect();

    let mut broken = Vec::new();
    let mut checked = 0usize;
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let here = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        let dir = here.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for link in links(&text) {
            // A file git does not track cannot be linked to: it is not in the clone. That
            // includes this repository's own untracked scratch files.
            checked += 1;
            let target = normalise(&format!("{dir}/{link}"));
            if !tracked.contains(&target) && !dirs.contains(&target) {
                broken.push(format!("{here} -> {link}"));
            }
        }
    }

    assert!(
        checked > 30,
        "checked only {checked} links across {} files; the link scan is broken",
        files.len()
    );
    assert!(
        broken.is_empty(),
        "documentation links that do not resolve, or resolve only because this filesystem \
         ignores case:\n  {}\n\nChecked against `git ls-files`, which is what a clone, CI and \
         GitHub all receive. A link whose case differs from the index opens fine on Windows and \
         macOS, 404s on GitHub, and panics on Linux.",
        broken.join("\n  ")
    );
}
