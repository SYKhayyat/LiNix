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
