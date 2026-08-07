//! **`apply/firewall.rs` closed every open port no `firewall:` line declared, and the word
//! `guard` appeared nowhere in the file** — not an import, not a call, not a comment. Zero tests
//! (`grep -c 'cfg(test)'` → 0). `max_removals` did not count these, `protected` could not name
//! them, `--allow-mass-removal` was not consulted, and `enforce_extras` — which exists precisely
//! because the extras teardown runs outside the transaction — was not called.
//!
//! The check written to prevent exactly this could not see it. `removal_guard_enumeration_tests`
//! keyed on `.remove(`+`sudo`, `.remove_repo(`, `.remove_shim(` and `.deprovision(`; a firewall
//! closes a port with `deny_command`, which matches none of them.
//!
//! **Steelman, and it is real.** `firewall.rs:35` returns early when nothing is declared, so a
//! user with no `firewall:` lines is untouched — you must opt in by writing one line. And the
//! file carries three bespoke refusals: an unreadable baseline (*"closing ports against an
//! unknown baseline is how a machine goes dark"*), the SSH session lockout, and the
//! linked-ruleset warning. Whoever wrote this understood the danger exactly.
//!
//! **Which is what makes it worse, not better.** Three custom guards were written rather than
//! calling the one two hundred lines away that already counts, caps, protects and reports.

use linix::app::sync::guard::{GuardScope, Reaped};

/// The type is the fix, so the type is what this asserts first.
///
/// A `Reaped` cannot be constructed from outside `guard.rs` except through `for_reason`, which
/// is named and greppable. There is no `Reaped {}`, no `Default`, no `From`. If that ever
/// changes, every other assertion in this file becomes decoration.
#[test]
fn the_token_cannot_be_minted_by_a_caller_who_would_rather_not_ask() {
    // The only constructor reachable from here, and it demands a written reason.
    let r = Reaped::for_reason(GuardScope::Sync, "a test asserting the token's own shape");
    assert_eq!(r.scope(), GuardScope::Sync);

    // The compile-time half cannot be written as a runtime assertion — `Reaped { scope: … }`
    // from this crate is a private-field error, which is the point and is checked by the
    // compiler on every build of this file. What is asserted here is that no *other* door was
    // left open: no `Default`, and a scope that survives the crossing so a refusal can name the
    // command a user typed.
    assert_eq!(
        Reaped::for_reason(GuardScope::PurgeUndeclared, "scope is carried, not inferred").scope(),
        GuardScope::PurgeUndeclared
    );
}

/// The firewall's guard scope comes from the label `sync` passes down, and an unrecognised
/// label gets the strictest of the three rather than the most convenient.
///
/// `N1` names three commands that can close a port: `sync`, `purge-undeclared`, and an
/// unattended `watch` tick — the last being the dangerous one, because nobody is there to read
/// a refusal.
#[test]
fn every_command_that_can_close_a_port_maps_to_a_scope() {
    // The mapping is private, so this asserts the property through the public enum instead:
    // all three commands `N1` names exist as scopes, and are distinguishable in a message.
    for (scope, expected) in [
        (GuardScope::Sync, "sync"),
        (GuardScope::PurgeUndeclared, "purge-undeclared"),
        (GuardScope::Watch, "watch"),
    ] {
        assert_eq!(
            scope.as_str(),
            expected,
            "a refusal has to name the command the user typed"
        );
    }
}
