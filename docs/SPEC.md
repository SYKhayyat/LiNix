# Shall v7 — the declarative model

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
| [`spec/bugs.md`](spec/bugs.md) | VI | Bugs killed by this design, and bugs carried forward. | Before building anything. |
| [`spec/decisions.md`](spec/decisions.md) | — | **All 208 decisions. 193 ANSWERED, 2 PARKED, 1 DEFERRED, 1 HALF RULED, 10 BUILT NEVER RULED, 1 OPEN.** Counted, not typed — `scripts/decision-count.sh --check`. | Before proposing anything. |
| [`attic/lessons.md`](attic/lessons.md) | — | Thirty-one lessons, each the residue of a shipped defect. For a person, once. | **Never.** It says so at the top, and it means agents. |

**Parts VIII–XIII were proposal documents — artifacts and channels (`D1`–`D17`), `when`
variables (`W1`–`W14`), rebuild and caches (`K1`–`K18`), `firewall:` (`N1`–`N7`), secrets
(`T1`–`T7`) and the next round (`U1`–`U39`). Every one of those decisions is ruled and every
rule they produced is in Part II, so the documents were deleted (`Y21`). The IDs still resolve:
they are entries in the register.

**Where the work stands (updated 2026-07-26).** Phases 0–6 are built and the container matrix
(ubuntu/fedora/arch/alpine/tools) is green, run for real. **Phase 7 and the entire U-series
backlog are built** — the provider mechanism (snapshots U27/U28/U29, init U36, storage U30,
secrets U38), and the language-power features (module parameters U32, generated declarations U33,
user verbs U35, `repl` U34). **The last two open items — `D5` and U27's "built-ins become snapshot
rows" half — were cleared to build by the owner decision session 2026-07-26 and are built now, not
parked on hardware** (Option A: typed-placeholder Windows row; see `plan.md` Tier 1 and Tier 3
item 6). What the real machine still owes is validation, not construction: D5's live `dpkg`/`rpm`
install and the Linux snapshot providers' live restore. **VI.0 — the bug that removed software
with no guard, no plan and no count, and that `--dry-run` performed — is FIXED** (S24/S25,
2026-07-23, verified 2026-07-24).

**On "the suite is green" — read this before quoting it.** Green here had always meant *on the
developer's Windows box*, and on 2026-07-26 that turned out to be load-bearing. `origin/main` was
112 commits behind `HEAD` until that day — the entire U-series, D5 and the provider mechanism had
never been compiled on Linux or macOS. The first CI run that saw them **failed on all three
platforms, on two different bugs**: Windows on a test that commits to a git repo and unwraps
(**S33** — it passes wherever a global git identity exists), Linux and macOS on a test asserting
Windows path semantics everywhere (**S34** — nobody had seen it, because the first red job hid the
second). Both are fixed, at the mechanism rather than the instance: the test binaries now read no
host git config at all, and the platform-specific assertion is behind `#[cfg(windows)]`. **The
suite is now green on Linux too, proven in a container with no git identity — 1,307 tests, 0
failures — including the `tests/` binaries that `cargo test --lib` never reaches.** That last
point is why this paragraph was wrong for a week: it cited `cargo test --lib`, which does not run
the binary where S33 lived.

**Build state is not readiness, and this file should stop implying it is.** The register is at
zero unbuilt items; the *validation* surface is far narrower than the build surface. **62 backends
exist across all platforms** and 23 have ever been run against a real package manager — 7 per
distro image, 18 in the `tools` image, `scoop` on the native Windows sweep, `brew` on the macOS
one, **45 plan-smoked** on any one image.

*"Registered" meant two things and three documents counted two different ones.* This file said 52
while the grades said 48 (Windows) and 56 (Ubuntu), and no two agreed because the word did not
mean the same thing twice: **62 backends are compiled into the build; how many *register* is
host-dependent**, because `create_default_registry` gates the OS-native ones behind
`cfg!(target_os = …)`. 48 on Windows and 56 on Ubuntu are both correct answers to the second
question. The 62 is asserted against the code by
`tests/backend_count_matches_the_spec_tests.rs` — a number in prose is a copy, and this one had
been wrong long enough that nobody could say which of the three was stale. Since 2026-07-26 `tools` and `gentoo` run nightly rather than on manual dispatch and
`fedora` joined the per-push matrix, so the widest run happens without anyone pressing a button.

**macOS has run.** `macos-native` went green on 2026-07-27 (run `30237464029`, `pass=263 fail=0
soft=6`), which is what `brew` — the one manager `release-check.sh`'s Darwin branch sweeps for
real — moves from *compiled* to *exercised*, and takes the count of backends that have ever run
against a real package manager from 22 to **23**. The destructive effectors — btrfs/zfs/lvm
restore, D5's `dpkg -i`/`rpm -U` handoff, U30 storage removal — are argv-tested and unrun.

> **This paragraph was false for eleven days and 228 commits, and it sits four lines under a
> sentence about exactly this.** It said macOS *"has never been run"* and that its job *"has not
> yet gone green"*, while the session record for 2026-07-27 — four lines further down the same
> tree — had the green run in it. The `62` beside it was right the whole time, because
> `tests/backend_count_matches_the_spec_tests.rs` asserts it against the code and this had
> nothing. **Where this file is checked it is true, and where it is prose it is not** — so read
> `23` as the weaker kind of claim, sourced from the harness's own Darwin canary rather than from
> a gate, and go to the run before you quote it.

**For what remains to build and in what order, read the ordered list at the top of
[`spec/plan.md`](spec/plan.md). It is the only list of build state** — the register says whether
a question is decided and stops there, deliberately, because the last time two files both tracked
what was built they disagreed for two days and the plan lost.

Facts marked **(measured)** were verified against real containers or real code with a citation.
Everything else is design.

Supersedes the v6 audit that found all of this, except where [`spec/bugs.md`](spec/bugs.md)
carries an item forward explicitly.

**Citations to `docs/archive/`, `spec/proposals/`, `spec/history.md` and `INEFFICIENCIES.md`
name files this repository no longer has.** They were cut on 2026-08-08 (`Y21`); every one is in
git, and `git log --diff-filter=D --name-only` finds the commit that removed it. The reasoning
worth keeping from them is thirty-one lines in [`attic/lessons.md`](attic/lessons.md), which is
not for agents to read.

---

## PROMPT — read this first, then follow it

You are implementing Shall v7 on `main` — the sole branch — at `C:\Users\Administrator\Videos\Nexus\shall`.
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
3. **Build without stopping for permission — and stop for exactly four things** (owner ruling,
   2026-07-23; this replaced the older "ask before every real decision", which had people
   stopping on file layout and test structure). **Stop and ask for:** anything with an ID in the
   register (`D*`, `W*`, `K*`, `N*`, `T*`, `U*`); anything that changes behaviour a user would
   notice; anything that would remove a feature; anything where Part II looks wrong. **Do not
   stop for** implementation detail, naming, file layout, test structure, or a choice between two
   options that is invisible from outside the program — make the call and put the reasoning in the
   commit message. When you do ask, explain in plain words, no jargon, as if to a smart new
   intern; **no metaphors**; real context and a recommendation.
4. **Never remove a feature without asking**, even one this document doesn't mention. Some
   may be genuinely important. The deletion list in II.17 is already approved — anything
   beyond it is a question.
5. **Do not invent a rule; do decide a detail.** If the spec doesn't say and the answer would be
   *visible from outside the program*, it is a gap — ask. If it is invisible from outside, decide
   it and record why in the commit. What is banned is the quiet default nobody wrote down: that is
   how this codebase got eleven magic numbers nobody can change (V-P5).
   **When a question is answered, the ruling ships in the same commit** — rewritten into
   `decisions.md` *and its index*, and into `target-state.md` plus `why.md` if it is a rule rather
   than a detail. A ruling that lives only in a chat log is the drift that made 84 decisions
   unanswerable; a ruling that lands in an entry but not in the index is the drift that made the
   register advertise 59 open questions it had already closed.

   **The same rule for a *finding*, and for the same reason.** A review round's output is a diff
   to `target-state.md` plus a test that fails without the fix — not a new dated document.
   Twelve dated review files in nine days is what the other way produced, and F-8 counted what
   they bought: `cargo fmt --check` went 26 → 0 → 0 → 0 → 12 → 60 across them, closed at the
   mechanism and the mechanism never run; `G-4` was closed with a mutation test its author
   watched go red and reopened two days later, same ID, same defect; *"a check that cannot
   fail"* appears in all seven grade rounds. **The finding was written down every time. Writing
   it down is not the mechanism.** Those twelve files were deleted on 2026-08-08 (`Y21`); they
   were a record of how the program got here and never instructions.
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
  Not where it came from. Not that it's good. This repo had ~884 comments that break this
  rule, written by models congratulating themselves; do not add the next one.
  *(139 in the first draft; ~884 across 2,147 comment blocks on 2026-07-16. **Re-measured
  2026-07-26: `src/` carries 9,572 comment-block lines** (`grep -rhE '^\s*(//|/\*|\*)'`). The 884
  figure is historical, not current. The marketing/self-congratulation subset the R1–R23 and F5
  passes swept is now confirmed clean — a grep for the sales vocabulary (`blazing`, `world-class`,
  `enterprise-grade`, `mission-critical`, `seamless`, `bulletproof`, …) finds nothing; the only two
  `magic` hits use the word pejoratively (V.83's "deciding by extension is magic that silently
  writes plaintext"), which is a constraint, not praise. What no grep can measure — a comment that
  narrates the line below it rather than stating a constraint — stays a per-comment judgement call,
  and the codebase's comment-audit passes (R14, F5) are where it is worked, not a single sweep.)*

### Lessons from the 2026-07-17 review pass

A five-pass read of the actual code (messages, redundant features, surprising defaults, failure
paths, security) produced the `R*` and `SEC*` lists under **Phase 5**. The lessons behind them:

- **Stale status drifts *both* ways.** This session the HEAD header lied *downward* — it said
  "Phases 3–6 not started" while a dozen Phase 3–5 items were done with commits behind them.
  Re-run the command; never trust a status line's direction. (Reinforces rules 9–11.)
- **`R1–R23` are owner-approved fixes — all done 2026-07-19. `SEC1–SEC7` were recorded
  vulnerabilities held back for a decision, and that decision has been made: all seven are now
  closed** (SEC1/SEC2 landed 2026-07-19, SEC4–SEC6 the same day, SEC7 deleted as dead code,
  SEC3's confinement half ruled **won't-fix** with only the outside-home confirmation built).
  *This bullet said "NOT yet decided — do not implement a SEC fix until the owner rules" for a
  week after the owner ruled and the fixes shipped.* The standing rule it encoded is still
  right and still applies to the next one: **a recorded vulnerability is not a licence to invent
  a fix** — the shape of the defence is the owner's call, because every one of these has a
  cheap version that closes the report and leaves the class.
- **A "feature" that hand-rolls its own transaction/graph parallel to `sync` is a second engine to
  delete, not maintain.** Teleport and the `shim` command were imperative shortcuts for "edit the
  file, sync" — and teleport's private transaction *bypassed the guard* (a real safety hole). When
  you find a command doing the machine's core loop by itself, that is the bug.
  **The follow-up matters as much as the finding:** what had to go was the private *engine*, not
  the verb. `teleport` was later re-added as `retarget` + `handle_sync` — a line edit that syncs,
  behind the guard like everything else — and it is in II.8's table today. `shim` did not come
  back, because `shim:` as a line already covers it. **"Delete the second engine" is not "delete
  the convenience"**; the test is whether the command routes through `sync`.
- **When you surface a redundant feature, the teardown shape is yours to choose; that it goes is
  the owner's ruling.** State NO-LEGACY and that better code already exists (usually "edit the file,
  sync"); do not agonize over helper-vs-delete.
### The lesson from 2026-07-23, which cost more than the rest combined

**An audit reads what is written; only running it reads what is there.** Thirteen sessions of
review — including one whose entire purpose was hunting false claims in this file — read II.10's
"every removal path calls it", checked it against the seven paths the sentence names, and passed
it every time. The eighth path was never named, so it was never checked, and it was uninstalling
software. It was found in the first twenty minutes of a session that did nothing but *start the
binary*, because `cargo test` could not overwrite a `.exe` a hung Shall was holding.

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

- **The security soft spot was the download/link backends, and that batch has landed.** The core
  was already safe — every PM command is argv (no `sh -c`), the II.12 hook ledger is enforced on
  every path, archive extraction rejects `..`. The rest closed across 2026-07-19 and 2026-07-23:
  **SEC1** `@bin` confinement (`[guard] confine_bin`, default on), **SEC2** HTTPS + checksum by
  default with `@allow_http` and `@unverified` as separate, never-implied opt-outs, **SEC4–SEC6**
  the injection/module-name hardening, **SEC7** the dead Lua exec path deleted. **SEC3 is decided
  as won't-fix:** `@target` stays unconfined — placing files outside `$HOME` is the feature — and
  only the outside-home confirmation was built. The secrets defects that outlived them are also
  fixed: **T2** (a decrypted secret refused a destination inside the git repo, checked *before*
  the tool is launched), **T5** (the plaintext is restricted before it exists, on all three
  platforms, with the Windows ACL done rather than excused) and **T1** (decrypt mode never backs
  up, so the previous secret cannot be left in plaintext beside the new one).
  **What is still owed here is not in the download backends:** U31's ruling that a health-check
  command rides the II.12 ledger is unbuilt, so an `@health=` command arriving with a pulled
  config runs unapproved. That is the one runnable thing in the tree that the ledger does not see.
