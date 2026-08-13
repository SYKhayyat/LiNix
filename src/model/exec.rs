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

/// The verb asking. An `exec:` line belongs to one or both of them (`H6`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Sync,
    Upgrade,
}

impl Verb {
    /// What `@on=` may say, and the one list of it.
    ///
    /// Lived in `grammar::statement` as `EXEC_ON_VALUES` while the type that *means* them lived
    /// here — two copies of three strings, which is how a value gets accepted by the parser and
    /// understood by nothing. The grammar reads this.
    pub const VALUES: &'static [&'static str] = &["sync", "upgrade", "both"];

    /// Does this line belong to this verb, given the whole line?
    ///
    /// **The catalogue's default is the row's, not the spelling's (`H8`).** A shipped step says
    /// on its own row which verb it belongs to — `rustup update` is an upgrade — so a user who
    /// writes `exec:step/rustup` and nothing else gets the step run by `upgrade`, which is the
    /// convenience the catalogue exists for. An `@on=` written on the line still wins: the row
    /// is a default, and a default a user cannot override is a rule wearing a default's name.
    pub fn claims_line(self, script: &str, on: Option<&str>) -> bool {
        match on {
            Some(explicit) => self.claims(Some(explicit)),
            None => match crate::model::step::named(script).and_then(crate::model::step::find) {
                Some(step) => self.claims(Some(&step.on)),
                None => self.claims(None),
            },
        }
    }

    /// Does this line belong to this verb?
    ///
    /// **`sync` is the default and that direction is the whole ruling.** `upgrade` ran no
    /// declared steps at all, so a firmware or `rustup` line correctly written and correctly
    /// approved was never run by the verb a user reaches for weekly. The obvious cure — run
    /// every `exec:` from `upgrade` too — makes a verb that has never executed user scripts
    /// start executing every script in every manifest that already exists, and the approval
    /// ledger cannot object: it answers *what* may run, never *which verb* may run it. So the
    /// widening is written on the line, one step at a time, and a manifest that says nothing
    /// means exactly what it meant yesterday.
    pub fn claims(self, on: Option<&str>) -> bool {
        match on.map(str::trim) {
            None | Some("sync") => self == Verb::Sync,
            Some("upgrade") => self == Verb::Upgrade,
            Some("both") => true,
            // Unreachable through the grammar, which refuses any other value by name. Read as
            // the default rather than as "runs everywhere": an unknown word must not widen
            // what runs it.
            Some(_) => self == Verb::Sync,
        }
    }
}

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
                format!("sha256:{} — NOT APPROVED; `shall lock` to approve", short)
            }
            Decision::NeedsApproval(_) => format!(
                "sha256:{} — CHANGED since you approved it; `shall lock` to approve",
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
        assert!(unapproved.contains("shall lock"), "{}", unapproved);

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

    /// Which verb claims which line, as a table (`H6`).
    ///
    /// **The `None` row is the ruling.** A manifest that says nothing about verbs belongs to
    /// `sync` and to `sync` alone, which is what makes this change invisible to every `exec:`
    /// line that already exists. The alternative — `upgrade` running everything — needs no
    /// option, no grammar and no test, and hands a verb that has never executed a user script
    /// every script somebody approved for a different verb.
    #[test]
    fn a_line_belongs_to_the_verb_it_names_and_a_silent_one_belongs_to_sync() {
        for (on, sync, upgrade, what) in [
            (None, true, false, "silent: today's meaning, unchanged"),
            (Some("sync"), true, false, "written out"),
            (Some("upgrade"), false, true, "the step this was built for"),
            (Some("both"), true, true, "the third case"),
            (
                Some(" upgrade "),
                false,
                true,
                "trimmed like every other value",
            ),
            (
                Some("nonsense"),
                true,
                false,
                "unknown reads as the default, never as both",
            ),
        ] {
            assert_eq!(Verb::Sync.claims(on), sync, "sync/{on:?}: {what}");
            assert_eq!(Verb::Upgrade.claims(on), upgrade, "upgrade/{on:?}: {what}");
        }
    }
}
