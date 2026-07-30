//! GRADER round 5, 2026-07-30 — RED. A grammar keyword written on a line by itself is read as a
//! package name, resolved against a real package index, and queued for install.
//!
//! Measured end to end, release binary, a module containing the single word `link` — which is
//! what a half-typed `link:SRC @target=DEST` line looks like:
//!
//!     $ linix eval
//!       "present": [ { "backend": "cargo", "name": "link", "source": "modules/kw.txt:1" } ]
//!     $ linix --dry-run sync -y
//!       install 1   remove 0   (total 1 change(s))     backends: cargo
//!     $ linix check
//!       ->  drift   1 to install …  run `linix sync`
//!
//! Thirteen of fourteen keywords behave this way and each resolves to a real backend holding a
//! real package of that name — the resolver searched live indexes to produce these:
//!
//!     when -> cargo:when      absent -> pip:absent    link  -> cargo:link    service -> cargo:service
//!     setting -> cargo:setting  shim -> scoop:shim    schedule -> cargo:schedule  repo -> cargo:repo
//!     if -> gem:if            else -> npm:else        end -> cargo:end       import -> gem:import
//!     include -> cargo:include                        use  -> refused (the only one)
//!
//! **This is not a broken parser.** A package name is one bare word (II.2), so a bare keyword is a
//! grammatically valid package line. Written with their punctuation the same words refuse
//! correctly and legibly — `link:`, `service:`, `shim:`, `when linux`, `when linux {` all exit 1
//! with a located `Configuration error`. The ambiguity is confined to the bare word, and what to
//! do about it — reserve them, warn on them, require a backend prefix on a colliding name — is a
//! language decision that belongs in `decisions.md`.
//!
//! The test is written at the grammar rather than through the binary on purpose: resolving one of
//! these costs 10–27 seconds, because a bare name has no backend and the resolver asks every
//! manager in priority order. The same fixture with `cargo:ripgrep` takes 0.2s.
//!
//! The keyword list is read from `known_prefixes()` rather than copied, so a prefix added later
//! is covered without anyone remembering to add it here.

use linix::config::grammar::statement::{known_prefixes, parse, Statement};
use linix::config::grammar::Origin;

/// Stands in for the live registry: the backends a fixture would have.
fn known(name: &str) -> bool {
    matches!(name, "apt" | "cargo" | "npm" | "gem" | "pip" | "scoop")
}

fn at(line: usize) -> Origin {
    Origin::new("modules/kw.txt", line)
}

/// Every `X:` prefix the grammar knows, written as the bare word `X` — the shape a line has when
/// someone typed the keyword and stopped before the colon.
#[test]
fn no_resource_keyword_is_read_as_a_package_name() {
    let mut wrong = Vec::new();

    for prefix in known_prefixes() {
        let bare = prefix.trim_end_matches(':');
        if let Ok(Statement::Package(p)) = parse(&at(1), bare, &known) {
            wrong.push(format!(
                "`{bare}` parsed as a package named `{}` — so `sync` will install it",
                p.selector.as_str()
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "{} of {} grammar prefixes are read as package names when written without their colon:\n  \
         {}\n\nA user who types `link` and stops has declared a package. `linix check` then \
         reports `1 to install` and tells them to run `linix sync`.",
        wrong.len(),
        known_prefixes().len(),
        wrong.join("\n  ")
    );
}

/// The control, and it is the reason the test above measures what it says. With the colon the
/// grammar refuses — so a green result above would mean the bare form is handled too, not that
/// the parser rejects everything that looks like a keyword.
#[test]
fn the_same_keyword_with_its_colon_is_still_refused() {
    let mut accepted_as_package = Vec::new();

    for prefix in known_prefixes() {
        // `link:` — the prefix with nothing after it.
        if let Ok(Statement::Package(p)) = parse(&at(1), prefix, &known) {
            accepted_as_package.push(format!("`{prefix}` -> package `{}`", p.selector.as_str()));
        }
    }

    assert!(
        accepted_as_package.is_empty(),
        "a prefix with an empty argument was read as a package: {}",
        accepted_as_package.join(", ")
    );
}

/// The control-flow words, which are not in `known_prefixes()` because they introduce a block
/// rather than a resource — and which behave identically. Measured: `when` -> `cargo:when`,
/// `if` -> `gem:if`, `else` -> `npm:else`, `end` -> `cargo:end`.
#[test]
fn no_control_keyword_is_read_as_a_package_name() {
    let mut wrong = Vec::new();

    for word in ["when", "if", "else", "end", "import", "include"] {
        if let Ok(Statement::Package(p)) = parse(&at(1), word, &known) {
            wrong.push(format!("`{word}` -> package `{}`", p.selector.as_str()));
        }
    }

    assert!(
        wrong.is_empty(),
        "control keywords read as package names: {}\n\nA module containing only the word `when` \
         resolves to `cargo:when` and `linix check` recommends the sync that installs it.",
        wrong.join(", ")
    );
}

// ---------------------------------------------------------------------------------------
// BUILDER round 6 — the family the three tests above do not reach.
//
// The grader's tests drive off `known_prefixes()` and a list of six control words. Between
// them they missed the directives (`exclude`, `intersect`, `module`), the half-typed line
// that still carries its options, and — the half that matters most — the escape hatch the
// ruling promised: `list:NAME` and `BACKEND:NAME` must still declare a package by any of
// these names, or the refusal has removed a feature rather than added a check.
// ---------------------------------------------------------------------------------------

/// Every word the grammar reserves, from the parser's own table rather than from a list
/// written here — including the directives, which are in neither of the grader's two lists
/// because they are neither a `X:` prefix nor a control-flow word.
const EVERY_KEYWORD: &[&str] = &[
    "absent",
    "repo",
    "shim",
    "schedule",
    "service",
    "link",
    "setting",
    "exec",
    "generate",
    "dotfiles",
    "firewall",
    "use",
    "param",
    "exclude",
    "intersect",
    "module",
    "when",
    "if",
    "else",
    "end",
    "import",
    "include",
];

/// The list above is a copy, so it is checked against the parser rather than trusted: every
/// `X:` prefix the parser knows must appear in it. A prefix added later fails here.
#[test]
fn the_keyword_list_in_this_file_covers_every_prefix_the_parser_has() {
    let missing: Vec<_> = known_prefixes()
        .iter()
        .map(|p| p.trim_end_matches(':'))
        .filter(|w| !EVERY_KEYWORD.contains(w))
        .collect();
    assert!(
        missing.is_empty(),
        "the parser grew prefixes this test does not cover: {:?}",
        missing
    );
}

#[test]
fn every_keyword_refuses_as_a_bare_word() {
    let mut accepted = Vec::new();
    for word in EVERY_KEYWORD {
        if let Ok(Statement::Package(p)) = parse(&at(1), word, &known) {
            accepted.push(format!("`{word}` -> package `{}`", p.selector.as_str()));
        }
    }
    assert!(
        accepted.is_empty(),
        "bare keywords still read as package names: {}",
        accepted.join(", ")
    );
}

/// The escape hatch. `list:NAME` is what a bare `NAME` was already short for (II.2), so the
/// ruling took nothing away — and this is the assertion that proves it, because a refusal
/// that also swallowed `list:link` would pass every test above.
#[test]
fn a_keyword_is_still_declarable_as_a_package_when_it_is_spelled_out() {
    for word in EVERY_KEYWORD {
        for line in [format!("list:{word}"), format!("cargo:{word}")] {
            match parse(&at(1), &line, &known) {
                Ok(Statement::Package(p)) => assert_eq!(
                    p.selector.as_str(),
                    *word,
                    "`{line}` declared the wrong name"
                ),
                other => panic!("`{line}` no longer declares a package: {other:?}"),
            }
        }
    }
}

/// The same typo with the rest of the line still attached — `link @target=…` is what you get
/// when you write the whole statement and drop one colon, and it is the likelier half of the
/// family, not a rarer one.
#[test]
fn a_keyword_carrying_its_options_is_refused_too() {
    for line in [
        "link @target=/etc/vimrc",
        "service @state=running",
        "shim @target=/usr/bin/rg",
    ] {
        let err = parse(&at(4), line, &known)
            .err()
            .unwrap_or_else(|| panic!("`{line}` was accepted"));
        assert!(
            err.what.contains("is a keyword"),
            "`{line}` refused for the wrong reason: {err}"
        );
    }
}

/// A refusal nobody can act on is the class this repo keeps re-finding. It must name the
/// file, the line, and both ways to mean the word.
#[test]
fn the_refusal_names_the_line_and_both_ways_to_mean_the_word() {
    let err = parse(&at(4), "link", &known).unwrap_err();
    let rendered = err.to_string();
    for needle in [
        "modules/kw.txt:4",
        "`link` is a keyword, not a package name",
        "link:/path/to/source",
        "list:link",
        "cargo:link",
    ] {
        assert!(
            rendered.contains(needle),
            "the refusal does not say `{needle}`:\n{rendered}"
        );
    }
}

/// A name that merely begins with a keyword is a name. `linker` is a real package and the
/// refusal must not reach it — the check binds the whole word, not a prefix of it.
#[test]
fn a_name_that_only_starts_with_a_keyword_is_still_a_package() {
    for word in ["linker", "services", "ending", "iffy", "usejs", "repos"] {
        match parse(&at(1), word, &known) {
            Ok(Statement::Package(p)) => assert_eq!(p.selector.as_str(), word),
            other => panic!("`{word}` is a package name and was refused: {other:?}"),
        }
    }
}
