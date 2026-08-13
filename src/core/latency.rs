//! How long a command may take before Shall says so.
//!
//! Nothing measured latency, which is how a 98-second `shall info cargo:ripgrep` shipped —
//! answering `not found in any available backend` about a package `shall search ripgrep` found
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
//! costs whatever the managers cost, and that is a fact about the host rather than about Shall.
//! So the budget is per **class**, and only the two classes whose cost Shall controls carry a
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
    /// Reads the config and answers. Asks no manager anything, so its cost is Shall's alone.
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
/// `None`. But Shall's *share* of that is measurable and it is already measured: `--timings`
/// computes the overlap ratio and the wave count on every run that asks for them, and nothing
/// read either. So the regression this could not see is the important one: a change that
/// serialises a fan-out drops overlap from 6.3× to 1.2×, the wall clock stays inside a budget of
/// `None`, and it stays there for ever.
///
/// **Collapse detectors, not targets, and the difference cost a red gate to learn.**
///
/// The first version of this carried `min_overlap: 2.0` and `max_waves: 2`, taken from the one
/// host it was written on — Windows, 24 ready backends, 6.3× and 2 waves. ubuntu-latest runs 16
/// child commands and reported **2.0× and 3 waves**: sitting exactly on one floor and over the
/// other, on a machine doing nothing wrong. That is the mistake `lifecycle-floor.txt` is entirely
/// about — *"the honest number varies by host, and a number guessed once is the kind of constant
/// this repo keeps discovering was wrong"* — committed while building the gate.
///
/// So neither number is absolute now. What is actually being asserted is that the fan-out is not
/// **serial**, and a serial run has an exact signature: overlap ≈ 1.0 and one wave per child.
/// Both bounds are expressed against that, which makes them true on a host nobody has measured.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    /// Summed child time over wall clock. A collapse to serial is ~1.0; the lowest legitimate
    /// reading seen on any host is 2.0, so this sits between them.
    ///
    /// **A floor only, and a floor on this number can be satisfied by making things worse.**
    /// The ratio is `sum(child time) / wall`, and contention inflates the numerator: measured
    /// over the same 23 children, width 20 spent 676.6s of child time against width 4's
    /// 182.5s — 3.7× the total work — and the ratio *rose* from 1.6× to 8.3× for it. Wall clock
    /// improved monotonically, so the parallelism earns its keep and this is not a budget on
    /// the design. It does mean the floor cannot be the only thing read: the gate pairs it with
    /// an arithmetic ceiling (a run cannot average more children in flight than it has) and
    /// prints the numerator, so the one regression neither catches — every child getting slower
    /// at constant concurrency, which moves `sum` and `wall` together — is at least visible in
    /// a diff of two runs.
    pub min_overlap: f64,
    /// The wave ceiling as a fraction of the child count: waves may not exceed
    /// `children / waves_per_child`. A serial run has one wave per child, so the ceiling scales
    /// with the fan-out instead of pinning a number measured on somebody's desk.
    pub waves_per_child: usize,
    /// The floor under that fraction, so a small fan-out is not policed into failure: four
    /// children over a divisor of three would allow one wave, which no real run achieves.
    pub min_waves_allowed: usize,
    /// Below this many child commands the ratio is not a measurement of anything — three
    /// managers on a bare CI runner cannot overlap 2×, and a gate that says they should is a
    /// gate people learn to ignore.
    pub min_children: usize,
}

impl Shape {
    /// How many waves this many children may take before the run is serial enough to fail.
    pub fn wave_ceiling(&self, children: usize) -> usize {
        (children / self.waves_per_child.max(1)).max(self.min_waves_allowed)
    }
}

impl Class {
    /// The ceiling, or `None` where the cost is the host's rather than Shall's.
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
    /// **What the right bound for it would be, named here because "exempt" is where a gap
    /// hides.** The 2026-08-13 review's objection is fair — `sync`, `upgrade` and `rebuild` are
    /// the slowest things this program does and the ones a person waits on, and the mechanism
    /// that caught the `sbom`/`export` collapse has never been pointed at them. The reason it
    /// cannot be pointed at them *as written* is that `waves_per_child` is a guess about the
    /// shape of the work, and a mutating run's shape is not a guess: it is the plan's own
    /// **critical-path depth**, which the engine holds and this layer never sees. The honest
    /// bound is `waves <= depth(graph)` — exact, unguessable, and false for no correct run —
    /// and building it means the engine reporting its own shape after
    /// `execute_with_telemetry`, because `report_if_over` is handed a subcommand name and
    /// nothing else.
    ///
    /// **That is not built.** What is deliberately *not* done in the meantime is giving
    /// `Mutating` a permissive `Shape` so the class stops looking exempt: a ceiling of
    /// `children` waves cannot be crossed by any run, and a gate that cannot fail is the exact
    /// defect the rest of this review is about.
    ///
    /// Every number here is a collapse detector — see [`Shape`] for what the first draft's
    /// targets cost. Four readings, from three platforms, all of them healthy:
    ///
    /// ```text
    /// Windows, release   23 children   6.3x   2 waves
    /// Windows, debug     23 children   5.7x   2 waves
    /// ubuntu-latest      16 children   2.0x   3 waves
    /// macos-latest        9 children   1.9x   3 waves
    /// ```
    ///
    /// A serial run is 1.0× with one wave per child. The floor sits at 1.5× — below every
    /// reading above and half again above serial — and the wave ceiling at half the child count
    /// with a floor of four, which still catches a nine-child run that took nine waves while
    /// leaving the honest three alone.
    pub fn shape(self) -> Option<Shape> {
        match self {
            Class::EveryBackend => Some(Shape {
                min_overlap: 1.5,
                waves_per_child: 2,
                min_waves_allowed: 4,
                min_children: 4,
            }),
            Class::ConfigOnly | Class::OneBackend | Class::Mutating => None,
        }
    }

    /// Which class a subcommand is in, by the name `--help` prints for it.
    pub fn of(subcommand: &str) -> Class {
        CLASSIFIED
            .iter()
            .find(|(name, _)| *name == subcommand)
            .map_or(Class::Mutating, |(_, class)| *class)
    }
}

/// Every subcommand this table names, with its class.
///
/// **Data rather than a `match`, and that is the whole point of the shape.** The gate that keeps
/// these names honest — `tests/latency_budget_tests.rs` — asserts them against `--help`, and a
/// `match` gives it nothing to iterate, so it read a hand-typed array of twenty-four strings
/// beside it instead. The copy omitted two entries, and one of them was `outdated`: a name
/// classified here that is not a subcommand at all (`shall list --outdated` is a flag), so the
/// arm was dead and the gate written to catch exactly that could not see it.
///
/// **The failure the gate guarded against was the failure it demonstrated** — `undo` sat in two
/// harness exemption lists because nothing validated the list, and the cure validated a
/// transcription of the list. A name that stops existing now fails that test, because the test
/// reads this.
///
/// Scanned linearly, and that is not worth a map: this is consulted once per command invocation.
const CLASSIFIED: &[(&str, Class)] = &[
    // Reads files, answers, stops.
    ("policy", Class::ConfigOnly),
    ("vars", Class::ConfigOnly),
    ("eval", Class::ConfigOnly),
    ("why", Class::ConfigOnly),
    ("protected", Class::ConfigOnly),
    ("completions", Class::ConfigOnly),
    ("path", Class::ConfigOnly),
    ("history", Class::ConfigOnly),
    ("diff", Class::ConfigOnly),
    ("plan", Class::ConfigOnly),
    ("profile", Class::ConfigOnly),
    ("module", Class::ConfigOnly),
    ("edit", Class::ConfigOnly),
    ("config", Class::ConfigOnly),
    ("hooks", Class::ConfigOnly),
    ("schedule", Class::ConfigOnly),
    ("fleet", Class::ConfigOnly),
    ("help", Class::ConfigOnly),
    ("info", Class::OneBackend),
    // **`sbom` and `export` are here, not in `ConfigOnly`.** They were filed as "reads files,
    // answers, stops" and they spawn one child process per manager — so the shape gate never
    // looked at them while they ran a serial loop at 1.0× overlap and twenty-one waves over
    // twenty-one children, and the five-second config budget they were handed printed a `WARN`
    // on an ordinary run. A class is about what a command *does*, not how it reads.
    ("sbom", Class::EveryBackend),
    ("export", Class::EveryBackend),
    ("list", Class::EveryBackend),
    ("search", Class::EveryBackend),
    ("check", Class::EveryBackend),
    ("adopt", Class::EveryBackend),
];

/// The names [`CLASSIFIED`] classifies, for the gate that checks them against `--help`.
///
/// Exists so that gate reads the table rather than a copy of it.
pub fn classified_names() -> impl Iterator<Item = &'static str> {
    CLASSIFIED.iter().map(|(name, _)| *name)
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
        "`shall {}` took {:.1}s. A {} command is budgeted {}s — this one is over, which is the \
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
    let ceiling = shape.wave_ceiling(children);
    if waves > ceiling {
        faults.push(format!(
            "{} wave(s) over {} child command(s), against a ceiling of {} — the run went quiet \
             {} time(s), because something had to be answered before the next question could be \
             asked, and that is close to asking them one at a time",
            waves,
            children,
            ceiling,
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
        "`shall {}` asked every manager and did not overlap them: {}. The seconds a fan-out \
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

        // Real readings from three hosts, including the one that failed the first draft's
        // numbers: 2.0x against a floor of 2.0, and three waves against a ceiling of two, on a
        // runner doing nothing wrong.
        for (children, summed_ms, wall_ms, waves, host) in [
            (23, 23_670, 3_750, 2, "Windows, release: 6.3x"),
            (23, 32_620, 5_730, 2, "Windows, debug: 5.7x"),
            (
                16,
                8_440,
                4_270,
                3,
                "ubuntu-latest, CI run 31517073405: 2.0x",
            ),
            (9, 3_340, 1_760, 3, "macos-latest, CI run 31517073405: 1.9x"),
        ] {
            let healthy = shape_violation(
                shape,
                children,
                Duration::from_millis(summed_ms),
                Duration::from_millis(wall_ms),
                waves,
            );
            assert!(
                healthy.is_none(),
                "a measured healthy run is reported as a violation ({host}): {healthy:?}"
            );
        }

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
    fn the_two_classes_shall_controls_carry_a_budget_and_the_others_do_not() {
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
