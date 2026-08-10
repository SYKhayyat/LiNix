// src/app/bisect.rs
//
// Snapshot bisect: given the ordered list of snapshots (oldest -> newest) and a
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
    let command = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    // Supervised: the oracle is a command the user typed, and a bisect that hangs on it hangs
    // with a snapshot half-restored. Its output is captured rather than printed — a bisect runs
    // this many times, and the answer that matters is the exit status.
    matches!(
        crate::core::executor::supervised_output(command, "the bisect test", true).await,
        Ok(o) if o.status.success()
    )
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
        crate::would_print!(
            "Would binary-search snapshots (restoring + testing each) to find the culprit."
        );
        return Ok(());
    }
    if !assume_yes {
        warn!("Bisect will RESTORE snapshots on this machine. Re-run with --yes to proceed.");
        return Ok(());
    }

    // **Where the machine started, so it can be put back.**
    //
    // `bisect` is a diagnostic: it answers "which change broke this?". It used to answer by
    // restoring snapshot after snapshot and then returning `Ok(())` from wherever the search
    // happened to stop — not the culprit's state, not the last good one, just whichever
    // candidate the final iteration probed. The machine's installed software was rearranged
    // into an arbitrary historical state, silently, by a command whose whole purpose is to
    // *tell you something*. Same family as `profile show`, which resolved a question by editing
    // `active`; the difference is only that this one changes the machine rather than a file.
    //
    // Taken before the first restore, so it captures the state the user was in, and restored on
    // every exit from the loop including the error one.
    let home = app
        .snapshot_manager
        .auto_snapshot(crate::core::snapshot::SnapshotLabel::PreBisect)
        .await?;
    match &home {
        Some(s) => info!(
            "Bisect: snapshot {} records the current state; it will be restored when the search \
             ends.",
            s.id
        ),
        // `has_provider` is checked above, so this is a provider that answered and produced
        // nothing. Said out loud rather than assumed away: it is the difference between a
        // diagnostic that gives you your machine back and one that does not.
        None => warn!(
            "Bisect: the snapshot provider took no snapshot of the current state, so this \
             machine will be left on whichever snapshot the search ends at. Restore it yourself \
             with `linix rollback` when you are done."
        ),
    }

    // Adaptive binary search. We mirror `first_bad` but the oracle is async (restore+test).
    let search = search_for_culprit(app, test, &snapshots).await;

    if let Some(s) = &home {
        info!("Bisect: restoring {} — the state you started in.", s.id);
        // Reported, not swallowed, and not allowed to hide the search's own error: a machine
        // left on a historical snapshot is the one outcome the user must not learn about later.
        if let Err(e) = app.snapshot_manager.restore_snapshot(&s.id).await {
            warn!(
                "Bisect: could not restore {}: {}. This machine is still on a snapshot the \
                 search restored.",
                s.id, e
            );
        }
    }

    match search? {
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

/// The binary search itself: restore a candidate, ask the oracle, narrow.
///
/// **Split out so its `?` cannot skip the restore.** With the loop inline, a restore that failed
/// halfway propagated straight out of `bisect` and the machine was left on the last snapshot the
/// search had reached — the failure mode of the original, made *worse* by an error path. The
/// caller holds the result until after it has put the machine back.
async fn search_for_culprit(
    app: &App,
    test: &str,
    snapshots: &[crate::core::snapshot::Snapshot],
) -> Result<Option<usize>> {
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
    Ok(culprit)
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
