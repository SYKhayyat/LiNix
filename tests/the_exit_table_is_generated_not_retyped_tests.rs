//! The exit-code table a reader sees must be the one the binary returns.
//!
//! **What this file was written about.** `exit.rs` used to carry `Exit::table()`, whose doc said
//! it existed *"so the documentation cannot drift from what the binary returns"*. Three things
//! were true of it at once, and together they let a function, its test and its documentation all
//! be wrong while every gate stayed green:
//!
//! 1. **It had no consumer.** It was called from exactly one place in the tree — `exit.rs`'s own
//!    `#[cfg(test)]` module. `shall --help` printed no exit-code information, and no readme or
//!    `docs/` file carried the generated text. The function written so documentation could not
//!    drift was not what any documentation was made from.
//! 2. **Its one test was a tautology.** `every_code_is_distinct_and_documented` asserted
//!    `table.contains(e.meaning())`, and `table()` was *built by calling* `meaning()`, so both
//!    sides came from one source. Measured, not reasoned: `cargo mutants --file src/core/exit.rs`
//!    replaced every meaning with `"xyzzy"`, and with `""`, and both mutants survived — as did
//!    both mutations of `table()`. Four mutants, four MISSED. The two mutations of `code()` were
//!    caught, which is the control: the codes were genuinely guarded and only the prose was not.
//! 3. **So the hand-written copy drifted**, in the same file, twenty lines away: row 2 of the
//!    module doc-comment read *"a read-only command that looked and found work to do"* where
//!    `meaning()` returned *"something needs you"*.
//!
//! That is the tree's recurring shape — an instrument reading a transcription of its subject —
//! reached from a new direction. `a_pub_fn_nobody_calls_is_dead_tests` catches a `pub fn` nobody
//! calls, and `Exit::table()` passed it because its own unit test called it. That gate is not at
//! fault: its header states the bar and argues it — *"zero references, not zero production
//! callers … policing those would make this a gate people delete"*. What nothing covered was the
//! **intersection**: a function whose only caller is a test *and* whose test cannot fail.
//!
//! **What was done about it.** `Exit::table()` is deleted — a generator with no consumer is the
//! second implementation, not the cure — row 2 is corrected, and the unit test now names the
//! four strings outright so a mutation of `meaning()` fails it. This file is what binds the two
//! copies that remain: the table a human reads, and what a script is handed. It demands no
//! design of either — correcting the doc, or deleting it, satisfies it — and it deliberately
//! does **not** assert that `--help` print the table, which would answer a question nobody asked.

use std::path::{Path, PathBuf};

fn exit_rs() -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/exit.rs");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The `| n | text |` rows of the markdown table in the module doc-comment.
fn rows_of_the_hand_written_table(src: &str) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.trim().strip_prefix("//!") else {
            // The table is in the module doc-comment; a `|` further down the file is code.
            if !out.is_empty() {
                break;
            }
            continue;
        };
        let cells: Vec<&str> = rest.trim().split('|').map(str::trim).collect();
        // `| 0 | converged — … |` splits to ["", "0", "converged — …", ""].
        if cells.len() != 4 {
            continue;
        }
        let Ok(code) = cells[1].parse::<i32>() else {
            continue;
        };
        if !cells[2].is_empty() {
            out.push((code, cells[2].to_string()));
        }
    }
    out
}

/// What `meaning()` actually returns, read from its `match` arms.
fn arms_of_meaning(src: &str) -> Vec<(String, String)> {
    let start = src
        .find("pub fn meaning(self) -> &'static str {")
        .expect("meaning() is not in exit.rs, or its signature changed");
    let body = &src[start..];
    let end = body.find("\n    }").expect("unterminated meaning()");
    let mut out = Vec::new();
    for line in body[..end].lines() {
        let t = line.trim();
        let Some((variant, rest)) = t.split_once(" => ") else {
            continue;
        };
        let Some(variant) = variant.trim().strip_prefix("Exit::") else {
            continue;
        };
        let text = rest.trim().trim_end_matches(',').trim_matches('"');
        if !text.is_empty() {
            out.push((variant.to_string(), text.to_string()));
        }
    }
    out
}

/// The self-test. A scan that found no rows, or no arms, would make the gate below vacuous —
/// and a vacuous gate over documentation drift is the exact thing this file is a finding about.
#[test]
fn the_scan_finds_both_copies_of_the_table() {
    let src = exit_rs();
    let rows = rows_of_the_hand_written_table(&src);
    let arms = arms_of_meaning(&src);
    assert_eq!(
        rows.len(),
        4,
        "the doc-comment table did not yield four rows; the reader has lost it: {rows:?}"
    );
    assert_eq!(
        arms.len(),
        4,
        "meaning() did not yield four arms; the reader has lost it: {arms:?}"
    );
    // And the two agree on at least one row, so a total mismatch means drift rather than a
    // scanner that is reading two unrelated things.
    let texts: Vec<&String> = arms.iter().map(|(_, t)| t).collect();
    assert!(
        rows.iter().any(|(_, t)| texts.contains(&t)),
        "no row of the doc table matches any arm of meaning(); these are not the same table.\n\
         rows: {rows:?}\narms: {arms:?}"
    );
}

/// Every row of the hand-written table is a string `meaning()` actually returns.
#[test]
fn the_hand_written_table_says_what_the_binary_returns() {
    let src = exit_rs();
    let arms = arms_of_meaning(&src);
    let texts: Vec<&String> = arms.iter().map(|(_, t)| t).collect();

    let drifted: Vec<String> = rows_of_the_hand_written_table(&src)
        .into_iter()
        .filter(|(_, text)| !texts.contains(&text))
        .map(|(code, text)| format!("code {code}: doc says {text:?}"))
        .collect();

    assert!(
        drifted.is_empty(),
        "{} row(s) of the exit-code table in `exit.rs`'s module doc say something the binary \
         does not return:\n  {}\nmeaning() returns:\n  {}\n\n\
         That table is what a reader is told the exit codes mean, and `meaning()` is what a \
         script actually gets. Nothing generates one from the other — this test is what binds \
         them, so correct whichever of the two is wrong.",
        drifted.len(),
        drifted.join("\n  "),
        texts
            .iter()
            .map(|t| format!("{t:?}"))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
