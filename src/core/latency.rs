//! How long a command may take before LiNix says so.
//!
//! Nothing measured latency, which is how a 98-second `linix info cargo:ripgrep` shipped —
//! answering `not found in any available backend` about a package `linix search ripgrep` found
//! in the same tree, seconds later (E14/E15). W14 fixed that one `info`; the budget it asked
//! for was never built, so nothing would notice the next one.
//!
//! **The split is diagnostic, not arbitrary.** Measured on Windows with 24 ready backends,
//! debug build, a fixture with 111 adopted packages:
//!
//! ```text
//! policy / vars / eval / check config        0.13 – 0.32 s
//! list                                       3.4  – 3.9  s
//! check health                               4.3  – 5.4  s
//! check                                      8.5  – 18.3 s
//! ```
//!
//! A command that only reads files is fast on every machine. A command that asks every manager
//! costs whatever the managers cost, and that is a fact about the host rather than about LiNix.
//! So the budget is per **class**, and only the two classes whose cost LiNix controls carry a
//! hard one.
//!
//! **The numbers are ceilings a collapse crosses, not targets.** A budget tight enough to
//! police 3 seconds against 5 on a shared CI runner is a gate that goes red on load, and a gate
//! that goes red on load is a gate people learn to ignore — `lifecycle-floor.txt` says the same
//! thing about guessed constants. These are set an order of magnitude above what was measured,
//! so what crosses them is the 98-second shape and not a busy afternoon.

use std::time::Duration;

/// What a command has to do before it can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Reads the config and answers. Asks no manager anything, so its cost is LiNix's alone.
    ConfigOnly,
    /// Asks exactly one manager, because the user named it — `info cargo:ripgrep`. The
    /// qualifier is the whole point: it is what makes this cheaper than asking everybody, and
    /// E14 was that the qualifier did not narrow the probe.
    OneBackend,
    /// Asks every ready manager. Its cost belongs to the host — 24 managers on this box — so it
    /// is measured and reported, never failed.
    EveryBackend,
    /// Changes the machine. Bounded by what a package manager does, which is unbounded.
    Mutating,
}

impl Class {
    /// The ceiling, or `None` where the cost is the host's rather than LiNix's.
    pub fn budget(self) -> Option<Duration> {
        match self {
            Class::ConfigOnly => Some(Duration::from_secs(5)),
            Class::OneBackend => Some(Duration::from_secs(15)),
            Class::EveryBackend | Class::Mutating => None,
        }
    }

    /// Which class a subcommand is in, by the name `--help` prints for it.
    ///
    /// Listed rather than derived, and the list is asserted against `--help` by
    /// `tests/latency_budget_tests.rs` — a name that stops existing fails that test rather than
    /// sitting here forever, which is the mistake `undo` made in two harness exemption lists.
    pub fn of(subcommand: &str) -> Class {
        match subcommand {
            // Reads files, answers, stops.
            "policy" | "vars" | "eval" | "why" | "protected" | "completions" | "path"
            | "history" | "diff" | "sbom" | "export" | "plan" | "profile" | "module" | "edit"
            | "config" | "hooks" | "schedule" | "fleet" | "help" => Class::ConfigOnly,
            "info" => Class::OneBackend,
            "list" | "search" | "check" | "outdated" | "adopt" => Class::EveryBackend,
            _ => Class::Mutating,
        }
    }
}

/// The subcommand name clap prints, taken off the `Commands` variant's own `Debug`.
///
/// Derived rather than listed: a second table of sixty-six names beside the enum is the shape
/// that produced an exemption for `undo`, a subcommand renamed away, sitting in two harness
/// lists for months. clap's rule is kebab-case of the variant name — `SelfUpgrade` is
/// `self-upgrade` — so the conversion is the same one clap made, in the other direction.
pub fn subcommand_name(command: &impl std::fmt::Debug) -> String {
    let debug = format!("{:?}", command);
    let variant = debug
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or("");
    let mut out = String::with_capacity(variant.len() + 2);
    for (i, c) in variant.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Say how long a command took, when it took longer than its class allows.
///
/// Reported rather than refused: a slow answer is still the answer, and a program that aborted
/// a `check` at five seconds would have turned a performance defect into a correctness one. The
/// point is that the number reaches somebody — E14 shipped because nobody was counting.
pub fn report_if_over(subcommand: &str, elapsed: Duration) {
    let class = Class::of(subcommand);
    let Some(budget) = class.budget() else { return };
    if elapsed <= budget {
        return;
    }
    tracing::warn!(
        "`linix {}` took {:.1}s. A {} command is budgeted {}s — this one is over, which is the \
         shape of the 98-second `info` that could not be seen because nothing measured it.",
        subcommand,
        elapsed.as_secs_f64(),
        match class {
            Class::ConfigOnly => "config-only",
            Class::OneBackend => "single-backend",
            _ => "read",
        },
        budget.as_secs(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_classes_linix_controls_carry_a_budget_and_the_others_do_not() {
        assert!(Class::of("eval").budget().is_some());
        assert!(Class::of("info").budget().is_some());
        // A host with forty managers is slow because it has forty managers.
        assert!(Class::of("list").budget().is_none());
        assert!(Class::of("sync").budget().is_none());
    }

    #[test]
    fn a_command_inside_its_budget_is_not_reported() {
        // Nothing to assert on the output here — the value is that this cannot panic and that
        // the boundary is inclusive, so a command exactly at its budget is not "over".
        assert!(Duration::from_secs(5) <= Class::ConfigOnly.budget().unwrap());
    }
}
