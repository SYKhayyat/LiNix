//! The latency class table is checked against the source of the table, not against a copy of it.
//!
//! `core/latency.rs::Class::of` says of itself: *"Listed rather than derived, and the list is
//! asserted against `--help` by `tests/latency_budget_tests.rs` — a name that stops existing
//! fails that test rather than sitting here forever, which is the mistake `undo` made in two
//! harness exemption lists."*
//!
//! That test does not read the table. It reads a `NAMED` array of twenty-four strings typed out
//! beside it, and checks *those* against `--help`. So the guarantee is not "every name in the
//! table exists" — it is "every name someone remembered to copy exists", and the two have
//! already diverged: the table classifies `outdated` as `EveryBackend`, `--help` has no
//! `outdated` subcommand, the copy omits the name, and the test passes.
//!
//! The failure it guards against is therefore the failure it demonstrates. `undo` sat in two
//! exemption lists after being renamed away because nothing validated the list; the fix
//! validated a transcription of the list.
//!
//! **What the dead row costs.** `Class::of("outdated")` is unreachable, so the arm is dead
//! rather than wrong, and the visible damage is small. The reason to care is that it is the
//! only evidence available about whether the table is maintained — and the one check built to
//! produce that evidence cannot see the one row where it is missing. A gate that reports on a
//! copy reports on the copy.
//!
//! The fix is to derive: read the arms, or drive `Class::of` off the same variant list
//! `subcommand_name` already walks. Not to correct the array — the array being correctable by
//! hand is the defect.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn help_text() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .arg("--help")
        .output()
        .expect("the binary should run");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Every string literal in the body of `Class::of`, in source order.
///
/// Read out of the file rather than called, because a `match` cannot be enumerated from
/// outside it — which is the same property that let a dead arm sit there.
fn names_in_the_class_table() -> Vec<String> {
    let src = std::fs::read_to_string(repo_root().join("src/core/latency.rs"))
        .expect("src/core/latency.rs should be readable");
    let start = src
        .find("pub fn of(subcommand: &str) -> Class {")
        .expect("`Class::of` should still be spelled this way");
    let body = &src[start..];
    let end = body
        .find("_ => Class::Mutating,")
        .expect("`Class::of` should still end with its fallthrough arm");

    let mut out = Vec::new();
    for line in body[..end].lines() {
        let code = match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        };
        let mut rest = code;
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('"') else { break };
            let name = &after[..close];
            if !name.is_empty() {
                out.push(name.to_string());
            }
            rest = &after[close + 1..];
        }
    }
    out
}

fn help_lists(help: &str, name: &str) -> bool {
    help.contains(&format!("  {name} ")) || help.contains(&format!("  {name}\n"))
}

/// The extractor finds the names it is supposed to find.
///
/// A source-scanning test that silently matched nothing would pass forever, which is the
/// failure mode of the test it replaces. These five are spread across all three arms.
#[test]
fn the_table_reader_reads_the_table() {
    let names = names_in_the_class_table();
    assert!(
        names.len() >= 20,
        "only {} name(s) came out of `Class::of`; the extractor has lost the body: {names:?}",
        names.len()
    );
    for expected in ["policy", "help", "info", "list", "adopt"] {
        assert!(
            names.iter().any(|n| n == expected),
            "`{expected}` is in `Class::of` and the extractor did not find it: {names:?}"
        );
    }
}

/// Every name the class table classifies is a subcommand the binary has.
#[test]
fn every_name_in_the_class_table_is_a_real_subcommand() {
    let help = help_text();
    let dead: Vec<String> = names_in_the_class_table()
        .into_iter()
        .filter(|n| !help_lists(&help, n))
        .collect();

    assert!(
        dead.is_empty(),
        "the latency class table classifies {} name(s) `--help` does not list: {:?}\n\n\
         `Class::of` cannot be reached with these, so the arm is dead. The check that was \
         supposed to catch this reads a hand-copied array instead of the table.",
        dead.len(),
        dead
    );
}

/// And the copy that stands in for the table covers it.
///
/// Kept separate from the test above because they fail for different reasons and want
/// different fixes: that one says a row is dead, this one says the instrument cannot see the
/// row. Deleting the array once the check derives its names makes this test unnecessary, which
/// is the outcome to aim for.
#[test]
fn the_hand_copied_list_covers_every_name_in_the_table() {
    let guard = std::fs::read_to_string(repo_root().join("tests/latency_budget_tests.rs"))
        .expect("tests/latency_budget_tests.rs should be readable");
    let start = guard
        .find("const NAMED: &[&str] = &[")
        .expect("the guard should still hold a copy of the table");
    let end = start
        + guard[start..]
            .find("];")
            .expect("the copy should still be a closed array");
    let copy = &guard[start..end];

    let missing: Vec<String> = names_in_the_class_table()
        .into_iter()
        .filter(|n| !copy.contains(&format!("\"{n}\"")))
        .collect();

    assert!(
        missing.is_empty(),
        "the class table names {} thing(s) the list that guards it does not: {:?}\n\n\
         Whatever is on this list is invisible to `every_subcommand_the_class_table_names_still_\
         exists`, which is the only check the table has.",
        missing.len(),
        missing
    );
}
