//! A `file.rs:NNN` written in a comment must still point at what the comment says is there.
//!
//! Comments in this tree cite each other by line number, and two of those citations are
//! load-bearing: `guard.rs`'s `Reaped::for_reason` doc says *"`grep -rn "Reaped::for_reason"`
//! is the list of places that do not ask"*, and each of the two production entries on that
//! list justifies itself by naming the line where the guard is enforced instead. A reviewer
//! auditing the guard is told exactly where to look. Both numbers are wrong — `transaction.rs`
//! calls `protection_of` at 1207, not 993; `sync/mod.rs` enforces per interrupted entry at
//! 1141, not 872 — and both land in unrelated code that reads plausibly enough to be mistaken
//! for the thing cited.
//!
//! Line numbers are the one kind of citation that goes stale without anybody touching it: the
//! refactor of 2026-08-12 moved 1,800 lines and every number below it shifted. Nothing in the
//! tree notices, which is `GRADER.md` §2.2b's class exactly — a claim with an expiry and no
//! alarm.
//!
//! **What this asserts, and what it cannot.** A line number is checkable; a claim is not. So
//! this checks the weakest thing that still catches the failure: of the symbols a citing
//! comment names, *at least one* must appear within a few lines of the cited number. That is
//! enough to catch every instance found on 2026-08-12 and cheap enough to keep. It says
//! nothing about whether the cited code still *means* what the citing comment says it means.
//!
//! **Deliberately the weak form, because the strong one over-reports.** Requiring *every*
//! named symbol to be near the line flags a comment that mentions two things and cites one of
//! them — which is most comments. The first draft of this file did that and produced 53
//! failures where hand-checking found 15. A screening instrument that cries wolf is deleted,
//! and then the fifteen come back.
//!
//! The fix is not to renumber. A citation that names a symbol — `protection_of` in
//! `transaction.rs` — survives every refactor that a number does not.

use std::collections::BTreeMap;
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
    /// The comment text around it, which is where the symbol name lives.
    context: String,
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
                // Two lines either side is the comment; that is where the symbol is named.
                let lo = i.saturating_sub(3);
                let hi = (i + 3).min(lines.len());
                out.push(Citation {
                    from: f
                        .strip_prefix(root)
                        .unwrap_or(f)
                        .to_string_lossy()
                        .replace('\\', "/"),
                    from_line: i + 1,
                    target,
                    target_line: digits.parse().unwrap(),
                    context: lines[lo..hi].join(" "),
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

/// Identifiers the citing comment mentions, which is what the cited line should contain.
fn named_symbols(context: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = context.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '`' {
            let mut j = i + 1;
            let mut buf = String::new();
            while j < bytes.len() && bytes[j] != '`' {
                buf.push(bytes[j]);
                j += 1;
            }
            // Only bare identifiers: `protection_of`, `is_removal_call`. Prose in backticks
            // and argv fragments are not symbols and would make this a spelling test.
            let ident = buf.trim_end_matches("()");
            if !ident.is_empty()
                && ident.len() > 3
                && ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && ident.chars().any(|c| c == '_' || c.is_ascii_lowercase())
                && !ident.ends_with(".rs")
            {
                out.push(ident.to_string());
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Every citation names a file that exists.
///
/// The cheapest half, and it already fails: `core/executor.rs` cites `e2e_tests.rs:108`, and
/// `tests/e2e_tests.rs` was deleted.
#[test]
fn every_cited_file_exists() {
    let root = repo_root();
    let files = rust_files(&root);
    let dangling: Vec<String> = citations(&files, &root)
        .into_iter()
        .filter(|c| resolve(&c.target, &files, &root).is_none())
        .map(|c| {
            format!(
                "{}:{} cites `{}`, which is not in the tree",
                c.from, c.from_line, c.target
            )
        })
        .collect();

    assert!(
        dangling.is_empty(),
        "{} citation(s) name a file this repository does not have:\n  {}",
        dangling.len(),
        dangling.join("\n  ")
    );
}

/// Every citation that names a symbol lands near that symbol.
///
/// Self-tested by construction: the tolerance is ±4 lines, and the failures below are 25,
/// 198, 214, 269 and 370 lines out, so this is not a test that a formatting change can trip.
#[test]
fn every_citation_that_names_a_symbol_still_lands_on_it() {
    let root = repo_root();
    let files = rust_files(&root);
    const TOLERANCE: usize = 4;

    let mut wrong: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let mut cache: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();

    for c in citations(&files, &root) {
        let Some(target) = resolve(&c.target, &files, &root) else {
            continue; // The other test owns this failure.
        };
        let symbols = named_symbols(&c.context);
        if symbols.is_empty() {
            continue; // Nothing checkable was claimed.
        }
        let lines = cache.entry(target.clone()).or_insert_with(|| {
            std::fs::read_to_string(target)
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect()
        });
        if c.target_line == 0 || c.target_line > lines.len() {
            wrong.push(format!(
                "{}:{} cites {}:{}, past the end of a {}-line file",
                c.from,
                c.from_line,
                c.target,
                c.target_line,
                lines.len()
            ));
            continue;
        }
        let lo = c.target_line.saturating_sub(TOLERANCE + 1);
        let hi = (c.target_line + TOLERANCE).min(lines.len());
        let window = lines[lo..hi].join("\n");

        // Only symbols the target file actually holds: a comment may name something that
        // lives elsewhere entirely, and that is not a claim about this line number.
        let present: Vec<String> = symbols
            .into_iter()
            .filter(|s| lines.iter().any(|l| l.contains(s.as_str())))
            .collect();
        if present.is_empty() {
            continue;
        }
        checked += 1;
        // One hit is enough. See the header: requiring all of them flags every comment that
        // mentions two things and cites one.
        if present.iter().any(|s| window.contains(s.as_str())) {
            continue;
        }
        let where_they_are: Vec<String> = present
            .iter()
            .map(|s| {
                let at: Vec<usize> = lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.contains(s.as_str()))
                    .map(|(i, _)| i + 1)
                    .take(4)
                    .collect();
                format!("`{s}` at {at:?}")
            })
            .collect();
        wrong.push(format!(
            "{}:{} cites {}:{} — none of the symbols it names is within {} lines: {}",
            c.from,
            c.from_line,
            c.target,
            c.target_line,
            TOLERANCE,
            where_they_are.join(", ")
        ));
    }

    assert!(
        wrong.is_empty(),
        "{} of {} checked citation(s) no longer land on the symbol they name (±{} lines):\n  {}\n\n\
         A line number is the one citation that rots without anyone editing it. Name the \
         symbol instead of the line.",
        wrong.len(),
        checked,
        TOLERANCE,
        wrong.join("\n  ")
    );
}
