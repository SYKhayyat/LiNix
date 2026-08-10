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
//! dies, and deleting them removes the very files the next `apt` expects to lock. They are rows
//! in the same table rather than a second list beside it: a row that says *why it may never be
//! removed* travels with the paths it is about, and cannot be re-admitted by someone extending
//! the table who never read the other one.
//!
//! **Staleness is proved, never assumed.** A lock carrying a pid is stale when that pid is not
//! running. A lock carrying nothing is stale when no process of the owning manager is running at
//! all. Neither question is asked on a platform that has no `/proc`, and every one of these
//! managers is Linux-only, so elsewhere this module finds nothing and says nothing.
//!
//! **And the other half of the same knowledge: waiting.** A lock that a *live* manager holds is
//! not a defect at all — it is the ordinary case of two package managers on one machine, and the
//! only thing that helps is to wait for the one that got there first. LiNix used to retry four
//! times over three and a half seconds and then tell the user *"this is not the transient failure
//! its output looks like"*, which was false in exactly the case it was printed. [`held_for`]
//! answers which of the three states the machine is in, so the retry loop can wait, refuse, or
//! send the user to `heal` — and say something true in each case.

use std::path::{Path, PathBuf};

/// A manager lock, what proves it is not held, and whether it may ever be cleared.
pub struct ManagerLock {
    /// Every binary that could be holding it, display name first. More than one because a
    /// manager is rarely one program: `apt`, `apt-get` and `dpkg` share dpkg's lock, and an
    /// `unattended-upgrade` holds it without either name on the command line.
    pub holders: &'static [&'static str],
    /// The backends whose failures this lock explains. `pacman`'s lock is what stops `yay`.
    pub backends: &'static [&'static str],
    /// Where the lock lives. Several, because a manager may leave more than one.
    pub paths: &'static [&'static str],
    /// Whether the file's contents are the pid of the holder.
    pub carries_pid: bool,
    /// Why this lock may never be removed — `None` when its existence *is* the lock and
    /// clearing a stale one is the repair.
    ///
    /// A row carrying a reason is still a full row: it is matched, waited on, and reported. The
    /// only thing it never is, is deleted.
    pub never_remove_because: Option<&'static str>,
    /// What the manager prints when someone else holds this lock. Lowercase — matched against
    /// the lowercased failure text.
    pub taken_markers: &'static [&'static str],
}

impl ManagerLock {
    /// The name to put in a sentence.
    pub fn holder(&self) -> &'static str {
        self.holders[0]
    }

    /// Whether a stale one of these may be cleared.
    pub fn removable(&self) -> bool {
        self.never_remove_because.is_none()
    }
}

/// Every lock whose *presence* means a run is in progress.
///
/// A manager joins this list when someone has checked that its lock is created and deleted
/// around the transaction rather than existing permanently — the same standard
/// `core::argv`'s terminator table holds, and for the same reason: the cost of being wrong is
/// paid on somebody's machine, not here.
pub const MANAGER_LOCKS: &[ManagerLock] = &[
    ManagerLock {
        holders: &["pacman"],
        // The AUR helpers drive pacman for the write, so pacman's lock is what stops them, and
        // the process holding it is called `pacman` whichever of them started it.
        backends: &["pacman", "yay", "paru"],
        paths: &["/var/lib/pacman/db.lck"],
        // pacman's db.lck is empty. Its own message says so: *"if you're sure a package manager
        // is not already running"* — it cannot tell you itself, because it wrote no pid.
        carries_pid: false,
        never_remove_because: None,
        taken_markers: &["unable to lock database", "could not lock database"],
    },
    ManagerLock {
        holders: &["dnf", "dnf5", "yum", "microdnf"],
        backends: &["dnf", "yum", "microdnf"],
        paths: &["/var/cache/dnf/metadata_lock.pid"],
        carries_pid: true,
        never_remove_because: None,
        taken_markers: &[
            "another app is currently holding the yum lock",
            "waiting for process with pid",
        ],
    },
    ManagerLock {
        holders: &["zypper"],
        backends: &["zypper"],
        paths: &["/run/zypp.pid"],
        carries_pid: true,
        never_remove_because: None,
        taken_markers: &[
            "system management is locked by the application",
            "zypp-refresh",
        ],
    },
    // The `flock(2)` family. Present always, held only sometimes, and never removable — but
    // still a row, because "who holds it and should we wait" is a question worth asking about
    // apt more than about any other manager on this list.
    ManagerLock {
        holders: &["apt", "apt-get", "dpkg", "unattended-upgr", "aptd"],
        backends: &["apt", "apt-get", "dpkg"],
        paths: &[
            "/var/lib/dpkg/lock-frontend",
            "/var/lib/dpkg/lock",
            "/var/lib/apt/lists/lock",
            "/var/cache/apt/archives/lock",
        ],
        carries_pid: false,
        never_remove_because: Some(
            "dpkg and apt lock these with flock(2). The files are permanent, their presence \
             means nothing, the kernel released the lock the moment the holder died, and \
             deleting one deletes what the next `apt` expects to lock",
        ),
        taken_markers: &[
            "could not get lock",
            "unable to acquire the dpkg frontend lock",
            "is another process using it",
        ],
    },
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

/// A lock that is there and is being left alone, with the reason.
///
/// **`heal` is never silent about a lock it examined.** The first version reported only what it
/// removed, so the run where it decided *not* to — the one that then failed on that very lock —
/// printed nothing at all, and the reason had to be inferred from a machine where the decision
/// went the other way. Twice, wrongly. A decision this consequential says itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftAlone {
    pub path: PathBuf,
    pub because: String,
}

/// Everything the scan concluded: what may go, and what may not and why.
#[derive(Debug, Default)]
pub struct Survey {
    pub stale: Vec<Stale>,
    pub left: Vec<LeftAlone>,
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

/// **A zombie holds nothing.** It is a process that has already exited and whose parent has not
/// yet collected its status; the kernel released every file it had open at the moment it died.
/// It nonetheless keeps its `/proc/<pid>` entry, and `comm` still reads `pacman`.
///
/// This cost a green CI run. A killed sync leaves its `pacman` reaped a moment later, and `heal`
/// runs inside that moment — asking "is a pacman running", getting yes from a corpse, and
/// leaving the lock. The scan that said no pacman was there looked nine seconds afterwards,
/// which is how it read as a contradiction rather than as a race.
///
/// Field 3 of `/proc/<pid>/stat` is the state letter. It is read after the last `)` because a
/// process name can contain spaces and parentheses — `(pac man)` would shift every field if the
/// line were split from the left.
fn is_zombie(proc_dir: &Path) -> bool {
    std::fs::read_to_string(proc_dir.join("stat"))
        .ok()
        .and_then(|stat| {
            let after_name = stat.rsplit_once(')')?.1;
            after_name.split_whitespace().next().map(str::to_string)
        })
        .is_some_and(|state| state == "Z")
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
                && !is_zombie(&e.path())
        })
    }
}

/// What the machine says about a manager's lock right now.
///
/// The three answers are three different actions, and collapsing any two of them is how the
/// wrong sentence got printed: LiNix treated *held by a running manager* and *left behind by a
/// dead one* as one failure, retried both four times in three and a half seconds, and told the
/// user neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Held {
    /// A live process holds it. Nothing is wrong; the work is not lost; waiting is the fix.
    Live(String),
    /// The lock is on disk and nothing holds it. Waiting is forever; `heal` is the fix.
    Stale(PathBuf),
    /// Nothing holds it. If a command just failed on it, the holder let go in between.
    Free,
}

/// Which lock, if any, explains a failure from this backend.
pub fn lock_of(backend: &str) -> Option<&'static ManagerLock> {
    MANAGER_LOCKS.iter().find(|l| l.backends.contains(&backend))
}

/// Whether this failure text is the manager saying someone else holds its lock.
///
/// Asked of the failure LiNix already has rather than of the filesystem, because the manager is
/// the only one that knows which of its locks it wanted. A backend with no lock in the table
/// answers `false` and pays one list scan for it.
pub fn says_the_lock_is_taken(backend: &str, message: &str) -> bool {
    let Some(lock) = lock_of(backend) else {
        return false;
    };
    let hay = message.to_ascii_lowercase();
    lock.taken_markers.iter().any(|m| hay.contains(m))
}

/// Who holds `backend`'s manager lock, as the machine can prove it.
///
/// The `flock(2)` rows cannot be answered from the filesystem at all — the file is there whether
/// or not it is held — so for those the running-process question *is* the whole answer, and it is
/// a sound one: the kernel drops an `flock` when its holder dies, so a lock that is genuinely
/// taken has a live taker by definition. Those rows can therefore never report [`Held::Stale`],
/// which is the same fact as their never being removable, arrived at from the other side.
pub fn held_for(
    backend: &str,
    procs: &dyn Processes,
    read: &dyn Fn(&Path) -> Option<String>,
) -> Held {
    let Some(lock) = lock_of(backend) else {
        return Held::Free;
    };
    if let Some(who) = running_holder(lock, procs) {
        return Held::Live(who);
    }
    if !lock.removable() {
        // Nothing is running, and the file's presence proves nothing. There is no stale
        // `flock` to report and nothing to wait for.
        return Held::Free;
    }
    for path in lock.paths {
        let path = Path::new(path);
        let Some(body) = read(path) else { continue };
        if !lock.carries_pid {
            // Its existence is the lock, and nothing of the manager's is running.
            return Held::Stale(path.to_path_buf());
        }
        // A pid file still naming a live process is held even when no process carries the
        // manager's name — `dnf` behind PackageKit is the case, and it is why the pid is read
        // rather than assumed to agree with the process list.
        match body.trim().parse::<u32>() {
            Ok(pid) if procs.pid_alive(pid) => return Held::Live(format!("pid {pid}")),
            Ok(_) => return Held::Stale(path.to_path_buf()),
            // Half-written by a process that died between `create` and `write`. That proves
            // nothing, and "proves nothing" must not become "go and delete it" — `find` leaves
            // exactly this file alone, and an answer here that sent the user to `heal` would
            // send them to a command that then declines.
            Err(_) => return Held::Free,
        }
    }
    Held::Free
}

/// The same question against the real machine.
pub fn held_for_on_this_machine(backend: &str) -> Held {
    held_for(backend, &ProcFs, &|p| std::fs::read_to_string(p).ok())
}

/// The first of a lock's holder programs that is running, phrased for a sentence.
fn running_holder(lock: &ManagerLock, procs: &dyn Processes) -> Option<String> {
    lock.holders
        .iter()
        .find(|h| procs.any_named(h))
        .map(|h| format!("a `{h}`"))
}

/// Every manager lock on this machine that exists and is provably not held.
///
/// `read` is how the file's contents are obtained, so the decision can be tested without a
/// filesystem. Nothing here removes anything — finding and acting are separate so `heal
/// --dry-run` can report exactly what the real run would do.
pub fn find(procs: &dyn Processes, read: &dyn Fn(&Path) -> Option<String>) -> Survey {
    let mut out = Survey::default();
    for lock in MANAGER_LOCKS {
        // The `flock(2)` rows are skipped whole, and silently. They are present on every Debian
        // machine that ever ran `apt`, so reporting them as "left alone" would print four lines
        // of noise on every `heal` — and the one thing that must never happen to them, being
        // removed, cannot happen if they never reach the loop below.
        if !lock.removable() {
            continue;
        }
        for path in lock.paths {
            let path = Path::new(path);
            let Some(body) = read(path) else {
                // A lock that is there and unreadable is not a lock to be silent about — it is
                // the strongest hint a reader could get about why a manager is refusing.
                if path.exists() {
                    out.left.push(LeftAlone {
                        path: path.to_path_buf(),
                        because: "it is there and could not be read, so nothing can be proved \
                                  about it"
                            .into(),
                    });
                }
                continue;
            };
            let because = match (lock.carries_pid, body.trim().parse::<u32>()) {
                // A pid that is still running: the lock is held, and this is the case the whole
                // module exists to not get wrong.
                (true, Ok(pid)) if procs.pid_alive(pid) => {
                    out.left.push(LeftAlone {
                        path: path.to_path_buf(),
                        because: format!("pid {pid} is running and holds it"),
                    });
                    continue;
                }
                (true, Ok(pid)) => format!(
                    "it names pid {pid}, and no such process is running — the run that took it \
                     was killed"
                ),
                // A pid file with no readable pid is not evidence of anything; leave it.
                (true, Err(_)) => {
                    out.left.push(LeftAlone {
                        path: path.to_path_buf(),
                        because: "it should name a pid and does not, which proves nothing either \
                                  way"
                        .into(),
                    });
                    continue;
                }
                (false, _) => match running_holder(lock, procs) {
                    Some(who) => {
                        out.left.push(LeftAlone {
                            path: path.to_path_buf(),
                            because: format!("{who} is running, so it is held"),
                        });
                        continue;
                    }
                    None => format!(
                        "it carries no pid and no `{}` is running, so nothing holds it",
                        lock.holder()
                    ),
                },
            };
            out.stale.push(Stale {
                path: path.to_path_buf(),
                holder: lock.holder(),
                because,
            });
        }
    }
    out
}

/// The same question against the real machine.
pub fn find_on_this_machine() -> Survey {
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
        assert_eq!(found.stale.len(), 1, "{found:?}");
        assert_eq!(found.stale[0].holder, "pacman");
        assert!(found.stale[0].because.contains("no `pacman` is running"));
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
        assert!(found.stale.is_empty(), "{found:?}");
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
        assert!(held.stale.is_empty(), "a live pid holds its lock: {held:?}");

        let stale = find(
            &Fake {
                alive: vec![1],
                running: vec!["dnf"],
            },
            &reader(&[("/var/cache/dnf/metadata_lock.pid", "4242\n")]),
        );
        assert_eq!(stale.stale.len(), 1, "{stale:?}");
        assert!(
            stale.stale[0].because.contains("4242"),
            "the reason has to name the pid it looked for: {}",
            stale.stale[0].because
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
        assert!(found.stale.is_empty(), "{found:?}");
    }

    /// A lock that is not there is not a problem. The common case, and it must cost nothing.
    #[test]
    fn a_machine_with_no_locks_has_nothing_to_clear() {
        let found = find(
            &Fake {
                alive: vec![],
                running: vec![],
            },
            &reader(&[]),
        );
        assert!(found.stale.is_empty());
        // And nothing to say about it either: the ordinary machine gets no output at all.
        assert!(found.left.is_empty());
    }

    /// **A lock left in place says why.** The whole of `Q50`'s misdiagnosis came from the one
    /// run where the answer was "no" printing nothing, so the reason had to be guessed from a
    /// machine where the answer was "yes".
    #[test]
    fn a_lock_that_is_left_alone_is_still_reported() {
        let found = find(
            &Fake {
                alive: vec![],
                running: vec!["pacman"],
            },
            &reader(&[("/var/lib/pacman/db.lck", "")]),
        );
        assert!(found.stale.is_empty());
        assert_eq!(found.left.len(), 1, "{found:?}");
        assert!(
            found.left[0].because.contains("a `pacman` is running"),
            "{}",
            found.left[0].because
        );
    }

    /// **The bug that cost a green CI run, as a unit test.** `is_zombie` reads
    /// `/proc/<pid>/stat`, whose third field is the state letter — after the last `)`, because
    /// a process name may itself contain spaces and parentheses.
    ///
    /// A killed `pacman` keeps its `/proc` entry, and `comm` still says `pacman`, until its
    /// parent reaps it. `heal` runs inside that window: it asked "is a pacman running", a corpse
    /// said yes, and the lock stayed. The scan that found no pacman ran nine seconds later,
    /// which made a race look like a contradiction.
    #[test]
    fn a_zombie_is_read_out_of_stat_whatever_the_process_was_called() {
        let dir = tempfile::TempDir::new().unwrap();
        let write = |body: &str| {
            std::fs::write(dir.path().join("stat"), body).unwrap();
            is_zombie(dir.path())
        };
        assert!(write("4242 (pacman) Z 1 4242 4242 0 -1 4194560"));
        assert!(!write("4242 (pacman) S 1 4242 4242 0 -1 4194560"));
        assert!(!write("4242 (pacman) R 1 4242 4242 0 -1 4194560"));
        // The name is the reason the field is found from the right: split from the left and
        // `(pac man)` moves every field along by one.
        assert!(write("4242 (pac man) Z 1 4242"));
        assert!(!write("4242 (pac man) S 1 4242"));
        // A stat nobody can read proves nothing, and "proves nothing" must not read as "dead":
        // that would clear a lock on the strength of a failed read.
        assert!(!is_zombie(std::path::Path::new("/no/such/proc/entry")));
    }

    /// **The apt/dpkg family may never be cleared, and the scan proves it.** Those files exist
    /// permanently and are locked with `flock(2)`; removing one deletes what the next `apt`
    /// expects to lock, and the kernel had already released the lock when the holder died.
    ///
    /// The rule is checked through `find` rather than by reading the table, because the table is
    /// what a future edit changes: a row that lost its reason would still be spotted here.
    #[test]
    fn no_flock_style_lock_is_ever_reported_as_clearable() {
        let flock: Vec<&str> = MANAGER_LOCKS
            .iter()
            .filter(|l| !l.removable())
            .flat_map(|l| l.paths.iter().copied())
            .collect();
        assert!(!flock.is_empty(), "the apt/dpkg family left the table");
        for path in flock {
            let found = find(
                &Fake {
                    alive: vec![],
                    running: vec![],
                },
                &reader(&[(path, "")]),
            );
            assert!(found.stale.is_empty(), "{path} was reported as clearable");
            // Nor as left-alone: these are on every Debian machine, and four lines of "still
            // there" on every heal is noise that trains a reader to skip the report.
            assert!(
                found.left.is_empty(),
                "{path} was reported at all: {found:?}"
            );
        }
    }

    /// A row that may never be removed has to say why, in the row. The reason used to live in a
    /// second table, which is the arrangement that lets someone extend one and not the other.
    #[test]
    fn a_row_that_may_not_be_cleared_carries_its_reason() {
        for lock in MANAGER_LOCKS {
            if let Some(why) = lock.never_remove_because {
                assert!(
                    why.contains("flock"),
                    "{} says it may not be cleared without saying what kind of lock it is",
                    lock.holder()
                );
            }
        }
    }

    /// Every entry says which manager it belongs to and where, because the report names both
    /// and a row with an empty field is a report with a hole in it.
    #[test]
    fn every_row_names_a_manager_and_at_least_one_path() {
        for lock in MANAGER_LOCKS {
            assert!(!lock.holders.is_empty());
            assert!(!lock.holder().is_empty());
            assert!(!lock.paths.is_empty(), "{} has no path", lock.holder());
            assert!(
                !lock.backends.is_empty(),
                "{} explains no backend's failures, so nothing can ever consult it",
                lock.holder()
            );
            assert!(
                !lock.taken_markers.is_empty(),
                "{} has no phrasing for 'someone else holds it', so the wait never triggers",
                lock.holder()
            );
            assert!(
                lock.paths.iter().all(|p| p.starts_with('/')),
                "{} has a relative lock path",
                lock.holder()
            );
        }
    }

    /// No backend may be claimed by two rows: the second would never be consulted, and which
    /// one wins would be the table's declaration order rather than anybody's decision.
    #[test]
    fn a_backend_belongs_to_at_most_one_lock() {
        let mut seen: Vec<&str> = Vec::new();
        for lock in MANAGER_LOCKS {
            for b in lock.backends {
                assert!(!seen.contains(b), "{b} is claimed by two locks");
                seen.push(b);
            }
        }
    }

    /// **The case the wait exists for.** Another pacman is mid-transaction. The lock is real,
    /// the holder is real, and the only thing that helps is to wait for it — which is not what
    /// four retries in three and a half seconds do.
    #[test]
    fn a_lock_a_live_manager_holds_says_to_wait() {
        let held = held_for(
            "pacman",
            &Fake {
                alive: vec![],
                running: vec!["pacman"],
            },
            &reader(&[("/var/lib/pacman/db.lck", "")]),
        );
        assert_eq!(held, Held::Live("a `pacman`".into()));
    }

    /// And the case waiting would never end: the lock outlived its holder. Waiting is forever,
    /// so the answer has to be the other one.
    #[test]
    fn a_lock_nothing_holds_says_it_is_stale_and_names_the_file() {
        let held = held_for(
            "pacman",
            &Fake {
                alive: vec![],
                running: vec![],
            },
            &reader(&[("/var/lib/pacman/db.lck", "")]),
        );
        assert_eq!(held, Held::Stale("/var/lib/pacman/db.lck".into()));
    }

    /// An AUR helper is stopped by pacman's lock, because pacman is what it runs to write.
    /// Reading the backend name literally would leave `yay` with no lock and no wait.
    #[test]
    fn the_aur_helpers_are_answered_by_pacmans_lock() {
        for backend in ["yay", "paru"] {
            assert_eq!(
                held_for(
                    backend,
                    &Fake {
                        alive: vec![],
                        running: vec!["pacman"]
                    },
                    &reader(&[("/var/lib/pacman/db.lck", "")]),
                ),
                Held::Live("a `pacman`".into()),
                "{backend} was not answered by pacman's lock"
            );
        }
    }

    /// **apt can be held but never stale.** The kernel releases an `flock` when its holder dies,
    /// so a taken one has a live taker by definition — and the permanent file on disk says
    /// nothing either way. Reporting `Stale` here would send a user to `heal` to delete a file
    /// that must never be deleted.
    #[test]
    fn a_flock_is_held_only_while_something_is_running() {
        let files = reader(&[("/var/lib/dpkg/lock-frontend", "")]);
        assert_eq!(
            held_for(
                "apt",
                &Fake {
                    alive: vec![],
                    running: vec!["dpkg"]
                },
                &files
            ),
            Held::Live("a `dpkg`".into()),
        );
        assert_eq!(
            held_for(
                "apt",
                &Fake {
                    alive: vec![],
                    running: vec![]
                },
                &files
            ),
            Held::Free,
            "a permanent file with nothing running is not a stale lock",
        );
    }

    /// A pid file is believed over the process list. `dnf` driven by PackageKit leaves no
    /// process called `dnf` at all, and the pid it wrote is the only thing that knows.
    #[test]
    fn a_live_pid_holds_its_lock_even_with_no_process_of_that_name() {
        assert_eq!(
            held_for(
                "dnf",
                &Fake {
                    alive: vec![4242],
                    running: vec![]
                },
                &reader(&[("/var/cache/dnf/metadata_lock.pid", "4242\n")]),
            ),
            Held::Live("pid 4242".into()),
        );
    }

    /// A backend with no lock in the table is not made to wait for one.
    #[test]
    fn a_backend_with_no_manager_lock_is_always_free() {
        assert_eq!(
            held_for(
                "npm",
                &Fake {
                    alive: vec![],
                    running: vec!["pacman"]
                },
                &reader(&[("/var/lib/pacman/db.lck", "")]),
            ),
            Held::Free
        );
        assert!(!says_the_lock_is_taken(
            "npm",
            "unable to lock database: File exists"
        ));
    }

    /// The marker match is what triggers the wait at all, so every row's phrasing is checked
    /// against the manager's real words — and against a failure that is *not* about a lock,
    /// because a wait on every failed install is a hang on every typo.
    #[test]
    fn each_managers_own_words_for_a_taken_lock_are_recognised() {
        let cases: &[(&str, &str)] = &[
            (
                "pacman",
                "error: failed to init transaction (unable to lock database)",
            ),
            (
                "dnf",
                "Waiting for process with pid 1234 to finish. Another app is currently holding \
                 the yum lock",
            ),
            (
                "zypper",
                "System management is locked by the application with pid 999",
            ),
            (
                "apt",
                "E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 42",
            ),
        ];
        for (backend, said) in cases {
            assert!(
                says_the_lock_is_taken(backend, said),
                "{backend} did not recognise its own lock message: {said}"
            );
            assert!(
                !says_the_lock_is_taken(backend, "E: Unable to locate package qqqq"),
                "{backend} called a missing package a taken lock"
            );
        }
    }
}
