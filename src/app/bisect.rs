// src/app/bisect.rs
//
// System time-travel bisect: given the ordered list of snapshots (oldest -> newest) and a
// test command whose success means "good", find the first snapshot where the test fails —
// i.e. the change that introduced a regression.
//
// The binary-search core (`first_bad`) is a pure function so it can be unit-tested against
// a synthetic oracle. The command wraps it with a real oracle that restores each candidate
// snapshot and runs the user's test. Filesystem-restore backends (btrfs/timeshift) may
// require a reboot to activate a restore, so this fully automates only where restore is
// immediate (e.g. ZFS rollback); elsewhere it clearly reports the step it reached.

use crate::app::App;
use crate::core::{Error, Result};
use tracing::{info, warn};

/// Classic bisection: over `len` items ordered oldest→newest where `is_good(i)` is
/// monotonic (all-good then all-bad), return the index of the FIRST bad item, or None if
/// every item is good. `is_good` is only called O(log n) times.
pub fn first_bad<F: FnMut(usize) -> bool>(len: usize, mut is_good: F) -> Option<usize> {
    if len == 0 {
        return None;
    }
    // Invariant: everything < lo is good; everything >= hi is bad (once found).
    let (mut lo, mut hi) = (0usize, len);
    let mut found = None;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if is_good(mid) {
            lo = mid + 1;
        } else {
            found = Some(mid);
            hi = mid;
        }
    }
    found
}

/// Run a shell command and return whether it succeeded (exit 0). Cross-platform.
pub async fn run_test(cmd: &str) -> bool {
    let mut command = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    matches!(command.status().await, Ok(s) if s.success())
}

/// Drive a bisect across the machine's snapshots using `test` as the good/bad oracle.
pub async fn bisect(app: &App, test: &str, assume_yes: bool) -> Result<()> {
    if !app.snapshot_manager.has_provider() {
        return Err(Error::Snapshot(
            "bisect needs a snapshot provider (btrfs/zfs/timeshift/Windows Restore); none is available".into(),
        ));
    }

    let mut snapshots = app.snapshot_manager.list_snapshots().await?;
    if snapshots.len() < 2 {
        println!(
            "Need at least 2 snapshots to bisect; found {}.",
            snapshots.len()
        );
        return Ok(());
    }
    // Oldest -> newest so the first failing index is the change that introduced the break.
    snapshots.sort_by_key(|s| s.parse_time());

    println!(
        "Bisecting {} snapshots with test: {}",
        snapshots.len(),
        test
    );
    println!("First, confirming the problem reproduces on the CURRENT system...");
    if run_test(test).await {
        println!("Test passes right now — nothing to bisect (the system is currently good).");
        return Ok(());
    }

    if app.config.dry_run {
        println!("[dry-run] Would binary-search snapshots (restoring + testing each) to find the culprit.");
        return Ok(());
    }
    if !assume_yes {
        warn!("Bisect will RESTORE snapshots on this machine. Re-run with --yes to proceed.");
        return Ok(());
    }

    // Adaptive binary search. We mirror `first_bad` but the oracle is async (restore+test).
    let (mut lo, mut hi) = (0usize, snapshots.len());
    let mut culprit = None;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let snap = &snapshots[mid];
        info!(
            "Bisect: restoring snapshot {} ({}) and testing...",
            snap.id, snap.timestamp
        );
        app.snapshot_manager.restore_snapshot(&snap.id).await?;
        let good = run_test(test).await;
        println!(
            "  {} @ {} -> {}",
            snap.id,
            snap.timestamp,
            if good { "GOOD" } else { "BAD" }
        );
        if good {
            lo = mid + 1;
        } else {
            culprit = Some(mid);
            hi = mid;
        }
    }

    match culprit {
        Some(i) => {
            let s = &snapshots[i];
            println!("\nFirst broken snapshot: {} ({})", s.id, s.timestamp);
            if i > 0 {
                let prev = &snapshots[i - 1];
                println!("Last good snapshot:    {} ({})", prev.id, prev.timestamp);
                println!("=> The regression was introduced between these two states.");
            } else {
                println!(
                    "=> The oldest snapshot is already broken; the cause predates your history."
                );
            }
        }
        None => println!("\nNo broken snapshot found — every restored state passed the test."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_bad_finds_transition() {
        // good,good,good,BAD,BAD  -> first bad = 3
        let states = [true, true, true, false, false];
        assert_eq!(first_bad(states.len(), |i| states[i]), Some(3));
    }

    #[test]
    fn first_bad_all_good_is_none() {
        let states = [true, true, true];
        assert_eq!(first_bad(states.len(), |i| states[i]), None);
    }

    #[test]
    fn first_bad_all_bad_is_zero() {
        let states = [false, false, false];
        assert_eq!(first_bad(states.len(), |i| states[i]), Some(0));
    }

    #[test]
    fn first_bad_is_logarithmic() {
        // Ensure the oracle is called O(log n) times, not O(n).
        let n = 1024;
        let mut calls = 0;
        let idx = first_bad(n, |i| {
            calls += 1;
            i < 700
        });
        assert_eq!(idx, Some(700));
        assert!(
            calls <= 11,
            "expected <= ceil(log2(1024))+1 calls, got {}",
            calls
        );
    }

    #[test]
    fn first_bad_empty_is_none() {
        assert_eq!(first_bad(0, |_| true), None);
    }
}
