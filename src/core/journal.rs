use crate::core::{Error, PackageSpec, Result};
use crate::utils::file::persist;
use chrono::{Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    /// An `InProgress` entry `cleanup` aged out at 4h — the process that started it is
    /// gone. Still healable: the mutation may have half-run.
    Abandoned,
}

/// What the log records, and it is not only packages.
///
/// **A mutation needs a write-ahead record exactly when the next run cannot recompute it.**
/// Most of what a sync does to a resource is a read-then-write converge from a line in your
/// config — a `service:` is started, a `setting:` written, a `firewall:` port opened, a
/// `link:` placed. Interrupted, none of those needs a log: the next sync reads the machine,
/// sees the declaration unmet, and finishes the job. Recomputing from the declaration is a
/// *better* recovery than replaying a log, because it also corrects drift the log never saw.
/// Journalling them would be durability theatre, and is deliberately not done.
///
/// Two things a sync does are not that, and both live here:
///
/// - **A package.** An interrupted `apt install` wedges dpkg in a state no declaration
///   describes, so the log is the only record that the mutation began.
/// - **A script.** `exec:` runs code, and `@undo=` runs an arbitrary shell command. Nothing
///   recorded how far either got, their authors never promised they could be run twice, and
///   there is no declared end state to converge towards. Recovery cannot finish one — but a
///   machine that was killed halfway through a script must not come back and say nothing,
///   which is what it did while these variants did not exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JournalAction {
    Install(PackageSpec),
    Remove {
        name: String,
        backend: String,
    },
    /// A declared `exec:` script, recorded before the interpreter is started. The hash is the
    /// content's, which is what `locks/exec.toml` counts runs of — so a reader can tell
    /// whether the run that was interrupted is the one the next sync is about to repeat.
    Exec {
        script: String,
        hash: String,
    },
    /// The `@undo=` of an `exec:` whose line went away (U3). Kept verbatim: it is a shell
    /// command a human wrote, and naming it is the only way a reader can judge what a
    /// half-execution left behind.
    ExecUndo {
        script: String,
        command: String,
    },
}

impl JournalAction {
    /// The entry's subject as `<backend>:<name>` — the label every report prints and the key
    /// recovery collapses repeated attempts on.
    pub fn key(&self) -> String {
        let (backend, name) = self.identity();
        format!("{}:{}", backend, name)
    }

    /// The pair an id is minted from. A script is keyed by the two words that identify it to
    /// the user: what kind of thing ran, and which one.
    pub(crate) fn identity(&self) -> (&str, &str) {
        match self {
            Self::Install(s) => (s.backend.as_str(), s.name.as_str()),
            Self::Remove { name, backend } => (backend.as_str(), name.as_str()),
            Self::Exec { script, .. } => ("exec", script.as_str()),
            Self::ExecUndo { script, .. } => ("exec-undo", script.as_str()),
        }
    }

    /// Whether re-running this action is a recovery.
    ///
    /// A package: yes. Every manager Shall drives reaches a state when told to, and reaching
    /// it twice is reaching it once, so an interrupted install is finished by installing.
    ///
    /// A script: no. Replaying it would be recovery inventing a mutation rather than
    /// completing one, and it could do real damage — the half that already ran would run
    /// again. What recovery owes an interrupted script is an account of it, not a repeat.
    pub fn is_replayable(&self) -> bool {
        matches!(self, Self::Install(_) | Self::Remove { .. })
    }

    /// What was interrupted and what the next run will do about it — the whole value of
    /// recording a script, since nothing can be replayed to fix one.
    pub fn describe_interruption(&self) -> String {
        match self {
            Self::Exec { script, hash } => format!(
                "`exec:{}` (content {}) was interrupted part-way through. Nothing recorded how \
                 far it got, and the run was never counted — so the next `sync` will run it \
                 again from the top. If that script is not safe to run twice, this is the \
                 moment to check what it left behind.",
                script,
                // By characters, not bytes. `hash_script` returns hex, but by the time this
                // runs the value has been through a file — and a byte slice landing inside a
                // multi-byte character panics. Recovery is the last place in the program that
                // may fall over on the contents of a damaged journal, which is the file it
                // exists to read.
                hash.chars().take(12).collect::<String>()
            ),
            Self::ExecUndo { script, command } => format!(
                "the undo of `exec:{}` was interrupted part-way through: `{}`. It stays \
                 recorded, so the next `sync` will run it again from the top.",
                script, command
            ),
            other => format!("{} was interrupted.", other.key()),
        }
    }
}

/// The source of truth for the 'shall heal' command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub action: JournalAction,
    pub status: ActionStatus,
    pub started_at_unix: i64,
    /// Set only on reaching a terminal state; `None` while Pending or InProgress.
    pub finished_at_unix: Option<i64>,
    pub error: Option<String>,
}

/// Recovery from power failure, OS crash, or a kill mid-transaction depends on an
/// 'InProgress' entry being flushed to disk before any backend is invoked. A backend
/// called ahead of that flush is a modification `heal` cannot see or undo.
///
/// **Append-only, one line per state change.** It used to serialise the entire map, pretty
/// printed, through a temp file and a rename, on every transition — so installing 50 packages
/// wrote the whole growing journal ~100 times and the bytes written were O(n²) in the number of
/// actions. Worse, it did that synchronously while holding the one mutex every concurrent DAG
/// worker has to take, which put a hard throttle directly under the transaction's concurrency:
/// the more parallel the graph became, the more this cost. A log is the canonical append-only
/// structure, and appending makes each transition a constant-size write.
///
/// Reading is forward, last-writer-wins per id — the same rule `heal` already applies.
pub struct Journal {
    path: PathBuf,
    pub entries: HashMap<String, JournalEntry>,
}

impl Journal {
    /// The WAL of an ordinary run: Shall's data directory, wherever `SHALL_DATA_DIR` and the
    /// platform say that is.
    ///
    /// **The journal lives beside the registry**, and a run that places the registry itself —
    /// a test kernel, an isolated CI root — passes that directory to [`Journal::at`] instead.
    /// The rule used to be a comment above the caller that derived both paths by hand, which is
    /// how the registry came to be isolated and the WAL not: every `cargo test` appended to the
    /// developer's real journal, 733 KB of it, until a `PackageSpec` format change made the file
    /// unparseable and bricked every test at bootstrap.
    pub fn new() -> Result<Self> {
        Self::at(crate::utils::safe_data_dir().join(Self::FILE_NAME))
    }

    /// `.jsonl`, because it is one JSON value per line and not one JSON document.
    pub const FILE_NAME: &'static str = "journal.jsonl";

    /// The WAL at an explicit path. Injected rather than always derived, so a test kernel
    /// gets its own: `TestKernel` isolated the registry and the groups dir but not this,
    /// so every `cargo test` run appended to the developer's real `journal.json` — 733KB
    /// of test noise in real user data, and a format change to `PackageSpec` then made
    /// that file unparseable and bricked every test at bootstrap.
    pub fn at(path: PathBuf) -> Result<Self> {
        debug!("Initializing WAL at {:?}", path);

        let mut journal = Self {
            path,
            entries: HashMap::new(),
        };

        if journal.path.exists() {
            journal.load_sync()?;
        } else {
            trace!("No existing WAL found, starting fresh.");
        }

        Ok(journal)
    }

    fn load_sync(&mut self) -> Result<()> {
        let data = std::fs::read_to_string(&self.path).map_err(|e| {
            Error::Io(format!(
                "Failed to read WAL Journal at {:?}: {}",
                self.path, e
            ))
        })?;

        if data.trim().is_empty() {
            return Ok(());
        }

        // Forward, last-writer-wins: a later line for the same id supersedes an earlier one,
        // which is how a transition is recorded without rewriting what came before.
        let mut unreadable = 0usize;
        for line in data.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<JournalEntry>(line) {
                Ok(entry) => {
                    self.entries.insert(entry.id.clone(), entry);
                }
                Err(_) => unreadable += 1,
            }
        }

        if unreadable > 0 && self.entries.is_empty() {
            // Nothing at all was readable, so this is not a torn tail — it is a corrupt file.
            // S10: a corrupt WAL must NOT brick every command. It used to return `Err`, which
            // failed `App::new`, which failed everything, with no message saying which file to
            // delete. So: move it aside (preserved for inspection, and so it stops
            // re-triggering), say so loudly (P3 — fail loud), and start fresh.
            let backup = {
                let mut s = self.path.clone().into_os_string();
                s.push(".corrupt");
                std::path::PathBuf::from(s)
            };
            // A preview moves nothing. Setting the file aside is a filesystem change like any
            // other, and `--dry-run heal` on a machine with a damaged WAL was making one.
            let previewing = crate::core::dry_run::active();
            let moved = !previewing && std::fs::rename(&self.path, &backup).is_ok();
            warn!(
                "the WAL at {:?} is corrupt — none of its {} line(s) could be read. {} \
                 Starting a fresh journal so commands still run; an operation interrupted \
                 before this cannot be auto-recovered — re-run `shall sync` to reconcile.",
                self.path,
                unreadable,
                match (previewing, moved) {
                    (true, _) => format!(
                        "A real run would move it to {:?} for inspection; this preview left it \
                         alone.",
                        backup
                    ),
                    (false, true) => format!("It has been moved to {:?} for inspection.", backup),
                    (false, false) => {
                        "It could not be moved aside; it will be overwritten on the next write."
                            .to_string()
                    }
                },
            );
        } else if unreadable > 0 {
            // A crash partway through an append leaves one truncated line, and every complete
            // line before it is still a true record. Those stand; the damage is named.
            warn!(
                "{} line(s) of the WAL at {:?} could not be read and were skipped; {} entr(ies) \
                 were recovered. If an operation was interrupted it may not be auto-healable — \
                 run `shall sync` to reconcile.",
                unreadable,
                self.path,
                self.entries.len()
            );
        } else {
            debug!(
                "Successfully loaded {} historical log entries.",
                self.entries.len()
            );
        }
        Ok(())
    }

    /// Record one entry's current state, durably, before the backend it describes is invoked.
    ///
    /// One line appended and synced — not a re-serialisation of every entry ever recorded.
    fn append(&self, entry: &JournalEntry) -> Result<()> {
        self.append_all(std::slice::from_ref(entry))
    }

    /// The same, for several entries, at the cost of **one** flush rather than one each.
    ///
    /// Every line is on disk before this returns, so each entry keeps exactly the guarantee
    /// the WAL is for: the record reaches the disk before the manager it describes is invoked.
    /// What changes is the price of that guarantee. `sync_data` is a physical flush, and
    /// opening a wave of *k* packages paid *k* of them, serialised, on the critical path,
    /// while holding the journal mutex throughout — the loop batched the lock acquisition and
    /// not the thing that costs.
    fn append_all(&self, entries: &[JournalEntry]) -> Result<()> {
        trace!("appending {} entr(ies) to WAL", entries.len());
        let lines = entries
            .iter()
            .map(|entry| {
                serde_json::to_string(entry)
                    .map_err(|e| Error::Other(format!("Failed to serialize Journal entry: {}", e)))
            })
            .collect::<Result<Vec<String>>>()?;
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        // A preview records no WAL entry: `append_lines` answers that, and a run that
        // performed nothing has nothing to roll back.
        crate::utils::file::append_lines(&self.path, &refs)
            .map(|_| ())
            .map_err(|e| Error::Persist(format!("Write of WAL Journal failed: {}", e)))
    }

    /// Rewrite the log from the in-memory entries, dropping everything they no longer contain.
    ///
    /// Only `cleanup` needs this — removal is the one thing an append cannot express — and it
    /// runs once per invocation, not once per package.
    fn compact(&self) -> Result<()> {
        let mut data = String::new();
        for entry in self.entries.values() {
            data.push_str(
                &serde_json::to_string(entry)
                    .map_err(|e| Error::Other(format!("Failed to serialize Journal: {}", e)))?,
            );
            data.push('\n');
        }
        persist(&self.path, &data)
            .map(|_| ())
            .map_err(|e| Error::Persist(format!("Atomic rewrite of WAL Journal failed: {}", e)))
    }

    fn generate_id(backend: &str, package: &str) -> String {
        format!("{}:{}:{}", backend, package, Uuid::new_v4().simple())
    }

    /// MUST be called and flushed before invoking any backend command.
    pub fn record_start(&mut self, action: JournalAction) -> Result<String> {
        Ok(self
            .record_starts(vec![action])?
            .pop()
            .expect("one action in, one id out"))
    }

    /// Open a WAL entry for every member of a wave, in one flush, in order.
    ///
    /// **Either all of them are recorded or none is.** The lines are serialised first and
    /// written second, so a serialisation failure leaves the file untouched and the in-memory
    /// map untouched with it — which is stronger than the per-entry loop this replaces, where
    /// a failure on the third entry left the first two on disk and in memory for the caller to
    /// close by hand.
    pub fn record_starts(&mut self, actions: Vec<JournalAction>) -> Result<Vec<String>> {
        let entries: Vec<JournalEntry> = actions
            .into_iter()
            .map(|action| {
                let (b_name, p_name) = action.identity();
                JournalEntry {
                    id: Self::generate_id(b_name, p_name),
                    action,
                    status: ActionStatus::InProgress,
                    started_at_unix: Utc::now().timestamp(),
                    finished_at_unix: None,
                    error: None,
                }
            })
            .collect();

        self.append_all(&entries)?;

        let mut ids = Vec::with_capacity(entries.len());
        for entry in entries {
            debug!("Operation {} marked as InProgress in WAL.", entry.id);
            ids.push(entry.id.clone());
            self.entries.insert(entry.id.clone(), entry);
        }
        Ok(ids)
    }

    pub fn record_success(&mut self, id: &str) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Completed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            let entry = entry.clone();
            self.append(&entry)?;
            trace!("Operation {} marked as Completed.", id);
        } else {
            warn!("Attempted to mark unknown operation {} as successful.", id);
        }
        Ok(())
    }

    pub fn record_failure(&mut self, id: &str, err: &str) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.status = ActionStatus::Failed;
            entry.finished_at_unix = Some(Utc::now().timestamp());
            entry.error = Some(err.to_string());
            let entry = entry.clone();
            self.append(&entry)?;
            // `debug!`, not `warn!`: the user is about to be told this failure once, in their
            // own words, by whoever is returning the error. Saying it again here — with a
            // 32-hex operation id and the word WAL in it — is the same sentence a third time
            // in vocabulary that belongs to the journal, not to the person who typed a typo.
            debug!("Operation {} recorded as Failed in WAL: {}", id, err);
        } else {
            warn!("Attempted to record failure for unknown operation {}.", id);
        }
        Ok(())
    }

    /// Work that started, touched the system, and never reached an outcome — what `heal`
    /// finishes.
    ///
    /// `Pending` is excluded because it never reached a backend. `Abandoned` is included: it is
    /// an `InProgress` entry that `cleanup` aged out at 4h, and aging out is a statement about
    /// how long ago the process died, not about whether the package it was mutating is still
    /// half-removed. Excluding it meant a crash left unattended over lunch stopped being
    /// healable at all — the case where the machine is least likely to have been put right by
    /// hand in the meantime.
    ///
    /// **`Failed` is not interrupted** (owner ruling, 2026-08-05 — `Q33`). A failed attempt
    /// reached an outcome and reported it to the user in their own words at the moment it
    /// happened; the package is not installed and its declaration is still in the manifest, so
    /// the very next `sync` schedules it again. Recovering it here was duplicated work rather
    /// than extra coverage — and it compounded, because an interrupted entry that can never be
    /// recovered stays `InProgress` for ever, which keeps [`needs_recovery`](Self::needs_recovery)
    /// true and so ran a full recovery of every past failure in front of every sync. One sweep
    /// spent 208 seconds of a `watch --once` doing exactly that.
    pub fn interrupted_actions(&self) -> Vec<JournalEntry> {
        self.entries
            .values()
            .filter(|e| Self::is_interrupted(e))
            .cloned()
            .collect()
    }

    fn is_interrupted(e: &JournalEntry) -> bool {
        matches!(e.status, ActionStatus::InProgress | ActionStatus::Abandoned)
    }

    /// Every install this log records as having **completed**, as `(backend, name)`.
    ///
    /// **This log is the ownership record until the registry catches up, and `S87` is the gap
    /// between them.** The write-ahead log is written per operation; the ownership registry is
    /// written **once, at the end of a run**. A kill in the window between them leaves a package
    /// installed, its install recorded here as `Completed`, and nothing in the registry claiming
    /// it — so it is owned by nobody. Nothing downstream notices: `sync` converges because the
    /// package genuinely is installed, the preview plans nothing because there is nothing to
    /// plan, and the damage surfaces only when somebody tries to remove it, as a cleanup that
    /// reports success and takes nothing away.
    ///
    /// `interrupted_actions` cannot see these — the entry is closed, not open — which is why
    /// recovery walked past them for as long as both mechanisms have existed.
    pub fn completed_installs(&self) -> Vec<(String, String)> {
        self.entries
            .values()
            .filter(|e| matches!(e.status, ActionStatus::Completed))
            .filter_map(|e| match &e.action {
                JournalAction::Install(spec) => Some((spec.backend.clone(), spec.name.clone())),
                _ => None,
            })
            .collect()
    }

    /// True makes `sync` run `heal` on its own, without asking.
    ///
    /// The same predicate `interrupted_actions` filters on, and it has to be: when the trigger
    /// and the work disagreed, `heal` ran for an interrupted entry and then also re-attempted
    /// every failure the machine had ever recorded.
    pub fn needs_recovery(&self) -> bool {
        self.entries.values().any(Self::is_interrupted)
    }

    /// `InProgress` entries are NEVER purged: they are the record that something on this
    /// machine is half-done, and dropping one loses the only evidence of it.
    ///
    /// `Failed` is purged on the same age rule as `Completed`, because it is terminal for the
    /// same reason — an outcome was reached and reported. It was kept before because `heal`
    /// retried it; now that it does not (`Q33`), keeping it for ever would have traded an
    /// unbounded retry for an unbounded file. Nothing else in the program reads a `Failed`
    /// entry, and every recovery attempt writes a fresh one.
    ///
    /// Returns whether anything was dropped, which is also whether the log on disk was
    /// rewritten.
    pub fn cleanup_expired_logs(&mut self, days_threshold: i64) -> Result<bool> {
        let cutoff = Utc::now() - ChronoDuration::days(days_threshold);
        let cutoff_ts = cutoff.timestamp();

        let initial_count = self.entries.len();

        self.entries.retain(|id, entry| {
            let is_terminal = matches!(
                entry.status,
                ActionStatus::Completed | ActionStatus::Abandoned | ActionStatus::Failed
            );

            if is_terminal {
                let terminal_time = entry.finished_at_unix.unwrap_or(entry.started_at_unix);
                if terminal_time < cutoff_ts {
                    trace!("Pruning expired log record: {}", id);
                    return false;
                }
            }
            true
        });

        let purged = initial_count - self.entries.len();
        if purged > 0 {
            info!(
                "Maintenance complete. Purged {} historical records older than {} days.",
                purged, days_threshold
            );
            // A removal is the one transition an append cannot express.
            self.compact()?;
        }

        Ok(purged > 0)
    }

    pub fn cleanup(&mut self) -> Result<()> {
        debug!("journal maintenance");

        // An InProgress entry older than this is read as a crashed process, not a slow one:
        // the wrong call either abandons a live install or waits forever on a dead one.
        let stale_limit = Utc::now() - ChronoDuration::hours(4);
        let stale_ts = stale_limit.timestamp();

        let mut aged_out = false;
        for entry in self.entries.values_mut() {
            if entry.status == ActionStatus::InProgress && entry.started_at_unix < stale_ts {
                debug!("Marking stale task {} as Abandoned.", entry.id);
                entry.status = ActionStatus::Abandoned;
                entry.finished_at_unix = Some(Utc::now().timestamp());
                aged_out = true;
            }
        }

        let purged = self.cleanup_expired_logs(7)?;
        // `cleanup_expired_logs` rewrites when it drops something; aging an entry out without
        // dropping anything still has to reach the disk, or the next process re-ages it.
        if aged_out && !purged {
            self.compact()?;
        }

        if self.entries.is_empty() && self.path.exists() {
            trace!("WAL is empty. Removing journal file.");
            let _ = std::fs::remove_file(&self.path);
        }

        Ok(())
    }
}

/// Run a mutation with a write-ahead record around it, so being killed part-way through is
/// something `heal` can finish.
///
/// **This exists because the record is not the transaction's to own.** The engine journals what
/// it schedules, and for a long time that was read as "what the engine schedules is what gets
/// journalled" — so `apply`, `upgrade`, `remove-orphans`, `purge-undeclared`, the lease sweep,
/// the suspension restore, `run`, `shell` and the remediation install all reached a package
/// manager with nothing recording that they had. Nine paths, one of which is the most
/// destructive command in the program. `apply` now goes through the engine because it executes
/// a plan; the rest are single commands with no plan behind them, and a whole transaction —
/// snapshot, health checks, `after_sync` — is the wrong shape for reclaiming an expired lease.
/// What they need is the log, and this is the log without the ceremony.
///
/// **`record_start` runs before `mutation` is polled**, which is the whole property: a future
/// passed in is not yet running, and awaiting it only after the WAL write has reached the disk
/// is what makes the record write-*ahead*. A WAL write that fails aborts the mutation rather
/// than letting it run unrecorded — the same call the engine makes stillborn.
///
/// One id per action, and a caller removing four names in one manager command passes four:
/// they succeed and fail together, but a reader of an interrupted log needs the names.
pub async fn journalled<T, Fut>(
    journal: &tokio::sync::Mutex<Journal>,
    actions: Vec<JournalAction>,
    mutation: Fut,
) -> Result<T>
where
    Fut: std::future::Future<Output = Result<T>>,
{
    let ids = {
        let mut j = journal.lock().await;
        let mut ids = Vec::with_capacity(actions.len());
        for action in actions {
            ids.push(j.record_start(action)?);
        }
        ids
    };

    let outcome = mutation.await;

    {
        let mut j = journal.lock().await;
        match &outcome {
            Ok(_) => {
                for id in &ids {
                    let _ = j.record_success(id);
                }
            }
            Err(e) => {
                let message = e.to_string();
                for id in &ids {
                    let _ = j.record_failure(id, &message);
                }
            }
        }
    }

    outcome
}

/// The `JournalAction`s for removing these names from one backend.
pub fn removals_of(backend: &str, names: &[String]) -> Vec<JournalAction> {
    names
        .iter()
        .map(|name| JournalAction::Remove {
            name: name.clone(),
            backend: backend.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// A failed attempt is not interrupted work (owner ruling, 2026-08-05 — `Q33`).
    ///
    /// It reached an outcome and said so; the package is not installed and its line is still in
    /// the manifest, so the next `sync` schedules it again. Recovering it here was the same work
    /// twice — and it compounded, because one interrupted entry that can never be recovered
    /// keeps `needs_recovery` true for ever, so every sync ran a full recovery of every failure
    /// the machine had ever recorded in front of it. 208 seconds of one `watch --once`.
    #[test]
    fn recovery_finishes_interrupted_work_and_does_not_retry_failures() {
        let tmp = tempdir().unwrap();
        let mut journal = Journal::at(tmp.path().join("journal.json")).unwrap();

        let spec = |name: &str| crate::core::PackageSpec {
            name: name.to_string(),
            backend: "apt".to_string(),
            options: Default::default(),
            requires: Vec::new(),
            present: true,
        };
        let interrupted = journal
            .record_start(JournalAction::Install(spec("half-done")))
            .unwrap();
        let failed = journal
            .record_start(JournalAction::Install(spec("typo")))
            .unwrap();
        journal.record_failure(&failed, "no such package").unwrap();
        let done = journal
            .record_start(JournalAction::Install(spec("fine")))
            .unwrap();
        journal.record_success(&done).unwrap();

        let offered: Vec<String> = journal
            .interrupted_actions()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(
            offered,
            vec![interrupted.clone()],
            "only the entry that started and never reached an outcome is recovery's to finish"
        );
        assert!(
            journal.needs_recovery(),
            "the trigger and the work must be the same predicate — when they disagreed, one \
             interrupted entry pulled every past failure along with it"
        );

        // And with the interrupted one resolved, nothing runs at all.
        journal.record_success(&interrupted).unwrap();
        assert!(!journal.needs_recovery());
        assert!(journal.interrupted_actions().is_empty());
    }

    /// A `Failed` entry is terminal, so it ages out like any other terminal entry. Kept for ever
    /// it would have traded an unbounded retry for an unbounded file: nothing else in the program
    /// reads one, and every recovery attempt writes a fresh one.
    ///
    /// `InProgress` is the one status that is never purged at any age — it is the only record
    /// that something on this machine is half-done.
    #[test]
    fn a_failed_entry_ages_out_and_a_half_done_one_never_does() {
        let tmp = tempdir().unwrap();
        let mut journal = Journal::at(tmp.path().join("journal.json")).unwrap();

        let spec = |name: &str| crate::core::PackageSpec {
            name: name.to_string(),
            backend: "apt".to_string(),
            options: Default::default(),
            requires: Vec::new(),
            present: true,
        };
        let failed = journal
            .record_start(JournalAction::Install(spec("typo")))
            .unwrap();
        journal.record_failure(&failed, "no such package").unwrap();
        let completed = journal
            .record_start(JournalAction::Install(spec("fine")))
            .unwrap();
        journal.record_success(&completed).unwrap();
        let half_done = journal
            .record_start(JournalAction::Install(spec("half-done")))
            .unwrap();

        let ancient = (Utc::now() - ChronoDuration::days(30)).timestamp();
        for id in [&failed, &completed, &half_done] {
            let e = journal.entries.get_mut(id).unwrap();
            e.started_at_unix = ancient;
            e.finished_at_unix = Some(ancient);
        }

        assert!(journal.cleanup_expired_logs(7).unwrap());
        assert!(
            !journal.entries.contains_key(&failed),
            "a failed attempt is terminal — nothing reads it and heal no longer retries it"
        );
        assert!(!journal.entries.contains_key(&completed));
        assert!(
            journal.entries.contains_key(&half_done),
            "an unresolved InProgress entry is the only record that a package is half-installed"
        );
    }

    #[test]
    fn an_aged_out_crash_is_still_healable() {
        // R23: `cleanup` flips an InProgress entry to Abandoned after 4h. That must not take
        // it out of heal's reach -- a node aborted mid-removal never entered the rollback
        // history, so the WAL is the only thing that knows the package is half-removed.
        let tmp = tempdir().unwrap();
        let mut journal = Journal::at(tmp.path().join("journal.json")).unwrap();

        let id = journal
            .record_start(JournalAction::Remove {
                name: "python3".into(),
                backend: "apt".into(),
            })
            .unwrap();

        // Backdate past the 4h staleness limit, then age it out.
        journal.entries.get_mut(&id).unwrap().started_at_unix =
            (Utc::now() - ChronoDuration::hours(5)).timestamp();
        journal.cleanup().unwrap();

        assert_eq!(
            journal.entries[&id].status,
            ActionStatus::Abandoned,
            "cleanup should still age the entry out"
        );
        assert!(
            journal.needs_recovery(),
            "an abandoned mutation must still trigger a heal"
        );
        assert!(
            journal.interrupted_actions().iter().any(|e| e.id == id),
            "an abandoned mutation must still be offered to heal"
        );
    }

    #[test]
    fn a_corrupt_wal_does_not_brick_every_command() {
        // S10: a bad parse used to fail App::new and therefore every command. It must
        // instead recover: move the bad file aside, start fresh, and still construct.
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("journal.json");
        std::fs::write(&path, b"{ this is not valid json ]]").unwrap();

        let journal = Journal::at(path.clone()).expect("a corrupt WAL must not fail construction");

        // Started fresh...
        assert!(!journal.needs_recovery());
        // ...the bad file was set aside for inspection...
        let backup = {
            let mut s = path.clone().into_os_string();
            s.push(".corrupt");
            std::path::PathBuf::from(s)
        };
        assert!(
            backup.exists(),
            "the corrupt WAL should be preserved at {:?}",
            backup
        );
        // ...and it is no longer at the live path (so it won't re-trigger).
        assert!(
            !path.exists(),
            "the corrupt WAL should have been moved off the live path"
        );
    }

    #[test]
    fn a_missing_wal_starts_fresh_without_error() {
        let tmp = tempdir().unwrap();
        let journal = Journal::at(tmp.path().join("nope.json")).unwrap();
        assert!(!journal.needs_recovery());
    }

    fn a_spec(name: &str) -> crate::core::PackageSpec {
        crate::core::PackageSpec {
            name: name.to_string(),
            backend: "apt".to_string(),
            options: Default::default(),
            requires: Vec::new(),
            present: true,
        }
    }

    /// The property the whole thing rests on, and the one an ordinary "did it write an entry"
    /// test would pass without: the record is on disk **before** the manager is invoked.
    ///
    /// A wrapper that recorded around the call and flushed afterwards would satisfy every
    /// assertion about the finished state and provide no recovery at all, because the case it
    /// exists for is the process dying in the middle. So the mutation itself is the observer —
    /// it reads the log from a second handle on the same file, which is what a fresh process
    /// after a crash would see.
    #[tokio::test]
    async fn the_record_reaches_disk_before_the_mutation_runs() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("journal.jsonl");
        let journal = tokio::sync::Mutex::new(Journal::at(path.clone()).unwrap());

        let seen_by_a_later_process = {
            let path = path.clone();
            journalled(
                &journal,
                vec![JournalAction::Install(a_spec("htop"))],
                async move { Journal::at(path).map(|j| j.interrupted_actions().len()) },
            )
            .await
            .unwrap()
        };

        assert_eq!(
            seen_by_a_later_process, 1,
            "a process starting while the mutation was running saw no interrupted entry — the \
             log is being written after the fact, which is not a write-ahead log"
        );
    }

    /// Success closes every id, so the entry stops being recovery's business.
    #[tokio::test]
    async fn a_completed_mutation_leaves_nothing_to_heal() {
        let tmp = tempdir().unwrap();
        let journal = tokio::sync::Mutex::new(Journal::at(tmp.path().join("j.jsonl")).unwrap());

        journalled(
            &journal,
            removals_of("apt", &["a".into(), "b".into()]),
            async { Ok(()) },
        )
        .await
        .unwrap();

        let j = journal.lock().await;
        assert_eq!(
            j.entries.len(),
            2,
            "one entry per name, not one per command"
        );
        assert!(
            !j.needs_recovery(),
            "a mutation that finished is not interrupted work"
        );
    }

    /// A failure is terminal and reported, not interrupted (`Q33`) — so it must not leave the
    /// next sync running a recovery of something that already told the user what happened.
    #[tokio::test]
    async fn a_failed_mutation_is_recorded_failed_and_not_healable() {
        let tmp = tempdir().unwrap();
        let journal = tokio::sync::Mutex::new(Journal::at(tmp.path().join("j.jsonl")).unwrap());

        let outcome: Result<()> = journalled(
            &journal,
            vec![JournalAction::Remove {
                name: "nope".into(),
                backend: "apt".into(),
            }],
            async { Err(Error::Other("no such package".into())) },
        )
        .await;

        assert!(outcome.is_err(), "the caller's error must reach the caller");
        let j = journal.lock().await;
        let entry = j.entries.values().next().unwrap();
        assert_eq!(entry.status, ActionStatus::Failed);
        assert!(entry.error.as_deref().unwrap().contains("no such package"));
        assert!(!j.needs_recovery());
    }

    /// A mutation that never returns — the process was killed — leaves the entry `InProgress`,
    /// which is the whole point. Modelled by dropping the future rather than awaiting it: the
    /// record is written by then, and nothing else runs.
    #[tokio::test]
    async fn an_abandoned_mutation_stays_interrupted() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("j.jsonl");
        let journal = tokio::sync::Mutex::new(Journal::at(path.clone()).unwrap());

        {
            let pending = journalled(
                &journal,
                vec![JournalAction::Install(a_spec("half"))],
                std::future::pending::<Result<()>>(),
            );
            futures::pin_mut!(pending);
            // Polled once — far enough to write the record and start the mutation, never far
            // enough to finish it — then dropped, which is what SIGKILL looks like from here.
            let _ = futures::poll!(&mut pending);
        }

        let after_the_crash = Journal::at(path).unwrap();
        assert_eq!(
            after_the_crash.interrupted_actions().len(),
            1,
            "the next process must find the half-done install recorded"
        );
        assert!(after_the_crash.interrupted_actions()[0]
            .action
            .is_replayable());
    }

    /// A WAL write that fails stops the mutation instead of letting it run unrecorded — the
    /// same call the transaction engine makes stillborn. Provoked by pointing the log at a
    /// path whose parent is a file, so the append cannot succeed.
    #[tokio::test]
    async fn a_log_that_cannot_be_written_refuses_the_mutation() {
        let tmp = tempdir().unwrap();
        let blocker = tmp.path().join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();
        let journal =
            tokio::sync::Mutex::new(Journal::at(blocker.join("sub").join("j.jsonl")).unwrap());

        let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ran.clone();
        let outcome: Result<()> = journalled(
            &journal,
            vec![JournalAction::Install(a_spec("htop"))],
            async move {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
        )
        .await;

        assert!(outcome.is_err(), "an unrecordable mutation must not run");
        assert!(
            !ran.load(std::sync::atomic::Ordering::SeqCst),
            "the manager was invoked with nothing recording that it had been"
        );
    }

    /// **The window `S87` lives in, and what closes it.**
    ///
    /// The log is written per operation; the ownership registry is written **once, at the end of
    /// a run**. A kill in between leaves a package installed, its install recorded here as
    /// `Completed`, and nothing in the registry claiming it — owned by nobody. Nothing
    /// downstream notices: `sync` converges (the package genuinely is installed), the preview
    /// plans nothing (there is nothing to plan), and the damage surfaces only when somebody
    /// tries to remove it, as a cleanup that reports success and takes nothing away.
    ///
    /// `interrupted_actions` cannot see these — the entry is **closed**, not open — which is
    /// exactly why recovery walked past them. `reconcile_ownership` reads this instead, so a
    /// declared package this machine's own log records installing is claimed on that evidence
    /// rather than on whether the manager's listing happens to report it today.
    ///
    /// That last clause is the half a container found: a `SIGKILL` can leave a package
    /// **unpacked but not configured** — on disk, on `PATH`, and correctly reported by
    /// `dpkg-query`'s status field as *not installed*. The lister is right to say so. Ownership
    /// is a different question from installedness, and this is the answer to it.
    #[tokio::test]
    async fn a_completed_install_is_recorded_even_though_it_is_not_interrupted() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("j.jsonl");
        let journal = tokio::sync::Mutex::new(Journal::at(path.clone()).unwrap());

        // One install that finished, one that was killed part-way, one removal that finished.
        journalled(
            &journal,
            vec![JournalAction::Install(a_spec("landed"))],
            std::future::ready(Ok(())),
        )
        .await
        .unwrap();
        {
            let pending = journalled(
                &journal,
                vec![JournalAction::Install(a_spec("half"))],
                std::future::pending::<Result<()>>(),
            );
            futures::pin_mut!(pending);
            let _ = futures::poll!(&mut pending);
        }
        journalled(
            &journal,
            vec![JournalAction::Remove {
                name: "gone".into(),
                backend: "apt".into(),
            }],
            std::future::ready(Ok(())),
        )
        .await
        .unwrap();

        let after = Journal::at(path).unwrap();

        let completed = after.completed_installs();
        assert_eq!(
            completed,
            vec![("apt".to_string(), "landed".to_string())],
            "only the install that finished belongs here — not the interrupted one, which \
             `interrupted_actions` owns, and not the removal, which takes ownership away rather \
             than granting it"
        );

        // The control, and it is the whole reason this method had to exist: the entry above is
        // invisible to the mechanism recovery already had.
        let interrupted: Vec<String> = after
            .interrupted_actions()
            .iter()
            .map(|e| e.action.key())
            .collect();
        assert_eq!(
            interrupted,
            vec!["apt:half".to_string()],
            "a completed install is not interrupted, so recovery's existing reader cannot see \
             it — which is why a package it left behind stayed owned by nobody"
        );
    }
}
