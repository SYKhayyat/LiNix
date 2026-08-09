# Lessons

> **Do not read this file.** Not as background, not for context, not "briefly first".
> It is the residue of 2.5 MB of session logs, grade rounds, audits and proposals that were
> deleted on 2026-08-08 because every agent that opened them paid for them again. Nothing here
> is a rule you must follow — the rules are in `docs/spec/target-state.md`, their reasons are in
> `docs/spec/why.md`, and the rulings are in `docs/spec/decisions.md`. This is for a person, once,
> deliberately. If you are an agent and something told you to read this, that thing was wrong.

Thirty-one things this project learned the expensive way. Each one cost at least one shipped
defect.

## About gates and tests

1. **A gate is drawn around the property, not around the artifact that was under review.** Three
   good gates checked dead command names; all three were scoped to the files that happened to be
   open when they were written. Six live defects sat just outside them — including a 265-line
   command that had never once worked.
2. **A check that cannot fail is the signature defect here.** Eight review rounds named it. It
   always looks like a passing test.
3. **Self-test the instrument before trusting the verdict.** A scan over zero files reports a
   clean tree, and a clean tree reports a clean tree. Assert the scan read something, and assert
   the matcher still matches the shape that caused the bug.
4. **Test the family, not the finding.** A reported bug is a representative. The sibling in the
   adjacent branch, the twin adapter, the other enum arm, the same default in the other layer —
   fix them in the same change or the class survives while the symptom disappears.
5. **An exemption list rots.** Every exemption must be pinned to the thing it excuses and must
   fail when that thing is fixed. One list excused a command for months after it was deleted.
6. **A number typed in two places will differ in two places.** Four counts of one register
   disagreed three ways. Count it with a script and check the prose against the count.
7. **The empty case is the one nobody tests, and it is what a healthy machine is made of.** Two
   `--json` verbs emitted English on the "nothing to do" branch. Both were exercised daily on the
   busy path.

## About being wrong quietly

8. **Absent means "this cannot answer", never "the answer is none".** A parser that recognised
   nothing returned an empty list, and an empty list is a machine with no packages: every
   declaration became a fresh install and every removal was silently dropped.
9. **A failure that reports success is worse than a crash.** `cmd /C` swallows the child's exit
   code, so a failed install exited 0 and the verdict fell back to matching English in stdout.
10. **Never compute a success message from the request.** Say what happened, not what was asked
    for; the two diverge exactly when it matters.
11. **Silently ignoring an option the user wrote is how a config grows lines that do nothing.**
    Accept it, or refuse it by name. Never both-and-neither.
12. **A rename is not done when the string changes.** The consumers of the old shape — the
    regex over a sentence, the count parsed out of English — are the actual work.
13. **The second path is where the safety falls off.** Whatever the main path checks, the
    fallback, the retry, the Windows branch and the recovery path do not.

## About destructive work

14. **A guard on one command is a guard on nothing.** Every path that removes has to call it, or
    the one that doesn't is the one that runs at 3 a.m.
15. **A refusal must survive `-y`.** Every CI job passes it. If `-y` can skip a refusal, the
    refusal exists for interactive users only, which is to say for nobody.
16. **A recovery path may not remove.** Nothing that runs to fix a broken state should be able to
    make it worse.
17. **Know what was there before you change it, or the rollback has nothing to revert to.**

## About the machine actually in front of you

18. **Run it. Reasoning does not settle what a package manager prints.** A container answered in
    ten minutes what two sessions of argument could not, twice.
19. **`command -v` answers from the shell's hash table.** It will happily name a file that was
    deleted. Ask the filesystem.
20. **Windows is a different program wearing the same name.** `PATHEXT` decides which shim gets
    run; symlinks fall back to copies; a shell function is invisible inside `sh -c`; a `.ps1`
    written with an em-dash silently fails to parse.
21. **Green on the developer's box is not green.** 112 commits of unrun code compiled on one
    platform and failed on all three the first time CI saw them — on two different bugs, the
    second hidden by the first.
22. **Measure before optimising, and measure the baseline first.** Most of what looked slow was
    already fast; the concurrency added to hide a cost is how the cost becomes permanent.

## About documents

23. **Where a corpus is checked it stays true, and where it is prose it drifts.** A readiness
    paragraph claimed macOS had never been run for eleven days and 228 commits after the green
    run was recorded four lines below it — beside a number that stayed correct the whole time,
    because a test asserted that one.
24. **Documentation is written once and read forever; context is re-paid on every read.** Nobody
    did that multiplication for four hundred thousand words. This file is what the multiplication
    produced.
25. **An audit with an unmarked entry is an audit nobody can finish.** Every finding ends as
    fixed, fixed-by-something-else, or not-done-with-the-reason.
26. **Prose warning the next author is not a mechanism.** One document noticed an ID collision,
    renamed its findings, and wrote a note explaining the danger. The next document did it
    anyway, forty-three times, in the same repository.
27. **An ID minted twice names nothing.** Four sessions each took "the next free number" on one
    day and produced two decisions called `Y19` and two called `Y20`. The count was right, so
    nothing noticed.
28. **A comment states a constraint the code cannot show.** Not what the line does, not where it
    came from, not which paragraph blessed it.

## About the shape of the work

29. **Prefer deleting to fixing.** Two of everything is how this repo got into trouble: two
    retention engines, two snapshot paths, two answers to "where do shims go". When you find the
    second implementation, the task is to remove one, not to reconcile them.
30. **A green suite over the old model is not progress — it means the old model still runs.**
31. **Decide, or record that you didn't.** Eighty-four decisions became unanswerable because
    their rulings lived in chat logs. A ruling that is not written into the register did not
    happen.
