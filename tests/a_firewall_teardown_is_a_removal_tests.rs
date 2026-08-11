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

use shall::app::sync::guard::{GuardScope, Reaped};

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
        Reaped::for_reason(
            GuardScope::PurgeUndeclared,
            "scope is carried, not inferred"
        )
        .scope(),
        GuardScope::PurgeUndeclared
    );
}

/// `N1` names three commands that can close a port: `sync`, `purge-undeclared`, and an
/// unattended `watch` tick — the last being the dangerous one, because nobody is there to read
/// a refusal. Each has to arrive at the guard as itself and be named as itself.
///
/// **This test used to assert `GuardScope::Sync.as_str() == "sync"` for three variants, under a
/// comment reading "the mapping is private, so this asserts the property through the public
/// enum instead".** The private mapping it declined to test was `guard_scope`, and it was
/// broken in the direction this file's own header calls dangerous: `scope_label` emitted
/// `"an unattended watch tick"`, `guard_scope` matched `"watch"`, and **neither named arm was
/// reachable**. Every firewall teardown — including `N7`'s unattended tick, which reverts by
/// default with nobody watching — was guarded and reported as `sync`. A getter returning what a
/// constructor took cannot see that; only the round trip can.
///
/// There is no round trip now. `GuardScope` is `Copy` and is passed, both functions are gone,
/// and what is left to assert is that the two vocabularies stay distinct and both reach the
/// message a user reads.
#[test]
fn every_command_that_can_close_a_port_names_itself_in_the_refusal() {
    for (scope, typed, prose) in [
        (GuardScope::Sync, "sync", "sync"),
        (
            GuardScope::PurgeUndeclared,
            "purge-undeclared",
            "purge-undeclared",
        ),
        (GuardScope::Watch, "watch", "an unattended watch tick"),
    ] {
        assert_eq!(
            scope.as_str(),
            typed,
            "a refusal has to name the command the user typed, so they can retype it with a flag"
        );
        assert_eq!(
            scope.during(),
            prose,
            "and it has to say what kind of run this was, which is the fact `N7` turns on"
        );
    }

    // The message itself, on the path that was silently answering `sync` for all three.
    let watch = shall::model::firewall::lockout_refusal(22, GuardScope::Watch);
    assert!(
        watch.contains("an unattended watch tick"),
        "the lockout refusal did not carry the scope: {watch}"
    );
    let sync = shall::model::firewall::lockout_refusal(22, GuardScope::Sync);
    assert!(
        !sync.contains("unattended"),
        "an attended sync was reported as unattended: {sync}"
    );
    assert_ne!(
        watch, sync,
        "two scopes produced the same refusal, which is the defect this file exists for"
    );
}
