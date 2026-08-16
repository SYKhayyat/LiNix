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

/// Held for the mutating part of a command. Dropping it releases the lock.
pub struct DataLock {
    file: File,
    owner_path: PathBuf,
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

    /// Take the lock, waiting up to `timeout` for whoever holds it.
    ///
    /// Waiting with no reason given is indistinguishable from hanging, so the wait announces
    /// the holder — the lock file carries the pid and the command that took it.
    pub fn acquire(data_dir: &Path, command: &str, timeout: Duration) -> Result<Self> {
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

        let stamp = format!("shall {} (pid {})", command, std::process::id());
        let _ = std::fs::write(&owner_path, stamp);
        Ok(Self { file, owner_path })
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
    /// **Found by the mutation gate on its first completing run** (`datalock.rs:119`, the
    /// `!s.trim().is_empty()` guard replaced with `true` and nobody noticed). Without the guard
    /// a blank file becomes the holder's name, so the contention message reads `waiting for `
    /// with the sentence ending in air — the one message this whole owner file exists to print.
    /// It is reachable: `acquire` writes the lock file and the stamp as two steps, so a reader
    /// arriving between them, or after a crash between them, sees exactly this.
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
}
