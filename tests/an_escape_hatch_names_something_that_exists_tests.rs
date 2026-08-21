//! An escape hatch that names a mechanism must name one that exists.
//!
//! `Reaped::for_reason` is the removal guard's escape hatch: it mints the token that says a
//! removal was authorised, for the paths that enforce the guard somewhere other than the plan.
//! Its own doc tells a reviewer that grepping for it "is exactly the list a reviewer wants", and
//! every call site carries a `_why` string saying where the enforcement really happens.
//!
//! **On 2026-08-21 one of those strings named a function that did not exist.** `heal`'s escape
//! hatch was justified by "each interrupted removal is enforced individually in
//! `heal_interrupted_removals`", and two ledger entries in `removal_guard_enumeration_tests`
//! cited the same name. Grep found it in exactly those three strings and nowhere else. The
//! mechanism was real and correct - inline in `heal`'s own loop - so nothing was unsafe. The
//! POINTER was fiction, and `_why` is `#[allow(dead_code)]` prose, so nothing could catch it.
//!
//! That is `S24`'s lesson arriving as a name instead of as a branch: a claim checked against
//! seven paths and not the eighth reads exactly like a claim that is true.
//!
//! **Deliberately narrow.** A scan of every backticked `snake_case` word in the tree returns 765
//! of them and 90 that resolve to nothing - test file names, TOML keys, clap and clippy
//! attributes. A gate at that precision is a gate somebody switches off, which this repository
//! says out loud elsewhere. The escape-hatch reasons are 32 sites citing 2 identifiers, and every
//! one of them is a promise about where a removal is guarded.

use std::path::{Path, PathBuf};

fn rust_sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }
    let mut paths = Vec::new();
    walk(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut paths,
    );
    paths.sort();
    paths
        .into_iter()
        .filter_map(|p| {
            let body = std::fs::read_to_string(&p).ok()?;
            Some((p.display().to_string(), body.replace("\r\n", "\n")))
        })
        .collect()
}

/// Everything between `for_reason(` and its matching `)`, quotes respected so a `)` inside the
/// reason text does not end the span early.
fn escape_hatch_arguments(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = body[from..].find("for_reason(") {
        let start = from + at + "for_reason(".len();
        let mut depth = 1usize;
        let mut in_str = false;
        let mut escaped = false;
        let mut end = start;
        for (i, c) in body[start..].char_indices() {
            end = start + i;
            if escaped {
                escaped = false;
                continue;
            }
            match c {
                '\\' if in_str => escaped = true,
                '"' => in_str = !in_str,
                '(' if !in_str => depth += 1,
                ')' if !in_str => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push(body[start..end].to_string());
        from = end.max(start + 1);
    }
    out
}

/// The `` `snake_case` `` words inside the string literals of one call's arguments.
fn cited_identifiers(arg: &str) -> Vec<String> {
    let mut out = Vec::new();
    for chunk in arg.split('`').skip(1).step_by(2) {
        let t = chunk.trim();
        let looks_like_an_identifier = t.contains('_')
            && t.starts_with(|c: char| c.is_ascii_lowercase())
            && t.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if looks_like_an_identifier {
            out.push(t.to_string());
        }
    }
    out
}

/// The source with every string literal blanked, so "appears in the tree" means appears as CODE.
/// A name that lives only inside the reason strings citing it is exactly the failure this file
/// exists to catch, and it would resolve against an unfiltered scan of the same text.
fn code_without_string_literals(sources: &[(String, String)]) -> String {
    let mut out = String::new();
    for (_, body) in sources {
        let mut in_str = false;
        let mut escaped = false;
        for c in body.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            match c {
                '\\' if in_str => escaped = true,
                '"' => in_str = !in_str,
                _ if in_str => {}
                _ => out.push(c),
            }
        }
        out.push('\n');
    }
    out
}

fn names_in(code: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let mut word = String::new();
    for c in code.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
        } else if !word.is_empty() {
            out.insert(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        out.insert(word);
    }
    out
}

/// The self-test. Every assertion below is over what the scan found; a scan that found nothing
/// would pass by finding nothing, which is the shape of every finding in this repository.
#[test]
fn the_scan_still_finds_the_escape_hatch_it_is_about() {
    let sources = rust_sources();
    let calls: usize = sources
        .iter()
        .map(|(_, b)| escape_hatch_arguments(b).len())
        .sum();
    assert!(
        calls >= 20,
        "found {calls} `Reaped::for_reason` call sites; there were 32 on 2026-08-21. Either the \
         escape hatch was renamed and this gate now watches nothing, or the parser stopped \
         matching it."
    );
    let cited: usize = sources
        .iter()
        .flat_map(|(_, b)| escape_hatch_arguments(b))
        .map(|a| cited_identifiers(&a).len())
        .sum();
    assert!(
        cited >= 1,
        "no escape-hatch reason names a mechanism at all, so the check below examines nothing"
    );
}

/// The gate itself.
#[test]
fn every_mechanism_an_escape_hatch_names_exists_in_the_source() {
    let sources = rust_sources();
    let known = names_in(&code_without_string_literals(&sources));

    let mut fiction: Vec<(String, String)> = Vec::new();
    for (path, body) in &sources {
        for arg in escape_hatch_arguments(body) {
            for name in cited_identifiers(&arg) {
                if !known.contains(&name) {
                    fiction.push((path.clone(), name));
                }
            }
        }
    }

    assert!(
        fiction.is_empty(),
        "an escape hatch from the removal guard names a mechanism that exists only in the \
         sentence naming it:\n{}\n\n`Reaped::for_reason` tells a reviewer that grepping for it \
         is exactly the list they want. A reviewer who follows one of these arrives nowhere, and \
         `_why` is `#[allow(dead_code)]` prose that nothing else checks. Extract the mechanism \
         into a function with this name, or cite the one that really enforces it.",
        fiction
            .iter()
            .map(|(p, n)| format!("  `{n}` cited in {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
