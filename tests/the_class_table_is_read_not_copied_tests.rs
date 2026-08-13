//! The gate over the latency class table reads **the table**, not a transcription of it.
//!
//! **What was found, 2026-08-13.** `Class::of` classified `"outdated"` as `EveryBackend`. There
//! is no `outdated` subcommand — it is `shall list --outdated`, a flag — so the arm was dead.
//!
//! The interesting half was why nothing caught it. `Class::of`'s own doc said *"the list is
//! asserted against `--help` by `tests/latency_budget_tests.rs` — a name that stops existing
//! fails that test rather than sitting here forever, which is the mistake `undo` made in two
//! harness exemption lists."* That test did not read the table. It read a `NAMED` array of
//! twenty-four hand-typed strings sitting beside it, and the array omitted `outdated` and
//! `help`. **The failure it guarded against was the failure it demonstrated**: `undo` sat in two
//! exemption lists because nothing validated the list, and the cure validated a transcription of
//! the list.
//!
//! **Why the copy existed, which is the part worth keeping.** `Class::of` was a `match`, and a
//! `match` cannot be enumerated from outside it — so a test that wanted the names had no way to
//! ask for them and typed them out instead. The copy was not laziness; it was the only thing
//! available. The table is data now (`CLASSIFIED`) and the program exposes
//! `Class::classified_names()`, so `latency_budget_tests` reads it and there is nothing to
//! drift from.
//!
//! **So this file no longer checks the names** — `latency_budget_tests` does that, against
//! `--help`, from the table itself. What it checks is the *structural* property that made the
//! bug possible, because that is the thing that can come back: a gate over a list must not keep
//! its own copy of the list. The original version of this file said so in its own words —
//! *"deleting the array once the check derives its names makes this test unnecessary, which is
//! the outcome to aim for"* — and this is that outcome, with a lock on the door behind it.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn guard_source() -> String {
    let p = repo_root().join("tests/latency_budget_tests.rs");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The guard asks the program for the names instead of holding them.
#[test]
fn the_guard_reads_the_table_rather_than_a_copy() {
    let guard = guard_source();

    assert!(
        guard.contains("classified_names()"),
        "`latency_budget_tests` no longer asks `Class::classified_names()` for the subcommands \
         it checks against `--help`. Whatever it reads instead is a second statement of the \
         table, and the two can disagree — which is exactly how `outdated` came to be \
         classified for years after it stopped being a subcommand."
    );

    assert!(
        !guard.contains("const NAMED"),
        "`latency_budget_tests` has grown a hand-typed list of subcommand names again. That is \
         the defect this file exists about: the previous one omitted `outdated`, so the gate \
         written to catch a stale name could not see the stale name."
    );
}

/// And the table is data the program can enumerate, which is what makes the above possible.
///
/// The self-test. Without it, a `classified_names()` that returned nothing would satisfy every
/// assertion in `latency_budget_tests` and this file would still be green — a gate over a gate,
/// both measuring an empty set.
#[test]
fn the_table_can_be_enumerated_and_is_not_empty() {
    let names: Vec<&str> = shall::core::latency::classified_names().collect();

    assert!(
        names.len() > 20,
        "the class table yielded {} names. `latency_budget_tests` checks each of them against \
         `--help`, so a table that cannot be read makes that gate vacuous rather than red.",
        names.len()
    );
    assert!(
        names.contains(&"check") && names.contains(&"list"),
        "the table does not name the commands whose latency it exists to classify: {names:?}"
    );
    assert!(
        !names.contains(&"outdated"),
        "`outdated` is back in the class table. It is not a subcommand — `shall list --outdated` \
         is a flag — and `H3` ruled that the name comes out rather than the subcommand going in."
    );
}
