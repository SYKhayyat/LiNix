//! A comment does not cite a source line by number, and every file it names exists.
//!
//! **What was found.** Comments in this tree cited each other by line number, and on 2026-08-13
//! **29 of 37 such citations no longer landed within four lines of the symbol they named**; one
//! named `e2e_tests.rs`, a file the repository had deleted. Two were load-bearing: `guard.rs`'s
//! `Reaped::for_reason` doc says *"`grep -rn "Reaped::for_reason"` is the list of places that do
//! not ask"*, and each of the two production entries on that list justified itself by naming the
//! line where the guard is enforced instead. Both numbers were wrong — by 214 and 269 lines —
//! and both landed in unrelated code that read plausibly enough to be mistaken for the thing
//! cited. A reviewer auditing the removal guard was sent to the wrong place, twice.
//!
//! **Why a number and not a name.** A line number is the one citation that goes stale with
//! nobody touching it: the refactor of 2026-08-12 moved 1,800 lines and everything below it
//! shifted. Nothing noticed, which is `GRADER.md` §2.2b's class exactly — a claim with an expiry
//! and no alarm.
//!
//! **So the numbers were not corrected, they were removed** — 73 of them across 25 files.
//! Renumbering buys one green run; the next refactor takes it back. What survives is the symbol,
//! because a refactor carries a symbol with the code it names: `plan.rs`'s `apply` still points
//! at `apply` wherever it moved to, and `plan.rs:483` points at whatever is at line 483 today.
//!
//! **This gate is therefore a prohibition, and that is deliberate.** The first version asked the
//! weaker question — *do the surviving citations still land?* — and a tree with no line citations
//! left answers "yes, all zero of them". A gate phrased that way stops measuring at the exact
//! moment it starts passing, and nothing says so; the review that produced this file spends four
//! pages on instruments that read a transcription of their subject, and a vacuous gate is the
//! same failure with the subject removed entirely. A prohibition cannot go vacuous, because its
//! subject is what somebody writes tomorrow.
//!
//! *An earlier draft of the weak form is worth recording, since it is the reason the weak form
//! was weak: requiring* every *named symbol to sit near the cited line produced 53 failures where
//! hand-checking found 15, and a screening instrument that cries wolf gets deleted — after which
//! the fifteen come back.*

use std::path::{Path, PathBuf};

const SELF: &str = "a_citation_in_a_comment_still_points_at_its_claim_tests.rs";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Every `.rs` under `src/` and `tests/`, so a citation can be resolved against either.
fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["src", "tests"] {
        let mut stack = vec![root.join(dir)];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// A citation lifted out of a comment: where it was written, what it names, which line.
#[derive(Debug)]
struct Citation {
    from: String,
    from_line: usize,
    target: String,
    target_line: usize,
}

/// `path/to/file.rs:123`, anywhere in a `//` comment.
fn citations(files: &[PathBuf], root: &Path) -> Vec<Citation> {
    let mut out = Vec::new();
    for f in files {
        // This file's own header quotes citations as examples; scanning it would make the
        // gate report itself.
        if f.file_name().is_some_and(|n| n == SELF) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("//") && !trimmed.starts_with('*') {
                continue;
            }
            for (start, _) in line.match_indices(".rs:") {
                // Walk back over the path, forward over the digits.
                let head = &line[..start];
                let name_start = head
                    .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '/'))
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let rest = &line[start + 4..];
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if digits.is_empty() {
                    continue;
                }
                let target = format!("{}.rs", &line[name_start..start]);
                out.push(Citation {
                    from: f
                        .strip_prefix(root)
                        .unwrap_or(f)
                        .to_string_lossy()
                        .replace('\\', "/"),
                    from_line: i + 1,
                    target,
                    target_line: digits.parse().unwrap(),
                });
            }
        }
    }
    out
}

/// Resolve `foo/bar.rs` against the files we have, preferring the longest suffix match.
fn resolve<'a>(target: &str, files: &'a [PathBuf], root: &Path) -> Option<&'a PathBuf> {
    let want = target.replace('\\', "/");
    files
        .iter()
        .filter(|f| {
            f.strip_prefix(root)
                .unwrap_or(f)
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with(&want)
        })
        .min_by_key(|f| f.to_string_lossy().len())
}

/// **No comment cites a line number.**
///
/// This gate used to ask the weaker question — *of the symbols a citing comment names, does at
/// least one still appear within ±4 lines of the cited number* — and on 2026-08-13 it answered
/// **29 of 37 do not**. Two of those were load-bearing: `Reaped::for_reason`'s doc says
/// *"`grep -rn "Reaped::for_reason"` is the list of places that do not ask"*, and both production
/// entries on that list justified themselves by naming the line where the guard is enforced.
/// Both numbers were wrong — by 214 and 269 lines — and both landed in unrelated code that read
/// plausibly enough to be mistaken for the thing cited. A reviewer auditing the guard was sent to
/// the wrong place, twice.
///
/// **The fix was not to renumber them, so this is not the test that checks they were renumbered.**
/// A line number is the one citation that rots with nobody editing it: the refactor of
/// 2026-08-12 moved 1,800 lines and every number below it shifted. Renumbering buys one green
/// run and the next refactor takes it back. The numbers are gone — 73 of them, across 25 files —
/// and what survives is the symbol name, which a refactor carries with the code it names.
///
/// **Stated as a prohibition on purpose, because the weaker form could go vacuous.** "They all
/// still land" is trivially true of a tree with no line citations left, so a gate phrased that
/// way would stop measuring at the exact moment it started passing, and nothing would say so.
/// A prohibition cannot: its subject is what someone writes tomorrow.
#[test]
fn no_comment_cites_a_line_number() {
    let root = repo_root();
    let files = rust_files(&root);

    let offenders: Vec<String> = citations(&files, &root)
        .into_iter()
        .map(|c| {
            format!(
                "{}:{} cites `{}:{}`",
                c.from, c.from_line, c.target, c.target_line
            )
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "{} comment(s) cite a source line by number:\n  {}\n\n\
         A line number goes stale without anyone editing it — 29 of 37 in this tree had, and two \
         of them sent a reviewer auditing the removal guard into unrelated code that read \
         plausibly enough to be mistaken for it. Name the symbol instead: `plan.rs`'s `apply` \
         survives the refactor that `plan.rs:483` does not.",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// Every `.rs` file a comment names in backticks either exists, or is said to be gone.
///
/// The other half of the same rot, and the half that outlives the line numbers: one comment
/// cited `e2e_tests.rs`, a file this repository deleted. Dropping the number would have hidden
/// that rather than fixed it, so the filename is checked on its own.
///
/// A mention may name a file that no longer exists **when it says so** — `executor.rs` explains
/// the mock's two-ledger design by recounting the bug that a since-deleted test file shipped,
/// and that is history a reader needs rather than a pointer they might follow. The escape is the
/// word, so it has to be written deliberately.
#[test]
fn every_rust_file_a_comment_names_exists_or_is_marked_gone() {
    let root = repo_root();
    let files = rust_files(&root);
    let mut missing: Vec<String> = Vec::new();

    for f in &files {
        if f.file_name().is_some_and(|n| n == SELF) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("//") && !trimmed.starts_with('*') {
                continue;
            }
            for chunk in line.split('`').skip(1).step_by(2) {
                let name = chunk.trim();
                if !name.ends_with(".rs") || name.contains(' ') {
                    continue;
                }
                // A glob is a description of a set, not a citation of a file. `src/**/*.rs` and
                // `*_lock.rs` are how several gates state their own scope.
                if name.contains('*') {
                    continue;
                }
                // `backends/registry.rs` and `backends/registry/mod.rs` are the same module
                // under Rust's two spellings, and a comment naming the first is not stale.
                let as_module = format!("{}/mod.rs", name.trim_end_matches(".rs"));
                if resolve(name, &files, &root).is_some()
                    || resolve(&as_module, &files, &root).is_some()
                {
                    continue;
                }
                if line.contains("since-deleted") || line.contains("no longer in the tree") {
                    continue;
                }
                missing.push(format!(
                    "{}:{} names `{}`, which is not in the tree",
                    f.strip_prefix(&root)
                        .unwrap_or(f)
                        .to_string_lossy()
                        .replace('\\', "/"),
                    i + 1,
                    name
                ));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "{} comment(s) name a Rust file this repository does not have:\n  {}\n\n\
         Either the file was renamed and the comment did not follow, or it was deleted and the \
         comment is history — in which case say `since-deleted` so a reader does not go looking.",
        missing.len(),
        missing.join("\n  ")
    );
}
