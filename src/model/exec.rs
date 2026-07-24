//! What one `exec:` line means right now (XIII.3) — the decision, with no I/O in it.
//!
//! `plan` must print the decision `sync` is about to make, so both ask this one function. A
//! preview computed by different code from the run is not a preview.
//!
//! Only the *first* of XIII.3's three states reaches here. A line whose `when` is false was
//! dropped during resolution, so it is simply absent — and absent means nothing runs and no
//! ledger row is touched, which is what "the lock row survives a false `when`" amounts to in
//! code. There is no branch for it because there is nothing for a branch to do.

use crate::core::exec_lock::Ceiling;
use crate::core::hook_lock::Verdict;

/// What a sync will do with this script, and why — the "why" being the part `plan` prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// It will run: this content is below its ceiling and the script is approved.
    Run,
    /// Its content has already run as many times as it may.
    AlreadyRun { count: u32, ceiling: Ceiling },
    /// II.12: never approved, or changed since it was. A refusal, not a skip — `-y` cannot
    /// approve, and a sync stops rather than running code nobody vouched for.
    NeedsApproval(Verdict),
}

impl Decision {
    /// Decide, given what the two ledgers say. Both are passed in rather than read here so the
    /// decision stays pure and testable without a repo on disk.
    pub fn of(verdict: &Verdict, count: u32, ceiling: Ceiling) -> Decision {
        // Approval first: an unapproved script is refused whether or not it would have run
        // anyway. Reporting "already run" for a script that changed under you would hide the
        // very edit II.12 exists to catch.
        if !verdict.is_approved() {
            return Decision::NeedsApproval(verdict.clone());
        }
        if ceiling.permits(count) {
            Decision::Run
        } else {
            Decision::AlreadyRun { count, ceiling }
        }
    }

    pub fn will_run(&self) -> bool {
        matches!(self, Decision::Run)
    }

    /// One line for `plan`, naming the fact that decided it.
    pub fn describe(&self, hash: &str) -> String {
        let short = &hash[..hash.len().min(12)];
        match self {
            Decision::Run => format!("sha256:{} — will run", short),
            Decision::AlreadyRun { count, ceiling } => format!(
                "sha256:{} — already run {} time(s), ceiling {}; will not run",
                short, count, ceiling
            ),
            Decision::NeedsApproval(Verdict::New) => {
                format!("sha256:{} — NOT APPROVED; `linix lock` to approve", short)
            }
            Decision::NeedsApproval(_) => format!(
                "sha256:{} — CHANGED since you approved it; `linix lock` to approve",
                short
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed() -> Verdict {
        Verdict::Changed {
            was: "a".repeat(64),
            now: "b".repeat(64),
        }
    }

    #[test]
    fn an_approved_script_below_its_ceiling_runs() {
        let d = Decision::of(&Verdict::Approved, 0, Ceiling::read(None));
        assert_eq!(d, Decision::Run);
        assert!(d.will_run());
    }

    /// The exit condition's second clause: it does not run on the next sync.
    #[test]
    fn the_same_content_does_not_run_twice_by_default() {
        let d = Decision::of(&Verdict::Approved, 1, Ceiling::read(None));
        assert!(!d.will_run());
        assert!(matches!(d, Decision::AlreadyRun { count: 1, .. }));
    }

    #[test]
    fn always_runs_every_time() {
        assert!(Decision::of(&Verdict::Approved, 500, Ceiling::read(Some("always"))).will_run());
    }

    /// II.12 outranks the count: a script that changed under you is a refusal, not a skip.
    /// Reporting "already run" here would hide the exact edit the ledger exists to catch.
    #[test]
    fn approval_is_checked_before_the_count() {
        let d = Decision::of(&changed(), 1, Ceiling::read(None));
        assert!(matches!(d, Decision::NeedsApproval(_)));
        assert!(!d.will_run());

        let d = Decision::of(&Verdict::New, 0, Ceiling::read(None));
        assert!(matches!(d, Decision::NeedsApproval(Verdict::New)));
    }

    /// `plan` must print the hash, the count and the decision before any of it happens — the
    /// exit condition's fourth clause.
    #[test]
    fn the_description_carries_the_hash_the_count_and_the_verdict() {
        let hash = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
        assert!(Decision::Run.describe(hash).contains("a1b2c3d4e5f6"));
        assert!(Decision::Run.describe(hash).contains("will run"));

        let done = Decision::AlreadyRun {
            count: 2,
            ceiling: Ceiling::read(Some("3")),
        };
        let line = done.describe(hash);
        assert!(line.contains("already run 2 time(s)"), "{}", line);
        assert!(line.contains("ceiling 3"), "{}", line);

        let unapproved = Decision::NeedsApproval(Verdict::New).describe(hash);
        assert!(unapproved.contains("NOT APPROVED"), "{}", unapproved);
        assert!(unapproved.contains("linix lock"), "{}", unapproved);

        let stale = Decision::NeedsApproval(changed()).describe(hash);
        assert!(stale.contains("CHANGED"), "{}", stale);
    }

    /// A short hash must not panic the describer — the ledger stores 64 hex chars, but nothing
    /// in the type says so.
    #[test]
    fn a_short_hash_does_not_panic() {
        assert!(Decision::Run.describe("abc").contains("abc"));
        assert!(Decision::Run.describe("").contains("will run"));
    }
}
