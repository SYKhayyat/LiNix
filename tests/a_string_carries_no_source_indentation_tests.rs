//! A user-facing string does not carry the indentation of the source it was written in.
//!
//! **What was found.** A Rust string literal that spans source lines needs a trailing `\` to
//! swallow the newline *and the next line's indentation*. Thirty-eight literals in this tree
//! were missing it, so what reached the user was a sentence with ten to forty spaces in the
//! middle of it:
//!
//! ```text
//! "a declared port is open - that is what declaring it means. To close one,
//!                   delete the line; `@value=` belongs on `default/incoming` only."
//! ```
//!
//! Several were refusal messages, which are the strings a user is meant to read and act on.
//!
//! **Why a gate and not a sweep.** The adjacent literals in the same functions did it
//! correctly, which is what makes these typos rather than a convention — and a typo class with
//! nothing watching it comes back one literal at a time. The sweep fixed thirty-eight; this is
//! what stops the thirty-ninth.
//!
//! **What is deliberately allowed.** Aligned columns are a real thing to write: `repl.rs`'s
//! `:vars   the variables ...`, the profile template's commented examples, a fixture quoting a
//! manager's tabular output. Two rules keep those out: a run of spaces that begins within the
//! first thirty characters of the line is a column, not a wrapped sentence; and a line carrying
//! an explicit `\n` escape is laying out its own text, where the spaces after a newline are
//! the layout.

use std::path::{Path, PathBuf};

const SELF: &str = "a_string_carries_no_source_indentation_tests.rs";

fn rust_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
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

/// **No user-facing string carries the indentation of the source line below it.**
#[test]
fn a_wrapped_string_literal_does_not_print_its_own_indentation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders: Vec<String> = Vec::new();

    for f in rust_files() {
        if f.file_name().is_some_and(|n| n == SELF) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // A comment's own alignment never reaches a user.
            if trimmed.starts_with("//") || trimmed.starts_with('*') || !line.contains('"') {
                continue;
            }
            // An explicit newline means the spaces after it are the printed layout.
            if line.contains("\\n") {
                continue;
            }
            let start = line.find('"').map(|q| q + 1).unwrap_or(0);
            let body: Vec<char> = line[start..].chars().collect();
            let mut run = 0usize;
            for (j, c) in body.iter().enumerate() {
                if *c == ' ' {
                    run += 1;
                    continue;
                }
                let column = j - run;
                // A run of spaces that starts inside the first thirty characters is a column.
                if run >= 4 && column > 30 && (c.is_ascii_alphabetic() || *c == '`') {
                    offenders.push(format!(
                        "{}:{}",
                        f.strip_prefix(root)
                            .unwrap_or(&f)
                            .to_string_lossy()
                            .replace('\\', "/"),
                        i + 1
                    ));
                    break;
                }
                run = 0;
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "{} string literal(s) print a run of spaces mid-sentence, which is the indentation \
         of the source line they were wrapped onto:\n  {}\n\nA multi-line Rust string \
         needs a trailing backslash to swallow the newline and the leading whitespace \
         after it.",
        offenders.len(),
        offenders.join("\n  ")
    );
}
