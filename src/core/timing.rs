//! Where a run's seconds went, per child command.
//!
//! [`latency`](crate::core::latency) measures the *total* and says when it crosses a budget.
//! That is enough to notice a 98-second `info` and not enough to act on one: the next question
//! is always which manager took the time, and until this existed the only way to answer it was
//! to time the managers by hand outside LiNix and subtract.
//!
//! **Sum and wall clock are both reported, because their ratio is the parallelism.** LiNix
//! spends its life waiting on other people's processes, so a run whose child time sums to 6 s
//! inside a 3.4 s wall clock is overlapping them 1.8×, and one whose ratio is 1.0 is asking
//! them one at a time. A breakdown that printed only the sum would hide the difference the
//! whole design turns on.
//!
//! Recording is off unless `--timings` asks for it: the lock is uncontended and the cost is a
//! `String` per child, but a measurement nobody requested is exactly the eager work this
//! module exists to find.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

/// When this process began, so every span can be placed on one timeline.
static START: Lazy<Instant> = Lazy::new(Instant::now);

static ENABLED: AtomicBool = AtomicBool::new(false);

static SPANS: Lazy<Mutex<Vec<Span>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// One child command, and when it ran relative to the start of the process.
struct Span {
    label: String,
    at: Duration,
    took: Duration,
}

/// Start recording. Called once, before anything is dispatched.
pub fn enable() {
    Lazy::force(&START);
    ENABLED.store(true, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// How long since recording began — the run's wall clock, near enough.
///
/// Measured from [`enable`] rather than from the dispatch of the verb, because config loading
/// and the model resolution happen before that and are exactly the part a user who asks "why
/// did it sit there doing nothing" is asking about.
pub fn elapsed() -> Duration {
    START.elapsed()
}

/// Mark the start of a child command. Returns `None` when recording is off, so a caller that
/// is not being measured allocates nothing.
pub fn begin() -> Option<Instant> {
    is_enabled().then(Instant::now)
}

/// Record a finished child command.
///
/// `label` is the program and its first argument — `winget list`, not the whole argv. The
/// package names on an `install` line are the part that differs between two runs of the same
/// command, and a breakdown keyed by them would have one row per package instead of one row
/// per thing the run actually waited on.
pub fn end(started: Option<Instant>, cmd: &str, args: &[String]) {
    let Some(started) = started else { return };
    record(
        cmd,
        args,
        started.saturating_duration_since(*START),
        started.elapsed(),
    );
}

/// The label a program and its argv are gathered under.
fn label_of(cmd: &str, args: &[String]) -> String {
    match args.first() {
        Some(first) => format!("{} {}", cmd, first),
        None => cmd.to_string(),
    }
}

/// Record a span whose start and duration are given rather than measured.
///
/// **The seam that makes this module testable without a clock.** Its aggregation — the label
/// keying, the slowest-first order, the wave count — is arithmetic, and a test that produced
/// its inputs with `thread::sleep` was asserting on the scheduler instead: two 5 ms sleeps sum
/// past one 30 ms sleep whenever Windows' 15.6 ms timer granularity and a loaded machine
/// conspire, which is precisely when the suite runs (AU5). Timing is now measured in exactly
/// one place — [`end`] — and tested with one loose assertion that cannot order-invert.
fn record(cmd: &str, args: &[String], at: Duration, took: Duration) {
    if let Ok(mut spans) = SPANS.lock() {
        spans.push(Span {
            label: label_of(cmd, args),
            at,
            took,
        });
    }
}

/// What a run spent, gathered per label.
pub struct Row {
    pub label: String,
    pub calls: usize,
    pub total: Duration,
    pub longest: Duration,
    /// When the first call started, measured from [`enable`].
    ///
    /// Aggregating durations answers *what* was slow and cannot answer *whether it waited*.
    /// Two runs with identical rows — one overlapping every child, one running them in file
    /// order — are the same table until the start offsets are printed beside it.
    pub first_at: Duration,
    /// When the last call finished.
    pub last_end: Duration,
}

/// The rows, slowest first, plus the wall clock and the summed child time.
///
/// Returned rather than printed so a test can assert on it without capturing stderr.
pub fn summary() -> (Vec<Row>, Duration, Duration) {
    let spans = match SPANS.lock() {
        Ok(s) => s,
        Err(poisoned) => poisoned.into_inner(),
    };

    let mut rows: Vec<Row> = Vec::new();
    let mut summed = Duration::ZERO;
    for span in spans.iter() {
        summed += span.took;
        match rows.iter_mut().find(|r| r.label == span.label) {
            Some(row) => {
                row.calls += 1;
                row.total += span.took;
                row.longest = row.longest.max(span.took);
                row.first_at = row.first_at.min(span.at);
                row.last_end = row.last_end.max(span.at + span.took);
            }
            None => rows.push(Row {
                label: span.label.clone(),
                calls: 1,
                total: span.took,
                longest: span.took,
                first_at: span.at,
                last_end: span.at + span.took,
            }),
        }
    }
    rows.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.label.cmp(&b.label)));

    // The last child to finish, not the process's own elapsed time: this is called from
    // `main` before the exit path, and the caller's own wall clock is the honest total.
    let span_end = spans
        .iter()
        .map(|s| s.at + s.took)
        .max()
        .unwrap_or_default();
    (rows, span_end, summed)
}

/// How many times the run went completely quiet — no child running at all — and started again.
///
/// One wave means everything overlapped. `n` waves means the run stopped `n - 1` times to wait
/// for an answer before it could ask the next question, and that is the difference between a
/// low overlap ratio caused by *slow* children and one caused by *sequenced* ones. Counted here
/// rather than eyeballed off the start offsets, because that is the reading everyone gets wrong.
pub fn waves() -> usize {
    let spans = match SPANS.lock() {
        Ok(s) => s,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut intervals: Vec<(Duration, Duration)> =
        spans.iter().map(|s| (s.at, s.at + s.took)).collect();
    intervals.sort_by_key(|(at, _)| *at);

    let mut waves = 0;
    let mut open_until = Duration::ZERO;
    for (at, end) in intervals {
        // `>=`, not `>`: a child that starts exactly as the last one ends did wait for it.
        if at >= open_until {
            waves += 1;
            open_until = end;
        } else {
            open_until = open_until.max(end);
        }
    }
    waves
}

/// Print the breakdown to stderr.
///
/// stderr, not stdout: `linix eval --timings | jq` must still get JSON, and a measurement
/// written into the answer is a measurement that breaks every caller parsing it.
pub fn report(wall: Duration) {
    if !is_enabled() {
        return;
    }
    let (rows, _, summed) = summary();

    if rows.is_empty() {
        eprintln!(
            "\nTimings: {:.2}s wall, no child commands — this run asked no package manager \
             anything.",
            wall.as_secs_f64()
        );
        return;
    }

    let calls: usize = rows.iter().map(|r| r.calls).sum();
    // Below 1.0 only when the children were genuinely serial and the run spent time outside
    // them; it is a ratio of sums, so it never divides by zero here (rows is non-empty).
    let overlap = summed.as_secs_f64() / wall.as_secs_f64().max(f64::EPSILON);

    let waves = waves();
    eprintln!(
        "\nTimings: {:.2}s wall · {} child command(s) summing to {:.2}s · {:.1}x overlap · {} wave(s)",
        wall.as_secs_f64(),
        calls,
        summed.as_secs_f64(),
        overlap,
        waves,
    );
    if waves > 1 {
        eprintln!(
            "  {} wave(s) means the run went quiet {} time(s) — nothing was running, because \
             something had to be answered before the next question could be asked.",
            waves,
            waves - 1,
        );
    }
    eprintln!("  (only commands LiNix spawns are counted; its own parsing is the remainder)");
    eprintln!("  {:>7}  {:>7}   command", "at", "took");
    for row in &rows {
        let calls = if row.calls == 1 {
            String::new()
        } else {
            format!(
                "  ({}x, longest {:.2}s, last ended {:.2}s)",
                row.calls,
                row.longest.as_secs_f64(),
                row.last_end.as_secs_f64(),
            )
        };
        eprintln!(
            "  {:>6.2}s  {:>6.2}s   {}{}",
            row.first_at.as_secs_f64(),
            row.total.as_secs_f64(),
            row.label,
            calls,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorder is global, so these run as one test rather than racing each other over it.
    #[test]
    fn a_run_is_gathered_per_label_slowest_first_and_off_by_default() {
        // Off by default: `begin` hands back nothing, and `end` records nothing from it.
        assert!(!is_enabled());
        assert!(begin().is_none());
        end(None, "winget", &["list".to_string()]);
        assert!(
            summary().0.is_empty(),
            "a disabled recorder recorded a span"
        );

        enable();
        assert!(is_enabled());

        // Given, not slept for. The durations below are the test's subject matter; producing
        // them with a sleep made the assertions a measurement of the host's scheduler.
        let ms = Duration::from_millis;
        record("winget", &["list".to_string()], ms(0), ms(30));
        record(
            "cargo",
            &["install".to_string(), "--list".to_string()],
            ms(30),
            ms(5),
        );
        record(
            "cargo",
            &["install".to_string(), "--list".to_string()],
            ms(35),
            ms(5),
        );
        // No args at all still names the program, rather than producing an empty row.
        record("emacs", &[], ms(40), ms(1));

        let (rows, _, summed) = summary();
        assert_eq!(rows.len(), 3, "three distinct labels, not one row per call");

        // Slowest first.
        assert_eq!(rows[0].label, "winget list");
        assert_eq!(rows[0].calls, 1);

        // Keyed by program + FIRST arg, so `--list` does not open a second row, and the two
        // calls gather into one.
        let cargo = rows
            .iter()
            .find(|r| r.label == "cargo install")
            .expect("cargo install row");
        assert_eq!(cargo.calls, 2);
        assert!(
            cargo.total >= cargo.longest,
            "total must include the longest"
        );

        assert!(rows.iter().any(|r| r.label == "emacs"));

        assert!(summed >= rows[0].total, "the sum covers every row");

        // Every span above ran to completion before the next began, so each is its own wave.
        // This is the reading that matters: same rows, same totals, and a serial run is only
        // distinguishable from an overlapped one here.
        assert_eq!(waves(), 4, "four children, none overlapping, is four waves");

        let winget = rows.iter().find(|r| r.label == "winget list").unwrap();
        assert!(
            winget.last_end >= winget.first_at + winget.longest,
            "a row's span must cover its own longest call"
        );

        end_records_the_time_that_actually_passed();
    }

    /// The half [`record`] cannot cover: that [`end`] measures a real elapsed interval.
    ///
    /// One span, one loose lower bound, nothing compared against another span — so there is no
    /// ordering for a busy scheduler to invert. A 30 ms sleep that records less than 10 ms is
    /// not a slow machine, it is a broken clock.
    ///
    /// **Last, and inside the same test.** The recorder is process-global; a second `#[test]`
    /// would run on another thread and add a row to the table the assertions above are
    /// counting.
    fn end_records_the_time_that_actually_passed() {
        let started = begin();
        assert!(started.is_some(), "recording is on");
        std::thread::sleep(Duration::from_millis(30));
        end(started, "linix-timing-probe", &["sleep".to_string()]);

        let (rows, _, _) = summary();
        let row = rows
            .iter()
            .find(|r| r.label == "linix-timing-probe sleep")
            .expect("the span was recorded");
        assert!(
            row.total >= Duration::from_millis(10),
            "a 30ms child was recorded as {:?}",
            row.total
        );
    }
}
