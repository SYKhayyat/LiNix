//! Does Part II describe the config language this program actually parses?
//!
//! Four statement prefixes have shipped without reaching Part II's own table: `exec:`,
//! `dotfiles:` and `firewall:` were caught after two days and a paragraph was written about
//! them; `generate:` was added in the same era, sat directly under that paragraph, and was
//! still missing on 2026-08-04 — read past by every session that read the warning. **The
//! paragraph is prose, and prose does not fail a build.**
//!
//! Q29 asked whether the fix was to close the language: no more `foo:` prefixes, ever. The
//! owner ruled on 2026-08-04 that it stays open — *"i dont think it is closed, no. we still
//! might add"* — which makes this file the load-bearing half. A language that keeps growing
//! needs a check that the documentation grows with it, and that is cheaper than a ban and
//! costs nothing anybody wanted to keep.
//!
//! **It reads `KEYWORDS` through the parser's own accessors, not by scraping the source.** A
//! regex over `statement.rs` would be a third copy of the list, free to be wrong in the
//! direction that hides a defect — which is this bug one level up, and is the shape the
//! `known_prefixes` accessor already exists to prevent.
//!
//! Both directions fail. A word in the code and not the docs is a statement nobody
//! documented; a word in the docs and not the code is a promise the parser does not keep, and
//! that one sends a reader to write a line that will be refused.

use shall::config::grammar::KeywordRole;
use std::collections::BTreeSet;

use crate::ledger::Ledger;
use std::path::PathBuf;

/// Compile-time, not the working directory: a test that reads `./docs` passes or fails
/// depending on where `cargo test` was invoked from.
///
/// **Line endings are normalised where the text enters the parser**, which is Q22's ruling one
/// file over: a byte-order mark is stripped at the same boundary rather than refused. The
/// scanner below splits on ` ``` ` followed by a newline, a sequence that does not occur in a
/// CRLF file — so on a Windows checkout this whole gate died with *"no fenced block follows
/// ..."*, a message that blames the heading for moving. `.gitattributes` now pins `*.md` to LF,
/// and this line is the half that does not depend on the reader's git config: **a coverage gate
/// that only runs on a correctly-configured clone is a gate that reports on the clone.**
fn spec() -> String {
    let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/spec/target-state.md");
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
        .replace("\r\n", "\n")
}

/// The fenced block that follows `heading` in the spec.
///
/// Panics rather than returning empty when the heading moves: a scan that quietly finds
/// nothing reports "the docs and the code agree" about two empty sets, which is the failure
/// mode every coverage gate in this repo is written to avoid.
fn fenced_block_after(text: &str, heading: &str) -> String {
    let (_, rest) = text
        .split_once(heading)
        .unwrap_or_else(|| panic!("the spec has no heading {heading:?} — it moved or was renamed"));
    let (_, after_open) = rest
        .split_once("```\n")
        .unwrap_or_else(|| panic!("no fenced block follows {heading:?}"));
    let (block, _) = after_open
        .split_once("\n```")
        .unwrap_or_else(|| panic!("the fenced block after {heading:?} is never closed"));
    block.to_string()
}

/// The reserved words the spec lists, grouped the way the spec groups them: each `#` comment
/// line opens a group, and the group's role is read from the comment's first word.
fn documented_words_by_role() -> Vec<(String, KeywordRole)> {
    let block = fenced_block_after(
        &spec(),
        "### A bare word that is a keyword is not a package",
    );
    let mut out = Vec::new();
    let mut role = None;
    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(label) = line.strip_prefix('#') {
            let label = label.trim();
            role = Some(match label.split_whitespace().next() {
                Some("prefixes") => KeywordRole::Prefix,
                Some("directives") => KeywordRole::Directive,
                Some("the") => KeywordRole::Foreign,
                other => panic!(
                    "the reserved-word block has a group labelled {other:?}, which names no \
                     KeywordRole. Label groups `prefixes`, `directives`, or `the words people \
                     arrive with`."
                ),
            });
            continue;
        }
        let role = role.expect(
            "the reserved-word block starts with words before any `#` group label, so nothing \
             says what they are",
        );
        for word in line.split_whitespace() {
            out.push((word.to_string(), role));
        }
    }
    assert!(
        !out.is_empty(),
        "the reserved-word block parsed to nothing — the scan is broken, not the docs"
    );
    out
}

/// The lowercase `word:` prefixes the Statements table demonstrates.
///
/// `BACKEND:NAME` and `NAME` are shapes, not prefixes, and are skipped on case. `list:` and
/// `re:` are the two lowercase prefixes that are deliberately *not* keywords — see
/// `NOT_A_KEYWORD`.
fn documented_statement_prefixes() -> BTreeSet<String> {
    let block = fenced_block_after(&spec(), "### Statements");
    let mut out = BTreeSet::new();
    for line in block.lines() {
        let Some(head) = line.split_whitespace().next() else {
            continue;
        };
        // `BACKEND,list:NAME` — take the prefix nearest the name, which is what the line is
        // demonstrating. `BACKEND:re:PATTERN` yields `re`.
        for segment in head.split(',') {
            let mut parts = segment.split(':').peekable();
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    break; // the payload, not a prefix
                }
                if part.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                    && part.chars().all(|c| c.is_ascii_lowercase())
                {
                    out.insert(part.to_string());
                }
            }
        }
    }
    assert!(
        !out.is_empty(),
        "the Statements table parsed to no prefixes — the scan is broken, not the docs"
    );
    out
}

/// Lowercase `word:` forms in the Statements table that are deliberately not keywords, each
/// with the reason. An unexplained exemption is where coverage goes to disappear (E29).
const NOT_A_KEYWORD: &[(&str, &str)] = &[
    (
        "list",
        "the pseudo-backend meaning `every manager in priority, in order` (II.7b). It occupies \
         a backend's position in the grammar, not a keyword's, which is exactly why `list:link` \
         is the escape hatch for installing a package named after a keyword.",
    ),
    (
        "re",
        "a modifier inside a package line (`apt:re:^lib`), not a statement prefix. It is parsed \
         after the backend has already been resolved, so it never reaches the KEYWORDS dispatch.",
    ),
];

#[test]
fn every_reserved_word_is_documented_with_its_role() {
    let documented: BTreeSet<(String, KeywordRole)> =
        documented_words_by_role().into_iter().collect();
    let in_code: BTreeSet<(String, KeywordRole)> =
        shall::config::grammar::statement::reserved_words()
            .into_iter()
            .map(|(w, r)| (w.to_string(), r))
            .collect();

    let undocumented: Vec<String> = in_code
        .difference(&documented)
        .map(|(w, r)| format!("{w} ({r:?})"))
        .collect();
    assert!(
        undocumented.is_empty(),
        "these words are reserved by the parser and are not in Part II's list, or are listed \
         under the wrong group:\n    {}\n\n\
         Add each to the reserved-word block in docs/spec/target-state.md, under the group \
         matching its KeywordRole. This is the check `generate:` needed and did not have.",
        undocumented.join("\n    ")
    );

    let promised: Vec<String> = documented
        .difference(&in_code)
        .map(|(w, r)| format!("{w} ({r:?})"))
        .collect();
    assert!(
        promised.is_empty(),
        "Part II lists these as reserved words and the parser does not reserve them, or \
         reserves them with another role:\n    {}\n\n\
         Either the parser lost a keyword or the docs kept one past its deletion. The second \
         is worse: it sends a reader to write a line that will be refused.",
        promised.join("\n    ")
    );
}

#[test]
fn every_statement_prefix_has_a_row_in_the_statements_table() {
    let documented = documented_statement_prefixes();
    let in_code: BTreeSet<String> = shall::config::grammar::statement::reserved_words()
        .into_iter()
        .filter(|(_, r)| *r == KeywordRole::Prefix)
        .map(|(w, _)| w.to_string())
        .collect();

    let missing: Vec<&String> = in_code.difference(&documented).collect();
    assert!(
        missing.is_empty(),
        "these statement prefixes parse today and have no row in Part II's Statements \
         table:\n    {missing:?}\n\n\
         Add a row showing the shape and what it declares. `generate:` was missing here for \
         months while its own rule sat forty lines below."
    );

    let unbacked: BTreeSet<String> = documented.difference(&in_code).cloned().collect();
    Ledger::of(
        "shown in the Statements table with no keyword behind it",
        "NOT_A_KEYWORD",
    )
    .pairs(NOT_A_KEYWORD)
    .scanning_at_least(10)
    .remedy("Either add the keyword, or say why the form is not one.")
    .audit(documented.len(), &unbacked);
}

/// A gate that cannot fail is a gate nobody has tested.
///
/// Both scans are string surgery over a document people edit for prose reasons. This drives
/// them over inputs with a known answer, so a reformat that silently empties one of them
/// fails here rather than passing everywhere.
#[test]
fn the_scans_can_actually_fail() {
    let doc = "### Statements\n\n```\nNAME    bare\napt:NAME  pinned\nshim:NAME  a shim\n```\n";
    let block = fenced_block_after(doc, "### Statements");
    assert!(block.contains("shim:NAME"), "the block reader lost a line");

    // Upper-case heads are shapes, not prefixes; lower-case ones are prefixes.
    assert!(!block.contains("BACKEND"), "fixture drifted");

    let missing_close = std::panic::catch_unwind(|| {
        fenced_block_after(
            "### Statements\n\n```\nshim:NAME  a shim\n",
            "### Statements",
        )
    });
    assert!(
        missing_close.is_err(),
        "an unterminated fence must fail loudly, not return the rest of the file"
    );

    let moved_heading =
        std::panic::catch_unwind(|| fenced_block_after("nothing here", "### Statements"));
    assert!(
        moved_heading.is_err(),
        "a heading that moved must fail loudly, not report agreement between two empty sets"
    );
}
