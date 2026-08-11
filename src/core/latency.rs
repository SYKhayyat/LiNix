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

/// What a fan-out has to look like, for the classes whose seconds belong to the host.
///
/// **The budget the wall clock cannot express.** `EveryBackend` costs whatever 24 managers cost,
/// so a ceiling in seconds is either useless or red on a busy afternoon — which is why it was
/// `None`. But LiNix's *share* of that is measurable and it is already measured: `--timings`
/// computes the overlap ratio and the wave count on every run that asks for them, and nothing
/// read either. So the regression this could not see is the important one: a change that
/// serialises a fan-out drops overlap from 6.3× to 1.2×, the wall clock stays inside a budget of
/// `None`, and it stays there for ever. Measured on Windows with 24 ready backends: 23 child
/// commands summing to 23.67 s inside 3.75 s of wall clock, 6.3× overlap, 2 waves.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    /// Summed child time over wall clock. A collapse to serial is ~1.0.
    pub min_overlap: f64,
    /// How many times the run went completely quiet and started again. One means everything
    /// overlapped; `n` means it stopped `n - 1` times to wait for an answer.
    pub max_waves: usize,
    /// Below this many child commands the ratio is not a measurement of anything — three
    /// managers on a bare CI runner cannot overlap 2×, and a gate that says they should is a
    /// gate people learn to ignore.
    pub min_children: usize,
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

    /// The shape budget, for the class whose cost is the host's but whose *scheduling* is not.
    ///
    /// `Mutating` is deliberately exempt: its waves are the plan's, and a dependency edge is a
    /// wave on purpose. Asserting a shape there would fail a graph that is doing exactly what
    /// it was asked to do.
    ///
    /// The numbers are collapse detectors, not targets, for the same reason the second budgets
    /// are set an order of magnitude above what was measured: 2× against a measured 6.3×
    /// catches serialisation and survives a loaded runner.
    pub fn shape(self) -> Option<Shape> {
        match self {
            Class::EveryBackend => Some(Shape {
                min_overlap: 2.0,
                max_waves: 2,
                min_children: 4,
            }),
            Class::ConfigOnly | Class::OneBackend | Class::Mutating => None,
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
    report_shape(subcommand, class);
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

/// Why a fan-out's shape is out of budget, or `None` if it is fine or unmeasurable.
///
/// Pure, and separate from the reporting, so the rule can be asserted without a package manager
/// and without a clock. Every argument comes from `core::timing`, which already computes all of
/// them for `--timings`.
pub fn shape_violation(
    shape: Shape,
    children: usize,
    summed: Duration,
    wall: Duration,
    waves: usize,
) -> Option<String> {
    if children < shape.min_children {
        return None;
    }
    let overlap = summed.as_secs_f64() / wall.as_secs_f64().max(f64::EPSILON);
    let mut faults = Vec::new();
    if overlap < shape.min_overlap {
        faults.push(format!(
            "{:.1}x overlap, under the {:.1}x floor — {} child command(s) summing to {:.2}s ran \
             in {:.2}s of wall clock, which is close to running them one at a time",
            overlap,
            shape.min_overlap,
            children,
            summed.as_secs_f64(),
            wall.as_secs_f64()
        ));
    }
    if waves > shape.max_waves {
        faults.push(format!(
            "{} wave(s), over the ceiling of {} — the run went quiet {} time(s), because \
             something had to be answered before the next question could be asked",
            waves,
            shape.max_waves,
            waves - 1
        ));
    }
    (!faults.is_empty()).then(|| faults.join("; "))
}

/// Say when a fan-out stopped fanning out.
///
/// Only on a run that asked for `--timings`, because that is the only run that records spans.
/// That is a real limit and it is stated rather than papered over: the *gate* is
/// `tests/latency_budget_tests.rs`, which drives the fan-out commands with `--timings` on
/// purpose. This is what puts the same sentence in front of a user who asked.
fn report_shape(subcommand: &str, class: Class) {
    let Some(shape) = class.shape() else { return };
    if !crate::core::timing::is_enabled() {
        return;
    }
    let (rows, _, summed) = crate::core::timing::summary();
    let children: usize = rows.iter().map(|r| r.calls).sum();
    let Some(why) = shape_violation(
        shape,
        children,
        summed,
        crate::core::timing::elapsed(),
        crate::core::timing::waves(),
    ) else {
        return;
    };
    tracing::warn!(
        "`linix {}` asked every manager and did not overlap them: {}. The seconds a fan-out \
         costs belong to the host; the scheduling does not.",
        subcommand,
        why
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape budget catches the regression the second budget cannot see.
    ///
    /// A change that serialises the fan-out leaves the wall clock inside a ceiling of `None`
    /// for ever. These are the same numbers measured on Windows with 24 ready backends —
    /// 23 children, 23.67 s of child time in 3.75 s of wall clock — against the same run with
    /// the overlap taken out of it.
    #[test]
    fn a_serialised_fan_out_is_out_of_budget_and_a_real_one_is_not() {
        let shape = Class::of("list")
            .shape()
            .expect("`list` asks every manager");

        let healthy = shape_violation(
            shape,
            23,
            Duration::from_millis(23_670),
            Duration::from_millis(3_750),
            2,
        );
        assert!(
            healthy.is_none(),
            "the measured release-build run is reported as a violation: {healthy:?}"
        );

        // The same children, run one after another. Wall clock == summed, so overlap is 1.0.
        let serial = shape_violation(
            shape,
            23,
            Duration::from_millis(23_670),
            Duration::from_millis(23_670),
            23,
        );
        let serial = serial.expect(
            "a fan-out that overlapped nothing is inside budget, which is the regression \
             `Class::EveryBackend => None` could not see",
        );
        assert!(serial.contains("overlap"), "{serial}");
        assert!(serial.contains("wave"), "{serial}");
    }

    /// Three managers on a bare runner cannot overlap 2×, and a gate that says they should is a
    /// gate people learn to ignore (`lifecycle-floor.txt` says the same about guessed constants).
    #[test]
    fn too_few_children_is_not_a_measurement() {
        let shape = Class::of("list").shape().unwrap();
        assert!(shape_violation(
            shape,
            2,
            Duration::from_millis(2_000),
            Duration::from_millis(2_000),
            2
        )
        .is_none());
    }

    /// `Mutating` is exempt on purpose: its waves are the dependency graph's, and an edge is a
    /// wave by design. A shape assertion there fails a plan doing exactly what it was asked to.
    #[test]
    fn only_the_fan_out_class_carries_a_shape() {
        assert!(Class::of("list").shape().is_some());
        assert!(Class::of("check").shape().is_some());
        assert!(Class::of("sync").shape().is_none());
        assert!(Class::of("eval").shape().is_none());
        assert!(Class::of("info").shape().is_none());
    }

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
