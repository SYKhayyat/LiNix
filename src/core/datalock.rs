//! One writer at a time on the data directory (II.8).
//!
//! Shall is not the only thing that starts Shall: the package-manager hooks it installs
//! (`DPkg::Post-Invoke` and its siblings) spawn a reconcile on every ordinary `apt install`,
//! typed by someone who has never heard of this tool. `registry.json`, the journal and the
//! `locks/` ledgers are written whole, and two whole writes are last-one-wins — the entry
//! that loses is a managed package nothing declares, which is drift, and drift is removed.
//!
//! The lock covers the directory rather than one file: those files must agree with each
//! other, and a lock over one of a set that must agree is the same as no lock.

use crate::core::{Error, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a waiting run gives the holder before it says so instead.
///
/// 120s: long enough to outlast the longest wait a holder can legitimately make before it
/// starts doing work — the rate-limit ceiling, 30s by default — with room for the install it
/// then performs. It is not meant to outlast a whole sync: past this point the honest answer is
/// that someone else is writing, not a longer silence (S27).
pub const WAIT_SECS: u64 = 120;

/// How many `DataLock`s this process is holding.
///
/// **`flock` is per open file description, not per process**, so a second handle opened in a
/// process that already holds the lock does not re-enter — it waits for itself, for ever. Every
/// door that takes the lock counts here, and every door that might take it asks here first.
static HELD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Whether this process is inside the lock right now.
///
/// The question is dynamic and has to be: `LockScope::Deferred` takes the lock at each mutating
/// action and releases it in between, so no value carried around by the type system can say
/// whether it is held at the moment somebody writes.
pub fn held() -> bool {
    HELD.load(std::sync::atomic::Ordering::Acquire) > 0
}

/// The file that counts writers, so a reader can tell whether one moved underneath it.
const GENERATION_FILE: &str = "shall.gen";

/// What a reader saw of the writers, at one instant.
///
/// Two observations that compare equal, with no writer holding the lock at either, mean no
/// writer committed anything in between — which is what makes a multi-file read one moment
/// rather than several. See [`crate::core::stable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Generation {
    /// Bumped once by every writer that finishes.
    count: u64,
    /// Whether somebody held the lock as this was taken. A reader that saw a writer cannot
    /// conclude anything from two equal counts: the writer may not have released yet.
    writer_active: bool,
}

/// Read the writer generation. Two small reads of tiny files, and no lock of any kind — a
/// reader must never wait on a writer, which is the whole reason this exists.
pub fn observe(data_dir: &Path) -> Generation {
    let count = std::fs::read_to_string(data_dir.join(GENERATION_FILE))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    Generation {
        count,
        writer_active: data_dir.join("shall.lock.owner").exists(),
    }
}

impl Generation {
    /// Whether a read that spanned these two observations saw one moment.
    pub fn spans_one_moment(self, later: Self) -> bool {
        self == later && !self.writer_active
    }
}

/// Held for the mutating part of a command. Dropping it releases the lock.
pub struct DataLock {
    file: File,
    owner_path: PathBuf,
    data_dir: PathBuf,
}

impl DataLock {
    /// Take the lock from an `async` command, without parking a runtime worker.
    ///
    /// The wait below is `thread::sleep` in a poll loop for up to two minutes, and every caller
    /// is inside `#[tokio::main]`. `run_exclusive` already moved its `flock` wait to the blocking
    /// pool for exactly this reason and wrote down why; this is the same wait, one layer up,
    /// which nobody had noticed was the same.
    pub async fn acquire_async(data_dir: &Path, command: &str, timeout: Duration) -> Result<Self> {
        let dir = data_dir.to_path_buf();
        let command = command.to_string();
        crate::core::off_the_runtime(move || Self::acquire(&dir, &command, timeout)).await?
    }

    /// Take the lock if it is free at this instant, or report that somebody else holds it.
    ///
    /// **For a caller with nothing to do with the wait.** A `hook-*` subcommand is fired by a
    /// manager mid-transaction; if the directory is locked, the run holding it is the run that
    /// is going to record what the manager just did, so waiting two minutes to be told so
    /// costs the transaction two minutes and changes nothing. This returns `None` instead,
    /// and says nothing — contention is the ordinary case here, not a fault.
    pub fn try_acquire(data_dir: &Path, command: &str) -> Result<Option<Self>> {
        let (file, owner_path) = Self::open_lock_file(data_dir)?;
        if file.try_lock_exclusive().is_err() {
            return Ok(None);
        }
        Ok(Some(Self::stamped(file, owner_path, data_dir, command)))
    }

    /// Open the lock file and name its owner stamp. Shared so the waiting and non-waiting
    /// doors cannot disagree about which file in which directory is the lock.
    fn open_lock_file(data_dir: &Path) -> Result<(File, PathBuf)> {
        crate::utils::file::ensure_dir(data_dir)?;
        let path = data_dir.join("shall.lock");
        let owner_path = data_dir.join("shall.lock.owner");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(Error::from)?;
        Ok((file, owner_path))
    }

    /// Record who holds it. Written after the lock is taken, so the stamp cannot name a
    /// process that failed to get it.
    fn stamped(file: File, owner_path: PathBuf, data_dir: &Path, command: &str) -> Self {
        let stamp = format!("shall {} (pid {})", command, std::process::id());
        let _ = std::fs::write(&owner_path, stamp);
        HELD.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self {
            file,
            owner_path,
            data_dir: data_dir.to_path_buf(),
        }
    }

    /// Take the lock for one write, unless this process is already inside it.
    ///
    /// **The door for code that cannot know whether its caller holds the lock** — a ledger
    /// save reached from `sync` is covered by the run's own lock, and the same save reached
    /// from `check` is covered by nothing. Asking the caller to pass a token down twenty call
    /// sites answers this at compile time, which is the wrong time: `Deferred` releases the
    /// lock between actions, so the answer changes during a run.
    ///
    /// `Ok(None)` means the lock is already this process's and the caller writes as it is;
    /// re-taking it would be `flock` waiting for the same process's other handle, for ever.
    pub fn for_this_write(what: &str) -> Result<Option<Self>> {
        if held() {
            return Ok(None);
        }
        Self::acquire(
            &crate::utils::safe_data_dir(),
            what,
            Duration::from_secs(WAIT_SECS),
        )
        .map(Some)
    }

    /// Take the lock, waiting up to `timeout` for whoever holds it.
    ///
    /// Waiting with no reason given is indistinguishable from hanging, so the wait announces
    /// the holder — the lock file carries the pid and the command that took it.
    pub fn acquire(data_dir: &Path, command: &str, timeout: Duration) -> Result<Self> {
        let (file, owner_path) = Self::open_lock_file(data_dir)?;
        let path = data_dir.join("shall.lock");

        if file.try_lock_exclusive().is_err() {
            eprintln!(
                "shall: waiting for the data directory — held by {}",
                Self::holder(&owner_path)
            );
            let deadline = Instant::now() + timeout;
            loop {
                if file.try_lock_exclusive().is_ok() {
                    break;
                }
                if Instant::now() >= deadline {
                    // S27: the old text ended "remove shall.lock if nothing is running", and
                    // that advice is never right. The lock is an OS lock on an open handle,
                    // released when the holding process exits — so a lock that is still
                    // contended after the wait proves a live holder, and deleting the file
                    // takes the lock away from it rather than from a corpse.
                    return Err(Error::Other(format!(
                        "the Shall data directory is locked by {}, and still was after {}s.\n  \
                         {} is where state lives, and two writers make a removal out of a race.\n  \
                         The lock is held by a running process, not by the file: {} exists\n  \
                         between runs and deleting it would take the lock from a live writer.\n  \
                         Wait for that run to finish, or stop it.",
                        Self::holder(&owner_path),
                        timeout.as_secs(),
                        data_dir.display(),
                        path.display()
                    )));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        Ok(Self::stamped(file, owner_path, data_dir, command))
    }

    /// Take the lock for one mutating step of a command that does not hold it for its run.
    ///
    /// **The one place the wait and the directory are written down.** Three call sites had
    /// copied `safe_data_dir()`, `Duration::from_secs(120)` and the name-it-yourself argument,
    /// which is three chances for the wait to disagree with `main`'s and a fourth caller to
    /// invent a fifth number. `LockScope::Deferred` is what says a command belongs here.
    pub async fn for_one_step(what: &str) -> Result<Self> {
        Self::acquire_async(
            &crate::utils::safe_data_dir(),
            what,
            Duration::from_secs(WAIT_SECS),
        )
        .await
    }

    /// Take the lock for one step if it is free, standing down rather than waiting.
    ///
    /// Beside [`for_one_step`](Self::for_one_step) so the directory stays written down once:
    /// a caller that spelled `safe_data_dir()` itself would be the fourth copy the doc on
    /// that function is about.
    pub fn try_for_one_step(what: &str) -> Result<Option<Self>> {
        Self::try_acquire(&crate::utils::safe_data_dir(), what)
    }

    /// Who is holding the lock, for the message. The stamp lives beside the lock file rather
    /// than inside it: Windows refuses to read a file another process holds an exclusive lock
    /// on, which would leave the one message this exists to print saying nothing.
    fn holder(owner_path: &Path) -> String {
        match std::fs::read_to_string(owner_path) {
            Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => "another shall".to_string(),
        }
    }
}

impl Drop for DataLock {
    fn drop(&mut self) {
        // Bumped before the stamp goes and before the lock is released, so a reader that sees
        // no writer and an unchanged count is reading after this one's writes, never during.
        //
        // **A preview does not bump it**, and that is not an exemption from S25 but the rule
        // itself: this counter says "a writer committed something", and a run that wrote
        // nothing has nothing for a reader to detect. Writing it anyway would also be a
        // preview leaving a file behind, which is the defect the whole dry-run rule exists to
        // prevent — `a_preview_leaves_the_config_byte_identical` caught exactly that here.
        if !crate::core::dry_run::active() {
            let next = observe(&self.data_dir).count.wrapping_add(1);
            let _ = std::fs::write(self.data_dir.join(GENERATION_FILE), next.to_string());
        }
        HELD.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        let _ = std::fs::remove_file(&self.owner_path);
        // The lock file itself stays. Deleting it races the next process, which may already
        // have opened this inode and be about to lock a file no longer at that name.
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("shall-datalock-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_lock_is_taken_and_released_by_drop() {
        let dir = tmp("release");
        {
            let _held = DataLock::acquire(&dir, "sync", Duration::from_secs(1)).unwrap();
        }
        // The same process can take it again once the first guard is gone.
        let _again = DataLock::acquire(&dir, "plan", Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn the_lock_file_names_its_holder() {
        let dir = tmp("holder");
        let _held = DataLock::acquire(&dir, "sync", Duration::from_secs(1)).unwrap();
        let stamp = DataLock::holder(&dir.join("shall.lock.owner"));
        assert!(stamp.contains("sync"), "{}", stamp);
        assert!(
            stamp.contains(&std::process::id().to_string()),
            "a holder nobody can identify is the message this exists to avoid: {}",
            stamp
        );
    }

    /// An owner file with nothing in it names nobody, and must say so.
    ///
    /// Without `holder`'s `!s.trim().is_empty()` guard a blank file becomes the holder's name,
    /// so the contention message reads `waiting for ` with the sentence ending in air — the one
    /// message this whole owner file exists to print. It is reachable: `acquire` writes the lock
    /// file and the stamp as two steps, so a reader arriving between them, or after a crash
    /// between them, sees exactly this.
    ///
    /// Every shape that carries no name, not just the empty one — a file holding a newline is
    /// what a truncated write leaves behind, and it is the case a `.is_empty()` without the
    /// `trim()` would let through.
    #[test]
    fn an_owner_file_with_no_name_in_it_falls_back_rather_than_naming_nobody() {
        let dir = tmp("blank-owner");
        std::fs::create_dir_all(&dir).unwrap();
        let owner = dir.join("shall.lock.owner");

        for (label, body) in [("empty", ""), ("newline", "\n"), ("spaces", "   \t  \n")] {
            std::fs::write(&owner, body).unwrap();
            assert_eq!(
                DataLock::holder(&owner),
                "another shall",
                "an owner file that is {label} names nobody"
            );
        }

        // Absent is the same answer by a different route, and it is the branch the fallback was
        // written for — so it is asserted here rather than assumed.
        std::fs::remove_file(&owner).unwrap();
        assert_eq!(DataLock::holder(&owner), "another shall");

        // And the positive control: a file with a name in it still yields the name, trimmed.
        // Without this the assertions above pass against a `holder` that returns the fallback
        // unconditionally, which is the mutant one layer out.
        std::fs::write(&owner, "  pid 4242 running sync\n").unwrap();
        assert_eq!(DataLock::holder(&owner), "pid 4242 running sync");
    }

    #[test]
    fn a_second_holder_is_refused_with_who_holds_it_rather_than_hanging() {
        let dir = tmp("contended");
        let first = DataLock::acquire(&dir, "sync", Duration::from_secs(1)).unwrap();

        // A second *process* is what the lock is for; within one process the advisory lock
        // is not re-entrant on a separate handle either, which is what this asserts.
        let path = dir.join("shall.lock");
        let other = File::open(&path).unwrap();
        assert!(
            other.try_lock_exclusive().is_err(),
            "a second handle took a lock the first still holds"
        );
        drop(first);
        assert!(other.try_lock_exclusive().is_ok());
        let _ = FileExt::unlock(&other);
    }

    /// **A wait that is given time spends it, rather than refusing at once.**
    ///
    /// Found by the mutation gate: `acquire` computes its deadline as `now + timeout` and leaves
    /// the loop when `now >= deadline`, and BOTH of those survived being inverted — to `now -
    /// timeout` and to `now < deadline`. Either mutant puts the deadline in the past on the first
    /// iteration, so a contended lock returns the timeout error immediately instead of waiting.
    ///
    /// Nothing noticed, because every test above contends and then asserts on the *refusal*.
    /// Refusing is what `acquire` does at the END of the wait; not one test made it wait. So the
    /// whole point of the parameter — that a run started by a `DPkg::Post-Invoke` hook stands
    /// behind an ordinary `apt install` instead of failing under it — was unmeasured.
    #[test]
    fn a_contended_lock_is_waited_for_and_then_taken() {
        let dir = tmp("wait-succeeds");
        let held = DataLock::acquire(&dir, "holder", Duration::from_secs(5)).unwrap();

        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(600));
            drop(held);
        });

        let started = Instant::now();
        let taken = DataLock::acquire(&dir, "waiter", Duration::from_secs(30));
        let waited = started.elapsed();
        releaser.join().expect("the holder thread panicked");

        assert!(
            taken.is_ok(),
            "the holder released well inside the timeout and the wait still failed: {:?}",
            taken.err()
        );
        // The half that kills the arithmetic mutants: a deadline in the past would have come
        // back at once with the error above, and a wait that returns instantly is not a wait.
        assert!(
            waited >= Duration::from_millis(400),
            "took the lock after {waited:?}, which is sooner than the holder released it — the \
             wait did not happen"
        );
    }

    /// And a wait that runs out runs out *after* the time it was given, not before.
    ///
    /// The other side of the same two mutants, and the one that pins the number rather than the
    /// behaviour: without it, `acquire` could satisfy the test above by waiting a fixed instant
    /// and ignoring `timeout` entirely.
    #[test]
    fn a_wait_that_runs_out_first_spends_the_time_it_was_given() {
        let dir = tmp("wait-expires");
        let _held = DataLock::acquire(&dir, "holder", Duration::from_secs(5)).unwrap();

        let started = Instant::now();
        // Matched rather than `expect_err`, which would want `DataLock: Debug` — a derive on a
        // production type to satisfy a test is the test choosing what the type looks like.
        let outcome = DataLock::acquire(&dir, "waiter", Duration::from_millis(700));
        let waited = started.elapsed();
        let Err(err) = outcome else {
            panic!("the holder never let go, so the wait must fail — it succeeded after {waited:?}")
        };

        assert!(
            waited >= Duration::from_millis(450),
            "gave up after {waited:?} on a 700ms timeout"
        );
        // And it still says who, because a timeout that names nobody is the sentence this file
        // exists to print going missing at the one moment it is read.
        assert!(
            err.to_string().contains("holder"),
            "the timeout does not name the holder: {err}"
        );
    }

    /// The counter that lets a reader detect a writer without waiting for one. It moves when a
    /// writer *finishes*, so an unchanged count with no holder means the reader is strictly
    /// after that writer rather than inside it.
    #[test]
    fn a_writer_that_finishes_moves_the_generation() {
        let dir = tmp("generation");
        let before = observe(&dir);

        {
            let _held = DataLock::acquire(&dir, "writer", Duration::from_secs(1)).unwrap();
            let during = observe(&dir);
            assert_eq!(
                during.count, before.count,
                "the count must not move while the writer is still going"
            );
            assert!(
                during.writer_active,
                "and the reader must be able to see it"
            );
            assert!(
                !before.spans_one_moment(during),
                "a read that started before this writer and ended inside it saw two moments"
            );
        }

        let after = observe(&dir);
        assert_eq!(after.count, before.count.wrapping_add(1));
        assert!(!after.writer_active);
        assert!(
            !before.spans_one_moment(after),
            "a writer committed in between, so the reader must read again"
        );
        assert!(
            after.spans_one_moment(observe(&dir)),
            "and a quiet moment compares equal to itself"
        );
    }

    /// The re-entrancy that stops the process waiting for itself.
    ///
    /// `flock` is per open file description: a second handle opened by a process that already
    /// holds the lock blocks until the first is released, which is never, because the code that
    /// would release it is waiting.
    #[test]
    fn a_process_inside_the_lock_does_not_take_it_again() {
        let dir = tmp("reentrant");
        // `HELD` is process-wide and this suite runs its tests in parallel, so the only honest
        // assertions here are about what this test's own lock does — not about the count being
        // zero, which a sibling test holding a lock of its own would make false.
        let outer = DataLock::acquire(&dir, "outer", Duration::from_secs(1)).unwrap();
        assert!(held(), "this process is inside a lock it took");

        let inner = DataLock::for_this_write("a ledger").unwrap();
        assert!(
            inner.is_none(),
            "taking it a second time in one process is how this deadlocks"
        );

        drop(outer);
    }
}
