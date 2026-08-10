//! **The register owns its ID letters, and no other document may define one.**
//!
//! `CLAUDE.md` reserves `D*`, `W*`, `K*`, `N*`, `T*`, `U*` for register entries: an ID with one
//! of those prefixes is a question whose answer is the owner's, so a builder who meets one stops
//! and asks. `docs/BUILDER.md` then numbered its own **work orders** — things to build without
//! asking — `W1` through `W43`. The two series were disjoint in subject and identical in
//! spelling: the register's `W9` is *"interpolation outside `when`"*, BUILDER's was *"run the
//! native sweep in CI"*. A builder handed "W9" could not tell from the ID whether to write code
//! or stop, and neither could a reviewer checking whether they had.
//!
//! **This happened twice, and the first time it was caught by hand.**
//! `PRODUCTION-READINESS-REVIEW.md` numbered its findings `U1`–`U3`, noticed the collision,
//! renumbered to `PR*`, and wrote a note at the top explaining the danger. `BUILDER.md` did it
//! anyway, forty-three times, in the same repository, after that note existed. Prose warning the
//! next author is not a mechanism; this is.

use std::path::{Path, PathBuf};

/// The prefixes `CLAUDE.md` reserves for the register.
const RESERVED: [&str; 6] = ["D", "W", "K", "N", "T", "U"];

/// The specification directory, which **is** the register and the documents that annotate it.
///
/// Not an exemption list — `SPEC.md` defines `docs/spec/` as the specification: `decisions.md`
/// is the register, and `bugs.md`, `history.md`, `why.md` and `target-state.md` exist to say
/// what each entry means, what it broke and how far it got. Those name register IDs constantly
/// and correctly; `bugs.md`'s `T1` row *is* the register's `T1`. Everything outside this
/// directory — a builder's brief, a grade round, a session log — is a separate work product,
/// and when one of those numbers its own findings it must do so in its own prefix.
const THE_REGISTER_AND_ITS_ANNOTATIONS: &str = "spec";

fn docs_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("docs")
}

fn markdown_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&docs_root(), &mut out);
    out.sort();
    out
}

/// Whether a line **defines** an ID rather than merely citing one.
///
/// Citing is the normal, correct thing every document does — "ruled as `Q16`", "see `W9`" — and
/// a check that forbade it would forbid the register being referred to at all. A *definition* is
/// a heading that opens with the ID, or a table row whose first cell is the ID in bold. Those
/// are the two shapes both colliding documents actually used.
fn defines_id(line: &str) -> Option<String> {
    let t = line.trim();

    let candidate = if let Some(rest) = t.strip_prefix('#') {
        rest.trim_start_matches('#').trim_start()
    } else {
        // A struck-out row still defines the id it strikes out — `~~` is how the register
        // retires one, and a retired id is exactly the kind that must not be reissued.
        t.strip_prefix("| **")
            .or_else(|| t.strip_prefix("| ~~**"))?
    };

    // `W9`, `W9a`, `W9 —`, `W9**`, `W9 ·` … take the leading identifier and stop.
    let id: String = candidate
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    let letters: String = id.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits: String = id
        .chars()
        .skip(letters.len())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() || letters.is_empty() {
        return None;
    }
    RESERVED.contains(&letters.as_str()).then_some(id)
}

#[test]
fn only_the_register_defines_a_register_id() {
    let mut offences: Vec<String> = Vec::new();
    let mut scanned = 0usize;

    for path in markdown_files() {
        if path
            .components()
            .any(|c| c.as_os_str() == THE_REGISTER_AND_ITS_ANNOTATIONS)
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        scanned += 1;
        for (n, line) in text.lines().enumerate() {
            if let Some(id) = defines_id(line) {
                offences.push(format!(
                    "{}:{} defines `{}`",
                    path.strip_prefix(docs_root().parent().unwrap())
                        .unwrap_or(&path)
                        .display(),
                    n + 1,
                    id
                ));
            }
        }
    }

    // The instrument before the verdict: a scan over no files, or one whose matcher has stopped
    // matching, reports a clean tree and is indistinguishable from a clean tree.
    assert!(
        scanned >= 4,
        "the scan read {} documents, which is not the docs tree",
        scanned
    );
    assert!(
        defines_id("### W9 — run the native sweep in CI").is_some(),
        "the matcher no longer recognises the heading shape that caused this"
    );
    assert!(
        defines_id("| **W29** (coverage ratchet threshold) | … |").is_some(),
        "the matcher no longer recognises the table shape that caused this"
    );
    assert!(
        defines_id("Ruled as `Q12`; see `W9` and `U1`.").is_none(),
        "citing a register ID is what every document correctly does; only defining one is the \
         defect, and a check that cannot tell them apart is a check nobody can keep"
    );
    assert!(
        defines_id("### B9 — run the native sweep in CI").is_none(),
        "`B` is not a register prefix and a work order numbered in it is the fix, not a finding"
    );

    assert!(
        offences.is_empty(),
        "these documents mint IDs the decision register owns, so a reader cannot tell a thing \
         to build from a question to ask:\n  {}",
        offences.join("\n  ")
    );
}

/// **And the register may not define one twice.**
///
/// The sibling of the check above, and the one that actually fired: on 2026-08-07 four sessions
/// each took "the next free Y", and the register came out of the day with two entries called
/// `Y19` and two called `Y20` — a parser question and a doc-corpus cut under one ID, a firewall
/// count and a flatpak key under the other. `scripts/decision-count.sh` counts headings, so the
/// total was right and the collision was invisible to it. An owner asked to rule on `Y20` had
/// two decisions to choose from and no way to know it.
///
/// The check above asks *who may mint an ID*; this one asks *whether an ID names one thing*.
/// Neither implies the other, which is why the first one passed all day.
#[test]
fn the_register_gives_each_id_to_exactly_one_entry() {
    let register = docs_root().join("spec").join("decisions.md");
    let text = std::fs::read_to_string(&register).expect("the register is readable");

    let mut seen: Vec<(String, usize)> = Vec::new();
    let mut duplicates: Vec<String> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let Some(id) = line.strip_prefix("## ").map(str::trim) else {
            continue;
        };
        // An entry heading is the bare ID. `## Index` and the like are not entries.
        let letters = id.chars().take_while(char::is_ascii_alphabetic).count();
        if letters == 0 || letters == id.len() || !id[letters..].chars().all(|c| c.is_ascii_digit())
        {
            continue;
        }
        if let Some((_, first)) = seen.iter().find(|(other, _)| other == id) {
            duplicates.push(format!(
                "`{id}` is defined at line {first} and again at {}",
                n + 1
            ));
        }
        seen.push((id.to_string(), n + 1));
    }

    assert!(
        seen.len() > 100,
        "the heading matcher found only {} entries, which is not this register",
        seen.len()
    );
    assert!(
        duplicates.is_empty(),
        "one ID, two decisions — an owner asked to rule on it cannot tell which:\n  {}",
        duplicates.join("\n  ")
    );
}
