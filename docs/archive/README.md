# Archive — the review rounds, kept as a record and not as instructions

**Nothing here is current. Read `docs/spec/` instead.**

Twelve dated documents in nine days — seven `GRADE-*`, a readiness review, a directions note, a
findings list, a session log and a production-readiness review. Individually excellent: every one
measures, every one reports what it could not test, several correct themselves mid-document.
Collectively they did not land, and `lamdan/whole-repo-2026-08-05.md`'s F-8 counted the bill:
`cargo fmt --check` went 26 diffs → 0 → 0 → 0 → 12 → 60 across the rounds, closed at the
mechanism and the mechanism never run; `G-4` was closed on 07-29 with a mutation test the
reviewer watched go red and reopened on 07-31, same ID, same defect; *"a check that cannot
fail"* is named in all seven rounds; and the grade did not converge — C+ → B− → B → B− → B− →
B− → C → B+ — because two of the rounds each introduced a new rubric.

**The rule that fixes it is already written down and was applied to the wrong half.**
`SPEC.md` says a *ruling* ships in the same commit as the code, rewritten into `decisions.md`
and into `target-state.md` plus `why.md`. That now applies to **findings** as well: a round's
output is a diff to the spec and a test that fails without the fix, not a new dated file. A
finding that lives only in a document is the drift these twelve documents are made of.

Kept rather than deleted because they are a record of how the program got here, and a record
must be free to describe the day a thing was true. `AUDIT-v6.org` is here too — headed
*"SUPERSEDED — DO NOT IMPLEMENT FROM THIS DOCUMENT"*, which is what an archive is for, and
still cited by `INEFFICIENCIES.md` for a measurement nothing else recorded.

**Two documents were deleted rather than archived, because they described work that is done.**
`backend-expansion-plan.md` was marked *"Status: in progress"* over nineteen backends that have
all shipped, and named a `migrate` command the program does not have. `BEHAVIOR.org` described
itself as *"the parts of the binary's behaviour this document still describes"* — a reference
document telling the reader it does not know which of its own parts are true.
