# LiNix v7 — the declarative model

**This file is the way in. It holds the instructions and the map; the specification itself is
in [`spec/`](spec/), one file per part.** It was 9,308 lines in one file until 2026-07-23, at
which point nobody could find a decision in it and 84 of them had no recorded answer.

## The map

| file | part | what it is | when you read it |
|---|---|---|---|
| [`spec/principles.md`](spec/principles.md) | I | The eight principles that decide arguments. | First, and never violate them. |
| [`spec/target-state.md`](spec/target-state.md) | II | **Canonical.** What to build. If code disagrees with it, the code is wrong. | Before you write a line. |
| [`spec/why.md`](spec/why.md) | V | The reason behind every Part II rule — each one the scar of a real bug. | **Before changing any Part II rule.** |
| [`spec/plan.md`](spec/plan.md) | III + IV | The work in dependency order, each phase with its exit condition; then the proofs. | When picking up work. |
| [`spec/bugs.md`](spec/bugs.md) | VI | Bugs killed by this design, and bugs carried forward. **Read VI.0 first.** | Before building anything. |
| [`spec/decisions.md`](spec/decisions.md) | — | **All 104 decisions, with a status on each.** | Before proposing anything. |
| [`spec/history.md`](spec/history.md) | VII | How far the work got, session by session. **The living truth** — every frozen status line drifts behind the tree. | After Part II, before touching anything. |
| [`spec/proposals/`](spec/proposals/) | VIII–XIII | Six features that are designed but not decided. | Only with the register open beside you. |

**The proposals, and the decisions each one is waiting on:**

| part | file | decisions |
|---|---|---|
| VIII | [artifact selection and channels](spec/proposals/artifacts.md) | D1–D17 |
| IX | [user-defined `when` variables](spec/proposals/vars.md) | W1–W14 |
| X | [rebuild, caches, desktops, backup](spec/proposals/rebuild.md) | K1–K16 |
| XI | [`firewall:`](spec/proposals/firewall.md) | N1–N7 |
| XII | [secrets](spec/proposals/secrets.md) | T1–T5 |
| XIII | [the next round](spec/proposals/next-round.md) | U1–U38 |

**VI.0 — FIXED (S24/S25, session 2026-07-23, sixteenth; verified 2026-07-24).** The bug that
removed software with no guard, no plan and no count — and that `--dry-run` performed — is closed.
It is no longer the blocker this line once made it, and it is no longer "build nothing before it."
**For what remains to build and in what order, see the ordered list at the top of
[`spec/plan.md`](spec/plan.md)** (`decisions.md` is now at zero-open).

Facts marked **(measured)** were verified against real containers or real code with a citation.
Everything else is design.

Supersedes [`AUDIT-v6.org`](AUDIT-v6.org) — the audit that found all of this — except where
[`spec/bugs.md`](spec/bugs.md) carries an item forward explicitly. Read the audit only for the
underlying evidence: the measurements and the `file:line` citations behind each finding.

---

## PROMPT — read this first, then follow it

You are implementing LiNix v7 on `main` — the sole branch — at `C:\Users\Administrator\Videos\Nexus\linix`.
This document is your specification. It was produced by a long design conversation with the
owner; **every rule in it was argued for and chosen, and Part V records why.**

**Before you write a line of code:** read Part I and Part II in full. Read Part III's "What
already exists". You cannot implement this correctly from a summary.

### Rules of engagement

1. **Part II is canonical.** If the code disagrees with Part II, the code is wrong. If Part
   II seems wrong, **stop and ask** — do not fix it yourself.
2. **Never change a Part II rule without reading its Part V entry first.** Each is the scar
   of a real bug. Most "obvious improvements" here are things we already tried and rejected;
   Part V says why. If Part V doesn't cover your case, that is a real gap — **ask.**
3. **Ask before every real decision.** The owner makes the decisions; you are responsible for
   bringing things to their attention. Explain clearly, in plain words, no jargon, as if to a
   smart new intern. **Do not use metaphors.** Give real context and a recommendation.
4. **Never remove a feature without asking**, even one this document doesn't mention. Some
   may be genuinely important. The deletion list in II.17 is already approved — anything
   beyond it is a question.
5. **Do not invent.** If the spec doesn't say, it's a gap. Ask. Do not guess and do not
   quietly pick a default — that is how this codebase got eleven magic numbers nobody can
   change (V-P5).
6. **Commit at every major step**, with a message that says what changed and what it does not
   yet do.
7. **Check everywhere. We cannot afford bugs here.** This codebase's flagship bug ran
   `apt-get purge` on hundreds of system packages during a routine test.
8. **Report honestly.** If tests fail, say so and paste the output. If you skipped a step,
   say that. If you're unsure something works, say you're unsure. Never describe unverified
   work as done.
9. **A ✅ is earned by a command, not by a belief.** Rule 8 was already here, in these words,
   and **Phases 0 and 1 were both marked ✅ while untrue anyway** — so the rule is not
   enough on its own. **Before writing ✅ on a phase, re-run that phase's Exit criterion and
   paste the result.** Before *trusting* one, re-run it. **A phase that deletes things is
   done when the greps are quiet, not when the new thing works** — Phase 0 and Phase 1 both
   failed exactly here: the replacement was built, the replaced was left standing, the tests
   went green, and green was read as done. **Green means the old code still works. That is
   the thing you were trying to remove.**
10. **At every phase change, run Part VII's audit section.** It is a list of commands, not
    prose. Delete each finding as its command goes quiet — **in the same commit as the fix**,
    because an audit nobody retires becomes the next thing nobody believes.
11. **A green suite is not success. It is the absence of one kind of failure.** The tests
    cannot see the plan. They do not know Phase 0 asked for a deletion, that II.6 asked for
    three verbs and got two, or that the grammar was supposed to *replace* the eight parsers
    rather than become the ninth. **Nothing in this document is verified by `cargo test`** —
    every ✅ that turned out false was green when it was written. So green is a floor, not a
    finding: it says you broke nothing that was already covered, which is the least
    interesting thing you could report and never the thing that was asked. **The question is
    never "do the tests pass?" It is "did I do what the plan said, in full?"** — and that is
    answered by re-reading the phase and checking yourself against it, line by line, not by
    reading a number. A partial implementation passes. A plan followed for three steps of
    five passes. The wrong design, built perfectly, passes.

### How to work

- **Follow Part III's phases in order.** Phase 0 is pure deletion and comes first
  deliberately: do not carefully port something you are about to delete.
- **Phase 2 cannot be split, and the branch is red for a long stretch.** That is expected. Do
  **not** run the old and new models side by side behind a flag — that is the exact "two ways
  to do one thing" disease this whole design cures, applied to ourselves.
- Every phase has an **exit condition**. Meet it before moving on. The exit condition is the
  bar — **not the test suite** (rule 11). Read the Exit lines and notice what they actually
  ask for: Phase 0 wants the codebase *smaller* and a line count reported; Phase 4 wants a
  test **per removal path proving the guard fires**; Phase 6 wants an **air-gapped container**
  to restore. None of those is "the suite is green", and no amount of green implies any of
  them. Phase 1's Exit is the one that reads like tests — "unit tests for every grammar rule
  above, including every error case" — and note that it names a *surface to cover*, not a
  result to observe; note also that **Phase 1 is one of the two phases that was falsely marked
  ✅.** Its tests were written and they passed. The phase still wasn't done, because covering
  the new grammar was never the same as unifying the parsers onto it.
- `cargo test` and `cargo clippy` must be green at every commit outside Phase 2's interior.
  Necessary, nowhere near sufficient: a phase can be green and untouched.
- Part IV lists the specific proofs. They are not optional.

### The three principles that decide arguments

- **Fail loud, never silent.** Every bug in this codebase is the same bug: something didn't
  work and said nothing. Given a choice between a wrong answer and a visible error, take the
  error. Always.
- **There is no legacy.** No users exist. No migration path, no compatibility shim, no
  deprecation warning, no old-format reader. Delete legacy branches on sight.
- **A comment states a constraint the code can't show. Nothing else.** Not what the line does.
  Not where it came from. Not that it's good. This repo has ~884 comments that break this
  rule, written by models congratulating themselves; do not add the next one.
  *(The figure was 139 in the first draft, measured against an older, smaller tree. Re-measured
  2026-07-16 across 2,147 comment blocks.)*

### Lessons from the 2026-07-17 review pass

A five-pass read of the actual code (messages, redundant features, surprising defaults, failure
paths, security) produced the `R*` and `SEC*` lists under **Phase 5**. The lessons behind them:

- **Stale status drifts *both* ways.** This session the HEAD header lied *downward* — it said
  "Phases 3–6 not started" while a dozen Phase 3–5 items were done with commits behind them.
  Re-run the command; never trust a status line's direction. (Reinforces rules 9–11.)
- **`R1–R23` are owner-approved fixes. `SEC1–SEC7` are recorded vulnerabilities whose solutions
  are NOT yet decided — do not implement a SEC fix until the owner rules** (the one exception is
  SEC7, a straight NO-LEGACY delete of dead code).
- **A "feature" that hand-rolls its own transaction/graph parallel to `sync` is a second engine to
  delete, not maintain.** Teleport and the `shim` command were imperative shortcuts for "edit the
  file, sync" — and teleport's private transaction *bypassed the guard* (a real safety hole). When
  you find a command doing the machine's core loop by itself, that is the bug.
- **When you surface a redundant feature, the teardown shape is yours to choose; that it goes is
  the owner's ruling.** State NO-LEGACY and that better code already exists (usually "edit the file,
  sync"); do not agonize over helper-vs-delete.
### The lesson from 2026-07-23, which cost more than the rest combined

**An audit reads what is written; only running it reads what is there.** Thirteen sessions of
review — including one whose entire purpose was hunting false claims in this file — read II.10's
"every removal path calls it", checked it against the seven paths the sentence names, and passed
it every time. The eighth path was never named, so it was never checked, and it was uninstalling
software. It was found in the first twenty minutes of a session that did nothing but *start the
binary*, because `cargo test` could not overwrite a `.exe` a hung LiNix was holding.

Three consequences, and they are rules, not observations:

1. **A list is an assertion about what is absent, and nothing verifies that half.** "Every X does
   Y — A, B, C" is checked by reading A, B and C, which is why the check always passes. When a
   claim quantifies over paths, the work is enumerating the paths *from the code*, never from the
   sentence.
2. **Fix a branch, read its sibling.** S6 examined `heal`'s removal branch and reasoned carefully
   about the guard; the install branch four lines down also removes, and no one read it. This is
   the `command -v` case in `CLAUDE.md` again, in the file that records the `command -v` case.
3. **Recovery paths are removal paths.** Anything that repairs, retries, rolls back, or completes
   an interrupted operation can delete, and every one of them is outside the plan the user read.
   They need the guard *more* than the ordinary paths, not less, because nobody is watching.

- **The security soft spot is the download/link backends.** The core is safe — every PM command is
  argv (no `sh -c`), the II.12 hook ledger is enforced on every path, archive extraction rejects
  `..`. But `web`/`appimage`/`github`/`link` take untrusted URLs and `@`-options straight to the
  filesystem: validate `@`-option paths (no `..`/separators/absolute escapes) and enforce
  TLS+checksum before making a downloaded file executable and putting it on PATH.
