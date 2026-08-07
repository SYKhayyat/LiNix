//! `split_removal_target` is the remove side of `backend:name`, and it had no test at all.
//!
//! `CLAUDE.md`: *"One parser for `backend:name`. Anything that splits on `:` and trusts the
//! prefix is a bug."* This one did not trust the prefix — it asks the registry, which is why it
//! survived `C13` — but it did split the *name* by hand, `name_part.split('@').next()`, and so
//! had never heard of two rules the read side learned separately:
//!
//! - **`Q23`**: an `@` that opens the name is part of the name (npm's `@scope/name`).
//! - **`V.113`**: a quoted name is opaque, spaces and `@` included.
//!
//! Seven call sites carried the result into `rebuild`, `cleanup` (three), `packages` and
//! `upgrade`. The cases below are the shapes those call sites can be handed, not the one that
//! was reported.

use linix::config::parser::split_removal_target;

/// The backends these tests pretend this machine has.
fn known(name: &str) -> bool {
    matches!(name, "apt" | "npm" | "winget" | "cargo")
}

fn split(input: &str) -> (Option<String>, String) {
    split_removal_target(input, known)
}

#[test]
fn a_scoped_npm_name_keeps_its_leading_at() {
    // The reported bug: this returned `(Some("npm"), "")`, and an empty name went to `remove`.
    assert_eq!(
        split("npm:@angular/cli"),
        (Some("npm".into()), "@angular/cli".into())
    );
    assert_eq!(
        split("npm:@bazel/bazelisk"),
        (Some("npm".into()), "@bazel/bazelisk".into())
    );
}

#[test]
fn a_scoped_name_with_options_keeps_the_name_and_drops_the_options() {
    // Only the first `@` is part of the name; every later one still opens the options. This is
    // the case that makes "strip everything from the first `@`" and "strip nothing" both wrong.
    assert_eq!(
        split("npm:@scope/name@version=1.2"),
        (Some("npm".into()), "@scope/name".into())
    );
}

#[test]
fn a_quoted_name_is_taken_whole() {
    // V.113: `winget list` answers with identifiers containing spaces, and a name LiNix lists
    // has to be a name LiNix can be given back. Splitting on `@` cut inside the quotes.
    assert_eq!(
        split(r#"winget:"Some App@2""#),
        (Some("winget".into()), "Some App@2".into())
    );
    assert_eq!(
        split(r#"winget:"Mozilla Firefox""#),
        (Some("winget".into()), "Mozilla Firefox".into())
    );
}

#[test]
fn the_ordinary_shapes_are_unchanged() {
    // The whole point of routing through the grammar is that nothing a caller already relied on
    // moves. These are what the seven call sites see almost every time.
    assert_eq!(split("apt:jq"), (Some("apt".into()), "jq".into()));
    assert_eq!(
        split("apt:jq@version=1.6"),
        (Some("apt".into()), "jq".into())
    );
    assert_eq!(split("jq"), (None, "jq".into()));
    assert_eq!(
        split("cargo:ripgrep@version=14"),
        (Some("cargo".into()), "ripgrep".into())
    );
}

#[test]
fn a_prefix_that_is_not_a_backend_is_not_read_as_one() {
    // C13's rule, and the reason this function takes an oracle rather than splitting blindly:
    // a package name may legitimately contain a colon, and a typo must not become a backend.
    assert_eq!(split("notabackend:jq"), (None, "notabackend:jq".into()));
    assert_eq!(split("aptt:curl"), (None, "aptt:curl".into()));
}

#[test]
fn every_call_site_shape_round_trips_to_something_a_backend_can_be_given() {
    // The family assertion rather than the case list: whatever is handed in, the name that
    // comes out is never empty and never still carries its options. An empty name is what the
    // reported bug produced, and it is the one answer no `remove` can do anything sane with.
    for input in [
        "apt:jq",
        "npm:@angular/cli",
        "npm:@scope/n@version=1",
        r#"winget:"Some App@2""#,
        "jq",
        "cargo:ripgrep",
        "notabackend:jq",
    ] {
        let (_, name) = split(input);
        assert!(!name.is_empty(), "`{}` produced an empty name", input);
        assert!(
            !name.contains("@version="),
            "`{}` left its options on the name: {:?}",
            input,
            name
        );
    }
}
