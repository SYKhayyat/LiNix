//! The package manager's own lock, left behind by a run that was killed (`Q50`).
//!
//! **The failure.** A sync is interrupted — Ctrl-C, a dead battery, a container torn down mid
//! transaction — and the manager underneath it dies with its lock file still on disk. From then
//! on *every* LiNix run on that machine fails, with the manager's own words:
//!
//! ```text
//! error: failed to init transaction (unable to lock database)
//! error: could not lock database: File exists
//!   if you're sure a package manager is not already running, you can remove
//!   /var/lib/pacman/db.lck
//! ```
//!
//! LiNix already says this well — it relays the advice and adds *"tried 4 times; the failure did
//! not change, so this is not the transient failure its output looks like"*. What it could not do
//! is act, and `heal` is the command whose entire job is *a run was interrupted, make this
//! machine workable again*.
//!
//! **Only locks whose existence IS the lock.** This is the distinction the whole module turns on,
//! and getting it wrong corrupts a package database:
//!
//! | manager | lock | may it be removed? |
//! |---|---|---|
//! | pacman | `/var/lib/pacman/db.lck` | **yes** — created on start, removed on exit |
//! | dnf | `/var/cache/dnf/metadata_lock.pid` | **yes** — a pid file, removed on exit |
//! | zypper | `/run/zypp.pid` | **yes** — a pid file, removed on exit |
//! | apt / dpkg | `/var/lib/dpkg/lock-frontend`, `/var/lib/apt/lists/lock` | **NO** |
//!
//! apt and dpkg lock with `flock(2)` on files that exist permanently, whether or not anything
//! holds them. Their presence says nothing at all, the kernel drops the lock when the holder
//! dies, and deleting them removes the very files the next `apt` expects to lock. They are named
//! here rather than omitted, because "it is not in the table" and "it must never be in the table"
//! are different facts and only one of them survives someone extending the list.
//!
//! **Staleness is proved, never assumed.** A lock carrying a pid is stale when that pid is not
//! running. A lock carrying nothing is stale when no process of the owning manager is running at
//! all. Neither question is asked on a platform that has no `/proc`, and every one of these
//! managers is Linux-only, so elsewhere this module finds nothing and says nothing.

use std::path::{Path, PathBuf};

/// A manager lock LiNix may clear, and what proves it is not held.
pub struct ManagerLock {
    /// The binary that would be holding it — used when the lock file carries no pid.
    pub holder: &'static str,
    /// Where the lock lives. Several, because a manager may leave more than one.
    pub paths: &'static [&'static str],
    /// Whether the file's contents are the pid of the holder.
    pub carries_pid: bool,
}

/// Every lock whose *presence* means a run is in progress.
///
/// A manager joins this list when someone has checked that its lock is created and deleted
/// around the transaction rather than existing permanently — the same standard
/// `core::argv`'s terminator table holds, and for the same reason: the cost of being wrong is
/// paid on somebody's machine, not here.
pub const MANAGER_LOCKS: &[ManagerLock] = &[
    ManagerLock {
        holder: "pacman",
        paths: &["/var/lib/pacman/db.lck"],
        // pacman's db.lck is empty. Its own message says so: *"if you're sure a package manager
        // is not already running"* — it cannot tell you itself, because it wrote no pid.
        carries_pid: false,
    },
    ManagerLock {
        holder: "dnf",
        paths: &["/var/cache/dnf/metadata_lock.pid"],
        carries_pid: true,
    },
    ManagerLock {
        holder: "zypper",
        paths: &["/run/zypp.pid"],
        carries_pid: true,
    },
];

/// Locks that look like the ones above and must never be treated as them.
///
/// Kept as data, and asserted against `MANAGER_LOCKS` by a test, so extending the table cannot
/// quietly re-admit one: these files exist whether or not anything holds them, the lock is an
/// `flock(2)` the kernel releases when the holder dies, and deleting them deletes what the next
/// `apt` expects to find.
pub const NEVER_REMOVE: &[(&str, &str)] = &[
    (
        "/var/lib/dpkg/lock-frontend",
        "dpkg locks it with flock(2); the file is permanent and its presence means nothing",
    ),
    (
        "/var/lib/dpkg/lock",
        "the same, one layer down — dpkg's own database lock",
    ),
    (
        "/var/lib/apt/lists/lock",
        "apt's index lock, also flock(2) on a permanent file",
    ),
    (
        "/var/cache/apt/archives/lock",
        "apt's download-cache lock, the same shape again",
    ),
];

/// What a stale lock is, once found: enough to report it and to remove it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stale {
    pub path: PathBuf,
    pub holder: &'static str,
    /// The sentence `heal` prints. Carried rather than rebuilt so the report and the decision
    /// cannot describe different things.
    pub because: String,
}

/// Ask the running system whether a process is there.
///
/// A trait rather than a direct `/proc` read so the decision can be tested against a machine
/// that is not this one — the whole risk here is answering "not running" about a manager that
/// is, and a rule that can only be exercised by killing a real pacman is a rule nobody exercises.
pub trait Processes {
    /// Is a process with this pid alive?
    fn pid_alive(&self, pid: u32) -> bool;
    /// Is any process running under this program name?
    fn any_named(&self, name: &str) -> bool;
}

/// The real answer, from `/proc`. Nothing else has to be installed — `pgrep` is absent from
/// several of the images these managers run on.
pub struct ProcFs;

impl Processes for ProcFs {
    fn pid_alive(&self, pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).is_dir()
    }

    fn any_named(&self, name: &str) -> bool {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            // No `/proc`: this is not Linux, and none of these managers runs here. Answering
            // "yes, something is running" is the safe direction — it clears nothing.
            return true;
        };
        entries.flatten().any(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
                && std::fs::read_to_string(e.path().join("comm"))
                    .is_ok_and(|comm| comm.trim() == name)
        })
    }
}

/// Every manager lock on this machine that exists and is provably not held.
///
/// `read` is how the file's contents are obtained, so the decision can be tested without a
/// filesystem. Nothing here removes anything — finding and acting are separate so `heal
/// --dry-run` can report exactly what the real run would do.
pub fn find(procs: &dyn Processes, read: &dyn Fn(&Path) -> Option<String>) -> Vec<Stale> {
    let mut out = Vec::new();
    for lock in MANAGER_LOCKS {
        for path in lock.paths {
            let path = Path::new(path);
            let Some(body) = read(path) else {
                continue;
            };
            let because = match (lock.carries_pid, body.trim().parse::<u32>()) {
                // A pid that is still running: the lock is held, and this is the case the whole
                // module exists to not get wrong.
                (true, Ok(pid)) if procs.pid_alive(pid) => continue,
                (true, Ok(pid)) => format!(
                    "it names pid {pid}, and no such process is running — the run that took it \
                     was killed"
                ),
                // A pid file with no readable pid is not evidence of anything; leave it.
                (true, Err(_)) => continue,
                (false, _) if procs.any_named(lock.holder) => continue,
                (false, _) => format!(
                    "it carries no pid and no `{}` is running, so nothing holds it",
                    lock.holder
                ),
            };
            out.push(Stale {
                path: path.to_path_buf(),
                holder: lock.holder,
                because,
            });
        }
    }
    out
}

/// The same question against the real machine.
pub fn find_on_this_machine() -> Vec<Stale> {
    find(&ProcFs, &|p| std::fs::read_to_string(p).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fake {
        alive: Vec<u32>,
        running: Vec<&'static str>,
    }

    impl Processes for Fake {
        fn pid_alive(&self, pid: u32) -> bool {
            self.alive.contains(&pid)
        }
        fn any_named(&self, name: &str) -> bool {
            self.running.contains(&name)
        }
    }

    fn reader(files: &[(&str, &str)]) -> impl Fn(&Path) -> Option<String> {
        let map: HashMap<String, String> = files
            .iter()
            .map(|(p, b)| (p.to_string(), b.to_string()))
            .collect();
        move |p: &Path| map.get(&p.to_string_lossy().replace('\\', "/")).cloned()
    }

    /// The case this exists for: a killed run, and pacman's empty lock left behind.
    #[test]
    fn a_lock_with_no_pid_is_stale_when_its_manager_is_not_running() {
        let found = find(
            &Fake {
                alive: vec![],
                running: vec![],
            },
            &reader(&[("/var/lib/pacman/db.lck", "")]),
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].holder, "pacman");
        assert!(found[0].because.contains("no `pacman` is running"));
    }

    /// **And the case that must never be got wrong.** A pacman is mid-transaction; the lock is
    /// its lock. Clearing it here is how a package database is corrupted, and it is the reason
    /// the check is a proof rather than a file-exists test.
    #[test]
    fn a_lock_is_left_alone_while_its_manager_is_running() {
        let found = find(
            &Fake {
                alive: vec![],
                running: vec!["pacman"],
            },
            &reader(&[("/var/lib/pacman/db.lck", "")]),
        );
        assert!(found.is_empty(), "{found:?}");
    }

    /// A pid file answers for itself, and the answer is about that pid — not about whether any
    /// process of that name exists. Two dnf runs on one machine is not a thing, but a *reused*
    /// pid belonging to something else is, and `pid_alive` is what the lock's own claim means.
    #[test]
    fn a_pid_file_is_judged_by_its_own_pid() {
        let held = find(
            &Fake {
                alive: vec![4242],
                running: vec![],
            },
            &reader(&[("/var/cache/dnf/metadata_lock.pid", "4242\n")]),
        );
        assert!(held.is_empty(), "a live pid holds its lock: {held:?}");

        let stale = find(
            &Fake {
                alive: vec![1],
                running: vec!["dnf"],
            },
            &reader(&[("/var/cache/dnf/metadata_lock.pid", "4242\n")]),
        );
        assert_eq!(stale.len(), 1, "{stale:?}");
        assert!(
            stale[0].because.contains("4242"),
            "the reason has to name the pid it looked for: {}",
            stale[0].because
        );
    }

    /// A pid file with nothing readable in it proves nothing, and "proves nothing" is not
    /// "is stale". Half-written by a process that died between `create` and `write` is exactly
    /// the moment to be careful rather than clever.
    #[test]
    fn an_unreadable_pid_is_not_evidence_of_staleness() {
        let found = find(
            &Fake {
                alive: vec![],
                running: vec![],
            },
            &reader(&[("/run/zypp.pid", "not a pid")]),
        );
        assert!(found.is_empty(), "{found:?}");
    }

    /// A lock that is not there is not a problem. The common case, and it must cost nothing.
    #[test]
    fn a_machine_with_no_locks_has_nothing_to_clear() {
        assert!(find(
            &Fake {
                alive: vec![],
                running: vec![]
            },
            &reader(&[])
        )
        .is_empty());
    }

    /// **The apt/dpkg family is excluded, and the exclusion is checked.** Those files exist
    /// permanently and are locked with `flock(2)`; removing one deletes what the next `apt`
    /// expects to lock, and the kernel had already released the lock when the holder died. A
    /// list that merely omitted them would re-admit them the first time somebody extended it.
    #[test]
    fn no_flock_style_lock_is_ever_in_the_removable_table() {
        for (path, why) in NEVER_REMOVE {
            assert!(
                !MANAGER_LOCKS.iter().any(|l| l.paths.contains(path)),
                "{path} is removable in the table, and it must not be: {why}"
            );
            // And the scan does not find it either, whatever is on disk.
            assert!(
                find(
                    &Fake {
                        alive: vec![],
                        running: vec![]
                    },
                    &reader(&[(path, "")])
                )
                .is_empty(),
                "{path} was reported as clearable"
            );
        }
    }

    /// Every entry says which manager it belongs to and where, because the report names both
    /// and a row with an empty field is a report with a hole in it.
    #[test]
    fn every_row_names_a_manager_and_at_least_one_path() {
        for lock in MANAGER_LOCKS {
            assert!(!lock.holder.is_empty());
            assert!(!lock.paths.is_empty(), "{} has no path", lock.holder);
            assert!(
                lock.paths.iter().all(|p| p.starts_with('/')),
                "{} has a relative lock path",
                lock.holder
            );
        }
    }
}
