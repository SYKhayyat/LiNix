//! One writer at a time on the data directory (II.8).
//!
//! LiNix is not the only thing that starts LiNix: the package-manager hooks it installs
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

/// Held for the whole run of a command that mutates state. Dropping it releases the lock.
pub struct DataLock {
    file: File,
    owner_path: PathBuf,
}

impl DataLock {
    /// Take the lock, waiting up to `timeout` for whoever holds it.
    ///
    /// Waiting with no reason given is indistinguishable from hanging, so the wait announces
    /// the holder — the lock file carries the pid and the command that took it.
    pub fn acquire(data_dir: &Path, command: &str, timeout: Duration) -> Result<Self> {
        std::fs::create_dir_all(data_dir).map_err(Error::from)?;
        let path = data_dir.join("linix.lock");
        let owner_path = data_dir.join("linix.lock.owner");

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(Error::from)?;

        if file.try_lock_exclusive().is_err() {
            eprintln!(
                "linix: waiting for the data directory — held by {}",
                Self::holder(&owner_path)
            );
            let deadline = Instant::now() + timeout;
            loop {
                if file.try_lock_exclusive().is_ok() {
                    break;
                }
                if Instant::now() >= deadline {
                    // S27: the old text ended "remove linix.lock if nothing is running", and
                    // that advice is never right. The lock is an OS lock on an open handle,
                    // released when the holding process exits — so a lock that is still
                    // contended after the wait proves a live holder, and deleting the file
                    // takes the lock away from it rather than from a corpse.
                    return Err(Error::Other(format!(
                        "the LiNix data directory is locked by {}, and still was after {}s.\n  \
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

        let stamp = format!("linix {} (pid {})", command, std::process::id());
        let _ = std::fs::write(&owner_path, stamp);
        Ok(Self { file, owner_path })
    }

    /// Who is holding the lock, for the message. The stamp lives beside the lock file rather
    /// than inside it: Windows refuses to read a file another process holds an exclusive lock
    /// on, which would leave the one message this exists to print saying nothing.
    fn holder(owner_path: &Path) -> String {
        match std::fs::read_to_string(owner_path) {
            Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => "another linix".to_string(),
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
            std::env::temp_dir().join(format!("linix-datalock-{}-{}", name, std::process::id()));
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
        let stamp = DataLock::holder(&dir.join("linix.lock.owner"));
        assert!(stamp.contains("sync"), "{}", stamp);
        assert!(
            stamp.contains(&std::process::id().to_string()),
            "a holder nobody can identify is the message this exists to avoid: {}",
            stamp
        );
    }

    #[test]
    fn a_second_holder_is_refused_with_who_holds_it_rather_than_hanging() {
        let dir = tmp("contended");
        let first = DataLock::acquire(&dir, "sync", Duration::from_secs(1)).unwrap();

        // A second *process* is what the lock is for; within one process the advisory lock
        // is not re-entrant on a separate handle either, which is what this asserts.
        let path = dir.join("linix.lock");
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
