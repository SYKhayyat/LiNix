//! **The oracle for the shared oracle.**
//!
//! `tests/ledger/mod.rs` now carries the four assertions that nine scanning gates each wrote out
//! by hand. That concentration is worth having only if the shared copy is the strict one — a
//! helper that is quietly permissive fails nine gates at once instead of one, and it fails them
//! silently, because a delegated assertion looks identical to a working one at the call site.
//!
//! So each of the four is driven over a planted input built to violate it, and watched to panic.
//! This is the step whose omission *is* finding 4: three gates asserted over a walk that had
//! stopped matching, and passed by finding nothing.

use std::collections::BTreeSet;

use crate::ledger::{Entry, Ledger};

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

const REASON: &str = "a sentence long enough to be an actual explanation of the mechanism, \
                      rather than a label standing in for one";

fn ledger() -> Ledger<'static> {
    Ledger::of("a planted offence", "PLANTED")
        .exempting([Entry {
            site: "excused.rs",
            why: REASON,
        }])
        .scanning_at_least(10)
}

/// The happy path, so a green run below cannot be explained by "it panics on everything".
#[test]
fn a_table_that_matches_the_walk_passes() {
    ledger().audit(50, &set(&["excused.rs"]));
}

/// Assertion 1. The check three of the nine gates did not have.
#[test]
#[should_panic(expected = "has no floor")]
fn a_ledger_without_a_floor_refuses_to_audit() {
    Ledger::of("a planted offence", "PLANTED")
        .pairs(&[("excused.rs", REASON)])
        .audit(50, &set(&["excused.rs"]));
}

#[test]
#[should_panic(expected = "under its floor")]
fn a_walk_that_read_almost_nothing_is_not_a_clean_walk() {
    ledger().audit(3, &set(&["excused.rs"]));
}

/// Assertion 2. A predicate that has stopped recognising its subject reads exactly like a tree
/// that has been cleaned up, and the difference is the whole value of the gate.
#[test]
#[should_panic(expected = "matched nothing at all")]
fn a_predicate_that_matches_nothing_is_a_failure_not_a_pass() {
    ledger().audit(500, &set(&[]));
}

/// Assertion 3.
#[test]
#[should_panic(expected = "not in PLANTED")]
fn a_site_the_walk_found_and_nothing_excuses_fails() {
    ledger().audit(50, &set(&["excused.rs", "surprise.rs"]));
}

#[test]
fn the_remedy_is_printed_beside_the_unexplained_site() {
    let panic = std::panic::catch_unwind(|| {
        ledger()
            .remedy("Route it through the one writer.")
            .audit(50, &set(&["surprise.rs"]));
    })
    .expect_err("an unexplained site must fail");
    let msg = panic
        .downcast_ref::<String>()
        .expect("the panic carries a message");
    assert!(
        msg.contains("Route it through the one writer."),
        "the remedy did not reach the failure a reader will actually see:\n{msg}"
    );
}

/// Assertion 4, first half. `helm` was exempt for a reason that had stopped being true, and
/// nothing said so — because the loop that read the table never read the reason.
#[test]
#[should_panic(expected = "excuses [\"gone.rs\"]")]
fn an_entry_the_walk_no_longer_finds_fails() {
    // One live exemption and one dead one, so the unexplained check above cannot fire first and
    // pass this test for the wrong reason.
    Ledger::of("a planted offence", "PLANTED")
        .pairs(&[("excused.rs", REASON), ("gone.rs", REASON)])
        .scanning_at_least(10)
        .audit(50, &set(&["excused.rs"]));
}

/// Assertion 4, second half.
#[test]
#[should_panic(expected = "no reason worth the name")]
fn an_entry_whose_reason_is_a_label_fails() {
    Ledger::of("a planted offence", "PLANTED")
        .pairs(&[("excused.rs", "legacy")])
        .scanning_at_least(10)
        .audit(50, &set(&["excused.rs"]));
}

/// The floor on the reason is per-site, because "it is a Windows path" is a complete reason and
/// "it reaches into the JSON a `PropertyProbe` returns, which a row cannot express" is a
/// different kind of claim needing a different length to make.
#[test]
#[should_panic(expected = "floor 200")]
fn a_raised_reason_floor_is_the_one_enforced() {
    Ledger::of("a planted offence", "PLANTED")
        .pairs(&[("excused.rs", REASON)])
        .scanning_at_least(10)
        .reason_of_at_least(200)
        .audit(50, &set(&["excused.rs"]));
}

/// A reason is measured in characters, not bytes, so a table written by someone whose
/// explanation is not ASCII is held to the same length rather than a shorter one.
#[test]
#[should_panic(expected = "no reason worth the name")]
fn a_reason_is_measured_in_characters() {
    let accented = "é".repeat(30); // 30 chars, 60 bytes — under the floor either way it is read
    let table: &[(&str, &str)] = &[("excused.rs", Box::leak(accented.into_boxed_str()))];
    Ledger::of("a planted offence", "PLANTED")
        .pairs(table)
        .scanning_at_least(10)
        .audit(50, &set(&["excused.rs"]));
}
