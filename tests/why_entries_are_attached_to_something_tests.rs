//! `why.md` is a mandatory gate, and half of it was rationale for nothing.
//!
//! `CLAUDE.md` states the rule: *"Every rule in `spec/target-state.md` has a matching entry in
//! `spec/why.md` explaining the bug it exists to prevent; do not change a target-state rule
//! without reading its why entry first."* That makes `why.md` the only document in this repo a
//! contributor is **required** to read before touching Part II.
//!
//! Measured on 2026-08-07: **155 `V.n` entries, 77 cited by a Part II rule, 46 cited from `src/`
//! or `tests/`, and 53 cited by nothing at all.** A third of a mandatory gate explains no rule
//! and enforces no check. That is not a size problem — it is the same drift the register's own
//! index had, in the file whose job is to stop drift.
//!
//! Two questions, and they are not the same question.
//!
//! **Does every citation resolve?** This is the one that protects `CLAUDE.md`'s rule. A Part II
//! rule ending `**V.141.**` sends the reader to an entry that has to be there, or the mandatory
//! read is a dead link and the rule can be changed with its reason unread. It passes today at
//! zero failures in *both* directions — Part II and the code — and it is a hard assertion for
//! that reason: it costs nothing until someone renumbers an entry.
//!
//! **Is every entry attached to something?** A ratchet, not an assertion, because the answer
//! today is 53 and the resolution is work rather than a typo. An uncited entry has three honest
//! ends: cite it from the rule it explains, move it to the doc comment of the test that enforces
//! it — a rationale attached to a check cannot go stale, which is the best pattern in this
//! corpus — or, if it explains nothing that still exists, it is the owner's to retire.
//!
//! **`UNCITED_CEILING` may only go down.** Nothing here deletes prose; the gate is that the pile
//! of unattached rationale cannot grow while nobody is looking, which is exactly how it got to 53.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Entries in `why.md` that no Part II rule and no line of `src/`or `tests/` cites.
///
/// **May only go DOWN.** Raising it is the drift this file exists to catch, happening.
const UNCITED_CEILING: usize = 52;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// The `V.n` labels `why.md` defines. An entry opens a line as `**V.42 — …**` or `**V.141.** …`,
/// so the terminator is a space, a full stop or an em dash — matching only one of those is how a
/// first draft of this test reported `V.141` as a dangling citation when the entry was there.
fn defined() -> BTreeSet<String> {
    let why = read("docs/spec/why.md");
    let mut out = BTreeSet::new();
    for line in why.lines() {
        let Some(rest) = line.strip_prefix("**V.") else {
            continue;
        };
        let label: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
            .collect();
        if label.is_empty() || !label.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let after = rest[label.len()..].chars().next().unwrap_or(' ');
        if after == ' ' || after == '.' || after == '—' {
            out.insert(format!("V.{label}"));
        }
    }
    out
}

/// Every `V.n` mentioned in `text`, whether or not it resolves.
fn cited_in(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == 'V' && bytes[i + 1] == '.' && bytes[i + 2].is_ascii_digit() {
            // Not a citation when it is part of a longer word (`IV.2`, `revV.3`).
            let boundary = i == 0 || !bytes[i - 1].is_alphanumeric();
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            // One optional letter suffix: `V.7b`, `V.115a`.
            if j < bytes.len() && bytes[j].is_ascii_lowercase() {
                let next = bytes.get(j + 1).copied().unwrap_or(' ');
                if !next.is_alphanumeric() {
                    j += 1;
                }
            }
            if boundary {
                out.insert(bytes[i..j].iter().collect::<String>());
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
}

/// Every `V.n` cited from Rust — a rule's rationale attached to the check that enforces it.
fn cited_in_code() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&repo().join("src"), &mut files);
    rust_files(&repo().join("tests"), &mut files);
    assert!(
        files.len() > 100,
        "the scan found {} Rust files, which means it is not scanning",
        files.len()
    );
    let mut out = BTreeSet::new();
    for f in files {
        if let Ok(text) = std::fs::read_to_string(&f) {
            out.extend(cited_in(&text));
        }
    }
    out
}

/// **The half that protects `CLAUDE.md`'s mandatory read.** A Part II rule that cites `V.141`
/// promises the reader an entry to read before changing it; if the entry is not there, the rule
/// can be changed with its reason unread and nothing says so.
///
/// Both directions, because a citation in a test doc comment is the same promise: `Q40`'s
/// rationale quoted in the test that enforces it is worth exactly as much as the entry it points
/// at still existing.
#[test]
fn every_citation_of_a_why_entry_resolves_to_one() {
    let defined = defined();
    assert!(
        defined.len() > 100,
        "why.md yielded only {} entries — the parser is broken, not the file",
        defined.len()
    );

    let from_part_ii = cited_in(&read("docs/spec/target-state.md"));
    let dangling: Vec<&String> = from_part_ii.difference(&defined).collect();
    assert!(
        dangling.is_empty(),
        "these Part II rules cite a `why.md` entry that does not exist: {dangling:?}\n\n\
         CLAUDE.md requires reading the entry before changing the rule. A citation that resolves \
         to nothing makes that read impossible, so either the rule's number is wrong or the entry \
         was renumbered out from under it."
    );

    let from_code = cited_in_code();
    let dangling: Vec<&String> = from_code.difference(&defined).collect();
    assert!(
        dangling.is_empty(),
        "these `src/` or `tests/` comments cite a `why.md` entry that does not exist: \
         {dangling:?}\n\nA rationale attached to a check is the best pattern in this corpus, and \
         it only works while the thing it points at is there."
    );
}

/// **The ratchet.** 53 entries explain no Part II rule and are quoted by no check — a third of a
/// document `CLAUDE.md` makes mandatory reading. The number may fall, never rise.
#[test]
fn the_pile_of_rationale_attached_to_nothing_does_not_grow() {
    let defined = defined();
    let cited: BTreeSet<String> = cited_in(&read("docs/spec/target-state.md"))
        .union(&cited_in_code())
        .cloned()
        .collect();

    let uncited: Vec<&String> = defined.difference(&cited).collect();

    assert!(
        uncited.len() <= UNCITED_CEILING,
        "{} `why.md` entries are cited by no Part II rule and no line of Rust, and the ceiling is \
         {UNCITED_CEILING}. It may only go down.\n\nAn entry has three honest ends: cite it from \
         the rule it explains, move it to the doc comment of the test that enforces it, or — if \
         it explains nothing that still exists — the owner retires it. Adding another unattached \
         one is not among them.\n\nUncited: {uncited:?}",
        uncited.len()
    );

    // The other half, and the half that rots quietly: a ceiling nobody lowers as the work lands
    // is a ceiling that stops meaning anything. Same rule as `NOWHERE_CEILING`.
    assert!(
        uncited.len() >= UNCITED_CEILING,
        "{} entries are uncited and UNCITED_CEILING still says {UNCITED_CEILING} — lower it to \
         {} in this change, so the next one cannot spend the slack you just earned.",
        uncited.len(),
        uncited.len()
    );
}
