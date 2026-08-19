//! `[journal] flush_every`: what a completion costs, and what a crash costs instead.
//!
//! The *opening* half of the write-ahead log is not negotiable and is not tested here — a record
//! that a package is about to be touched reaches the disk before the manager is invoked, because
//! recovery cannot replay work it has no record of starting. `record_starts` already batches a
//! whole wave into one flush without weakening that.
//!
//! The closing half is the trade this setting names. Buffering a completion means a crash in the
//! window leaves an entry reading in-progress, and the next run re-installs a package that is
//! already installed — which is what `heal` does anyway. **That is the property these tests pin:
//! not that the buffer is fast, but that what the buffer loses is recoverable.**

use shall::core::journal::{ActionStatus, Journal, JournalAction};
use shall::core::PackageSpec;

fn install_of(name: &str) -> JournalAction {
    JournalAction::Install(PackageSpec {
        name: name.to_string(),
        backend: "apt".to_string(),
        ..Default::default()
    })
}

/// What another process would see, which is the only view a crash leaves behind.
fn on_disk(path: &std::path::Path) -> Journal {
    Journal::at(path.to_path_buf()).expect("the log re-reads")
}

fn temp_log(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(format!("{name}.jsonl"));
    (dir, path)
}

/// `flush_every = 1` is the strict setting, and it must mean exactly what it did before the
/// buffer existed: the completion is on disk when the call returns.
#[test]
fn one_means_every_completion_reaches_the_disk_at_once() {
    let (_dir, path) = temp_log("strict");
    let mut journal = Journal::at(path.clone()).unwrap();
    journal.set_buffer_limit(1);

    let id = journal.record_start(install_of("curl")).unwrap();
    journal.record_success(&id).unwrap();

    assert_eq!(journal.unflushed(), 0, "nothing may be left waiting");
    assert_eq!(
        on_disk(&path).entries.get(&id).map(|e| e.status),
        Some(ActionStatus::Completed),
        "another process must be able to see it"
    );
}

/// Zero is the value someone writes meaning "never flush". It is the one answer the buffer must
/// not be able to express, so it reads as one.
#[test]
fn zero_is_not_a_way_to_switch_the_log_off() {
    let (_dir, path) = temp_log("zero");
    let mut journal = Journal::at(path.clone()).unwrap();
    journal.set_buffer_limit(0);

    let id = journal.record_start(install_of("curl")).unwrap();
    journal.record_success(&id).unwrap();

    assert_eq!(journal.unflushed(), 0);
    assert_eq!(
        on_disk(&path).entries.get(&id).map(|e| e.status),
        Some(ActionStatus::Completed)
    );
}

/// The buffer is invisible from inside the process: `heal` and `needs_recovery` read the map,
/// so a completion is complete the moment it is recorded no matter what the setting says.
#[test]
fn a_buffered_completion_is_already_true_in_this_process() {
    let (_dir, path) = temp_log("in-memory");
    let mut journal = Journal::at(path).unwrap();
    journal.set_buffer_limit(32);

    let ids = journal
        .record_starts(vec![install_of("curl"), install_of("jq")])
        .unwrap();
    for id in &ids {
        journal.record_success(id).unwrap();
    }

    assert_eq!(journal.unflushed(), 2, "held, not written");
    assert!(
        !journal.needs_recovery(),
        "nothing is interrupted — the work finished, the disk just does not know yet"
    );
    assert!(journal.interrupted_actions().is_empty());
}

/// And this is the cost, stated rather than implied: a crash before the flush leaves the entry
/// reading in-progress, which is precisely the input `heal` is built to take.
#[test]
fn what_a_crash_loses_is_a_re_run_and_not_a_record() {
    let (_dir, path) = temp_log("crash");
    let id = {
        let mut journal = std::mem::ManuallyDrop::new(Journal::at(path.clone()).unwrap());
        journal.set_buffer_limit(32);
        let id = journal.record_start(install_of("curl")).unwrap();
        journal.record_success(&id).unwrap();
        // `ManuallyDrop` is the crash: the process stops here, so the `Drop` flush never runs.
        id
    };

    let survivor = on_disk(&path);
    assert_eq!(
        survivor.entries.get(&id).map(|e| e.status),
        Some(ActionStatus::InProgress),
        "the completion is gone, as designed"
    );
    assert!(
        survivor.needs_recovery(),
        "and what is left is an entry recovery knows how to finish"
    );
    assert_eq!(
        survivor.interrupted_actions().len(),
        1,
        "one idempotent re-install, which is the whole price"
    );
}

/// A clean exit is not a crash. Every path that finishes a unit of work flushes explicitly;
/// `Drop` is what makes the ones nobody has written yet safe.
#[test]
fn a_journal_that_goes_out_of_scope_takes_its_buffer_with_it() {
    let (_dir, path) = temp_log("drop");
    let id = {
        let mut journal = Journal::at(path.clone()).unwrap();
        journal.set_buffer_limit(32);
        let id = journal.record_start(install_of("curl")).unwrap();
        journal.record_success(&id).unwrap();
        assert_eq!(journal.unflushed(), 1);
        id
    };

    assert_eq!(
        on_disk(&path).entries.get(&id).map(|e| e.status),
        Some(ActionStatus::Completed),
        "returning is not crashing"
    );
}

/// The batch never straddles a wave. Opening wave two writes wave one's completions first, so
/// the file read forward is the run in the order it happened.
#[test]
fn opening_a_wave_carries_the_previous_wave_down_with_it() {
    let (_dir, path) = temp_log("waves");
    let mut journal = Journal::at(path.clone()).unwrap();
    journal.set_buffer_limit(1000);

    let first = journal.record_starts(vec![install_of("curl")]).unwrap();
    journal.record_success(&first[0]).unwrap();
    assert_eq!(
        journal.unflushed(),
        1,
        "held while the wave is the last one"
    );

    let second = journal.record_starts(vec![install_of("jq")]).unwrap();
    assert_eq!(journal.unflushed(), 0, "and released when the next opens");

    let seen = on_disk(&path);
    assert_eq!(
        seen.entries.get(&first[0]).map(|e| e.status),
        Some(ActionStatus::Completed)
    );
    assert_eq!(
        seen.entries.get(&second[0]).map(|e| e.status),
        Some(ActionStatus::InProgress),
        "the wave that is still running still reads as running"
    );
}

/// The limit is a limit: the flush happens on the completion that reaches it, not at some later
/// convenient moment.
#[test]
fn the_buffer_flushes_when_it_is_full_and_not_before() {
    let (_dir, path) = temp_log("full");
    let mut journal = Journal::at(path.clone()).unwrap();
    journal.set_buffer_limit(3);

    let ids = journal
        .record_starts(vec![
            install_of("a"),
            install_of("b"),
            install_of("c"),
            install_of("d"),
        ])
        .unwrap();

    journal.record_success(&ids[0]).unwrap();
    journal.record_success(&ids[1]).unwrap();
    assert_eq!(journal.unflushed(), 2, "two of three");
    assert_eq!(
        on_disk(&path).entries[&ids[0]].status,
        ActionStatus::InProgress,
        "nothing has reached the disk yet"
    );

    journal.record_success(&ids[2]).unwrap();
    assert_eq!(journal.unflushed(), 0, "the third one pays for all three");
    let seen = on_disk(&path);
    for id in &ids[..3] {
        assert_eq!(seen.entries[id].status, ActionStatus::Completed);
    }
    assert_eq!(
        seen.entries[&ids[3]].status,
        ActionStatus::InProgress,
        "and only those three"
    );
}

/// A failure is a completion for this purpose — the same buffer, the same flush, the same error
/// text preserved through it. The parallel branch is where a fix to one half gets forgotten.
#[test]
fn a_failure_is_buffered_and_flushed_exactly_like_a_success() {
    let (_dir, path) = temp_log("failure");
    let mut journal = Journal::at(path.clone()).unwrap();
    journal.set_buffer_limit(2);

    let ids = journal
        .record_starts(vec![install_of("curl"), install_of("jq")])
        .unwrap();
    journal
        .record_failure(&ids[0], "the archive said no")
        .unwrap();
    assert_eq!(journal.unflushed(), 1);
    journal.record_success(&ids[1]).unwrap();
    assert_eq!(journal.unflushed(), 0);

    let seen = on_disk(&path);
    assert_eq!(seen.entries[&ids[0]].status, ActionStatus::Failed);
    assert_eq!(
        seen.entries[&ids[0]].error.as_deref(),
        Some("the archive said no"),
        "the reason survives the buffer"
    );
    assert_eq!(seen.entries[&ids[1]].status, ActionStatus::Completed);
}

/// Maintenance rewrites the file from the in-memory entries, which already contain the buffered
/// transitions. If the buffer were not cleared by that rewrite, the next flush would append the
/// very lines the rewrite existed to remove — including, when the rewrite emptied the file, lines
/// belonging to entries that no longer exist.
#[test]
fn a_rewrite_does_not_leave_the_buffer_to_undo_it() {
    let (_dir, path) = temp_log("compact");
    let mut journal = Journal::at(path.clone()).unwrap();
    journal.set_buffer_limit(1000);

    let ids = journal
        .record_starts(vec![install_of("curl"), install_of("jq")])
        .unwrap();
    for id in &ids {
        journal.record_success(id).unwrap();
    }
    assert_eq!(journal.unflushed(), 2);

    // Age one of them past the purge horizon. Maintenance only rewrites when it has something
    // to drop, so without this it appends nothing and the property under test never runs —
    // which is how the first version of this test passed for the wrong reason.
    let ancient = chrono::Utc::now().timestamp() - 60 * 60 * 24 * 30;
    let doomed = ids[0].clone();
    if let Some(entry) = journal.entries.get_mut(&doomed) {
        entry.started_at_unix = ancient;
        entry.finished_at_unix = Some(ancient);
    }

    assert!(
        journal.cleanup_expired_logs(7).unwrap(),
        "the aged entry must actually be purged, or this proves nothing"
    );
    assert_eq!(
        journal.unflushed(),
        0,
        "the rewrite wrote what survived; nothing may be owed afterwards"
    );
    assert!(!journal.entries.contains_key(&doomed));

    drop(journal);
    let seen = on_disk(&path);
    assert!(
        !seen.entries.contains_key(&doomed),
        "a flush that still thought it owed the purged entry would put it straight back"
    );
    assert_eq!(
        seen.entries.get(&ids[1]).map(|e| e.status),
        Some(ActionStatus::Completed),
        "and the survivor is recorded once, by the rewrite"
    );
    let lines = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        lines.lines().filter(|l| !l.trim().is_empty()).count(),
        1,
        "one surviving entry, one line"
    );
}

/// The other half: ordinary maintenance that drops nothing rewrites nothing, so the buffer is
/// still owed afterwards — and `Drop` is what settles it. An append-only log read forward takes
/// the last record per id, so the completion landing after the opening line is the answer.
#[test]
fn maintenance_that_drops_nothing_leaves_the_buffer_to_the_flush() {
    let (_dir, path) = temp_log("no-compact");
    let mut journal = Journal::at(path.clone()).unwrap();
    journal.set_buffer_limit(1000);

    let ids = journal
        .record_starts(vec![install_of("curl"), install_of("jq")])
        .unwrap();
    for id in &ids {
        journal.record_success(id).unwrap();
    }

    journal.cleanup().unwrap();
    assert_eq!(
        journal.unflushed(),
        2,
        "nothing was purged, so nothing was rewritten"
    );

    drop(journal);
    let seen = on_disk(&path);
    for id in &ids {
        assert_eq!(
            seen.entries.get(id).map(|e| e.status),
            Some(ActionStatus::Completed),
            "last record per id wins, and the completion is the last one"
        );
    }
}
