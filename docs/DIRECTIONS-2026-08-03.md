# Direction — 2026-08-03

> **Status: ruled.** This file replaces the speculative version written the same day, which
> proposed three architecture shifts and two dreams. Two of the shifts are dead, one is halved,
> and the two dreams survive. Everything below is either **ruled**, **done**, or **to look
> into** — and each says which.
>
> Rulings were made by the owner on 2026-08-03 and are recorded in
> [`spec/decisions.md`](spec/decisions.md). The rules among them are in
> [`spec/target-state.md`](spec/target-state.md) with their entries in
> [`spec/why.md`](spec/why.md); this file is the map, not the authority.

---

## 1. Done in this commit

**The option lookup can no longer fall through** (was §8a finding 4). `keys_for` matched the
statement kind as a *string* and ended in `_ => SCHEDULE_OPTION_KEYS`. Nothing was wrong on the
day it was written — every caller passed a spelling with an arm — but the tenth kind would have
inherited schedule's options in silence: `@cron` accepted on a thing with no schedule, its own
options refused, no error anywhere. It is an enum now, the match is exhaustive, the default arm
is gone, and the dead `"exec"` arm is reachable because `validate_exec` reads the same table
instead of `EXEC_OPTION_KEYS` directly. `validate_generate` likewise, via an empty
`GENERATE_OPTION_KEYS` — the empty set is a table, not a special case in a validator.

Each validator kept its own hint. Folding three refusals into one generic *"takes: runs, undo"*
would have cost the sentence that says what those options **mean**, and the error messages are
the best thing in this repo — `tests/option_table_coverage_tests.rs` quantifies over
`OptionKind::ALL` so a kind wired to the wrong table (which still compiles) fails.

**The two dead parsers are deleted** (was §8a finding 3). `parsers/brew.rs` and `parsers/nix.rs`
had zero callers and passing unit tests — a green suite over code that cannot run, which is
SPEC.md rule 11's exact counter-example.

They were checked for lost behaviour before deletion, and there was none: the live
`fetch_installed` parses `brew list --versions` identically, and the live `parse_brew_search`
skips the `==> Formulae` headers the dead one skipped. For nix the live code is far ahead — two
JSON layouts across nix versions, against a captured Determinate Nix 3.21.9 fixture.

**One difference did surface, and it is a family question, not a brew one** — see §4.

---

## 2. Ruled: build these

### Legibility is a defect class with rules of its own

**Ruled 2026-08-03.** There are two ways this tool can be wrong. It can do the wrong thing, or
it can do the right thing and **tell you something false about it**. The second is worse: after
the first you can go and look at your machine, and after the second you stop looking.

AU1 was `linix check` printing `ok — the machine matches your files` when it did not. The same
session's own disclosure had `init` scaffold into the live config directory and report
`created` — true about *what*, wrong about *where*. Neither is a crash. Both passed. The defect
is that the person understood their machine **less** accurately after the command than before.

Three rules follow, and they are rules and not a mood:

1. **"Nothing to do" is a claim about the world and has to be earned.** It is the most confident
   sentence the tool ever says and the only output nobody writes a test for. `Declined::reported`
   (`app/sync/planner.rs`) is this rule already built for one path — an empty plan with a
   non-empty `skipped` is not `already up to date`. The rule generalises it.
2. **Every mutation states where, not just what.**
3. **Absence is reported like presence.** A thing not done, not found, or skipped is output, not
   silence.

This is the standard the error messages already meet — file, line, what is wrong, what to do,
*and what the concept means* — applied to success, to absence, and to history rather than only
to failure.

### The plan becomes an object — the internal half only

**Ruled 2026-08-03: build the capability, do not publish the contract.**

Twenty-two backends have ever run against a real package manager. **Thirty never have.** But
most of what a backend is comes down to "given `pipx:black`, what argv do you run" — and that is
a string, checkable with pipx absent. `plan` already computes those strings without touching
anything, which is why plan-smoking covers 45 backends while execution covers 22: the cheap
check already reaches double what the expensive one does.

So: make the plan a stable object the code passes around and can serialize, and assert argv per
backend against it — all 52, any machine, milliseconds, nothing installed. That is coverage for
the thirty backends that are never getting a container image.

**Not** a public versioned schema with a hard refusal on mismatch. Once the format is published
it is permanent under NO-LEGACY, and it buys fleet deployment and PR-reviewable plans, neither of
which anyone has asked for. Registered as **Q26** and **DEFERRED**, not refused — the internal
object is a precondition for it, so nothing is foreclosed.

`model::Resolver` must stay pure for any of this to work. See §5.

---

## 3. Ruled: do not build these

### Shift 1 — deriving ownership from git history — WITHDRAWN

`registry.json` records what LiNix owns, and six subsystems exist to carry, lock, bundle and
restore it. The proposal was to derive ownership from the config repo's git history instead and
demote the ledger to a cache.

**Killed on 2026-08-03, in both its forms.**

The strong form makes git **required for the core loop** — not for `history`, for `sync`. Git is
optional today and X.5 says so deliberately: `core/git.rs` is a dependency-free shell-out, and
its own refusal reads *"install it to use LiNix's manifest history … everything else works
without it."* A git-less machine gets `bundle` instead. Requiring git to uninstall a package
inverts that, and git is not present by default on Windows, in a minimal container, or on a
small server.

The weak form — git as a corroborating second source, intersection governs removal, union
governs reporting — was designed out and then measured against the failures it would catch. It
**misses AU4**, the most recent real instance: a fresh config sandbox with a stale data dir
produced seven phantom removals, and a fresh sandbox has no history to be authoritative with, so
git abstains. It also misses the over-broad `adopt` case `adopt.rs`'s header warns about, because
adopt writes its lines into the manifest and commits them, so git agrees with the wrong ledger.

What is left is "a registry from another machine or another time" — real, but narrow, against a
three-valued ownership check with a four-condition abstention gate, a history walk on the removal
path, and a reconcile mode. **`guard.rs` is already the general brake** and does not care why the
registry is wrong. Registered as **Q25** and ruled **no**; the ledger-demotion and
reconstruction-verb questions fall with it.

*What survives is git as pure enrichment where it happens to be present — see §4.*

### Shift 3 — the tier-1 / tier-2 boundary — DROPPED

Stating in Part II that some declarations can be undone exactly (`link:`, `dotfiles:`,
`setting:`, `@bin`, `exec:` with `@undo=`) and the rest delegate to a manager that mutates in
place, then printing the tier per row in `plan`. **The owner declined it on 2026-08-03.** **Q27**
is ruled **no**. Recorded here so the next audit that notices `setting:`'s read-before-write does
not re-propose it.

### Q29 — freezing the statement set — HALF RULED 2026-08-04

> **The resource-kind half is ruled: the set stays OPEN.** *"i dont think it is closed, no. we
> still might add."* So the recommendation below was right about that half and the ratchet is no
> longer an alternative to the ban — it is the **price** of not banning, and it is built
> (`tests/grammar_table_matches_the_spec_tests.rs`). On its first run it found `generate:`
> missing from Part II's Statements table: **the fourth prefix to ship unlisted, and it had been
> sitting directly beneath the paragraph written about the other three.**
>
> **The computation half is still open** and is not implied by the ruling next to it. Nobody has
> said whether a fourth `vars` provider or another logic keyword may be added.

Not ruled when this was written, and it is written down because the file it came from is being
replaced.

The proposal was to declare the config language closed and route all future computation through
`generate:`. **The question has a hole in it:** `generate:` output is merged *"as if typed"*, so
it goes back through this same grammar — a generator can emit a thousand `apt:` lines it
computed, and **cannot emit a statement kind that does not exist**. Generators expand quantity,
not kind. `generate:` is also off by default behind `allow_generators` and runs through the II.12
ledger, which makes it a fine escape hatch and a weak policy.

So it splits, and the halves deserve different answers:

- **Is computation closed?** No more logic keywords, no fourth `vars` provider, no `repl`. This
  is what `generate:` genuinely covers. **Recommended: yes.**
- **Is the resource-kind set closed?** Never another `foo:` prefix. Nothing absorbs the twelfth
  kind if that is wrong, and extensibility grades A− precisely because the backend mechanism is
  open. **Recommended: no**, with the ratchet below as the gate instead.

Either way, the three Part II misses that motivated the freeze (`exec:`, `dotfiles:`,
`firewall:` all shipped absent from the statement table) have a better cure than a ban:
`KEYWORDS` at `statement.rs` is already the single list — its own comment records that three
copies had drifted until `setting:HKCU\Software\Foo` was read as a set difference by the one that
had never heard of `setting:`. **A test asserting `KEYWORDS` matches the Part II table** makes
the twelfth keyword impossible to ship undocumented without banning it. An afternoon.

---

## 4. To look into

### `why` grows a chain

> **Answered 2026-08-04: it is already built.** Measured by running the verb, not by reading
> it — see §6 step 3. What follows is the question as it was asked.

**The owner asked for this to be investigated.** `linix why curl` should answer with the whole
story, not a fact: declared at `modules/dev.txt:12`, in module `dev`, pulled in by profile
`Workstation`, active because this host is `thinkpad`, added in commit `a3f9c`, and here is the
message you wrote.

It is nearer than it looks and it is **the surviving half of Shift 1** — git as enrichment, never
as authority. Nothing votes on ownership, nothing has to be reconciled, and on a machine without
git you get the config half of the sentence and no error.

What already exists: the registry stores `source: …/modules/starter.txt:1` and
`__scopes: module:starter;profile:Main`, so the provenance is being captured today. `why` is a
verb (`cli/args.rs:738`). `git.rs::commit_manifest_changes` already extracts the package-level
delta of a commit. **The first step is to find out how much of the chain is already reachable and
how much is missing** — a measurement, before any design.

This is rule 1 of the legibility ruling pointed at a single verb, which is why the two are one
piece of work seen from two ends.

### Compaction — DRY and SOLID, measured

> **Begun 2026-08-04.** Move 1 (`Ledger<T>`) is done as the `LockFile` trait. Move 2 is begun:
> the ratchet exists and 2 of 29 backends are converted. Move 3's re-measure is §6 step 7. The
> honest number below (~1,950 lines, ~3%) is unchanged and still the wrong reason to do it.

**The owner asked for this to be investigated.** Not "make it smaller" — **89,592 lines across
204 files** as of this commit, and a codebase that got shorter by collapsing its seams would be
worse. The target is **each rule stated once, behind an abstraction still shaped to extend**.

*(The old file said 88,599 across 206 on `ee5adf3`. Two files are gone and the tree grew ~1,000
lines in between — which is the §6 step-7 practice arriving before the practice did.)*

The measured candidates, in the order they should be taken:

| # | Move | Non-test lines | Buys |
|---|---|---|---|
| 1 | `Ledger<T>` behind the six `*_lock.rs` types | ~−240 | The seventh ledger inherits the file rules |
| 2 | Convert the ten formulaic language backends to `ManagerConfig` | ~−1,500 | Adding a backend becomes adding data |
| 3 | Re-measure, then decide about the other eleven with a number in hand | — | — |

**Why the duplication exists decides the remedy, and the first draft got it wrong.** This was not
written by something copying its neighbour: `backends/capability.rs` says *"the two lists are one
list, asserted equal by a test below"*, and **all six** `*_lock.rs` files honour the dry-run
rule — not four of six. A copy-and-edit process leaves that bug in half the family. The rule was
found once and fixed everywhere it lived.

So the diagnosis is a **cost asymmetry**, not blindness. Correcting a family's *semantics* is
cheap and `CLAUDE.md` demands it, and it happened on schedule. Collapsing a family's *carrier* is
a large cross-cutting change under *no change breaks existing code*, in a repo where a green
suite is explicitly not evidence — and it never had a session where it was obviously the right
thing to spend the day on. Every centralisation here is downstream of an incident, and structural
debt never produces an incident.

**Which fixes the prescription: make the structural move small, ordered and pre-authorised.** The
one test that makes it permanent, in the style of `absent_marker_coverage_tests.rs`:

> Every backend registered on this host either is built from a `ManagerConfig`, or appears in an
> explicit list of exceptions, each with a one-line reason.

The list starts long and gets shorter, and converting backend #12 becomes a chore with a visible
finish line rather than a refactor somebody has to justify. **Write the ratchet before converting
anything** — it is worth more than the conversions, and it is what stops backend thirty-nine from
being written by copying thirty-eight.

Honest number: roughly 1,950 non-test lines, about **3%**. The reason to do it is the second
column, not the first. Step 2 must go **one backend at a time against the container harness** —
a converted backend nothing ran is a rewrite nobody verified.

### Should the hand-rolled parsers sanitize?

> **Answered 2026-08-04: yes, and at the boundary rather than per parser.** `run_output` and
> `search_output` sanitize now, so a backend inherits it by reading output the way everything
> else does. The scan written to prove it found **five more sites this section never listed**,
> two of them production — including `tool_help.rs`, which reads a tool's `--help`, the output
> most likely of all to be coloured. `sanitize` moved to `utils/text.rs`, the layer `core` may
> depend on.

Fell out of deleting the dead parsers. They called `sanitize()` — ANSI-escape stripping — and the
live code does not. Not a brew or nix issue: **all sixteen table-driven backends sanitize, and
the ~14 parsers hand-rolled inside `src/backends/` mostly do not** (`cargo.rs`, `go.rs`,
`yarn.rs`, `storage.rs`, `flatpak.rs` and `snap.rs` being the exceptions that do).

Measured, not assumed: inside `src/backends/`, **only `flatpak.rs` and `snap.rs` call
`sanitize()`** — `brew.rs`, `nix.rs`, `cargo.rs`, `go.rs`, `yarn.rs` and `storage.rs` hand-roll
their parsing without it, while every function in `src/parsers/` sanitizes.

No case found where it bites today — these managers do not colour a pipe. It is an inconsistency
across a family of thirty, which is `CLAUDE.md`'s shape, and the `ManagerConfig` conversion above
dissolves it as a side effect: a converted backend has nowhere to put a parser except
`src/parsers/`.

### The backend count nobody agrees on

> **Adjudicated 2026-08-04, and nobody was wrong.** *"Registered" meant two different things.*
> **62 backends are compiled into the build**; how many *register* is host-dependent, because
> the OS-native ones sit behind `cfg!(target_os = …)`. 48 (Windows) and 56 (Ubuntu) are both
> correct answers to the second question and belong in a grade, which is a dated measurement of
> one machine. `SPEC.md`'s 52 answered neither and is corrected; the 62 is now asserted against
> the argv table, which works because every registrar is already required to have a row there.

`SPEC.md` said 52 registered and 22 ever run; the grade said 48 registered and 24 live on this
host. Neither was obviously stale and no one had adjudicated them. Three places counted the same
thing and no two agreed — the §2 disease, in the documentation register.

---

## 5. What must not be coalesced

Carried forward unchanged, because it is the part of the old file most likely to be violated by
someone acting on §4.

**`app/context.rs::App` is already too big — do not feed it.** Forty-five public methods,
twenty-eight dependents. Half are honest composition-root accessors; the other half is business
logic that landed on the god object because it was reachable. **If deduplication finds a shared
helper and the nearest home is `App`, that is a signal to make a new type, not to add method
forty-six.**

**`app/sync/guard.rs` is one chokepoint on purpose.** It exists because of VI.0. *A guard on one
command is a guard on nothing.* 1,276 lines, and it stays one file.

**`model::Resolver` and `app::sync::StateResolver` are not two implementations.** The names
invite the "two of everything" reflex and it is wrong here: `model::Resolver` is pure — layout
in, `DesiredState` out — and `StateResolver` is the async shell that gathers vars, facts and
priority and delegates. Pure core, imperative shell. **Merging them is the single worst change
available in this file**: it would put I/O inside the only component testable without a machine,
and §2's plan-object work rests entirely on that component staying pure.

**Test volume is not the target.** The quantifying tests — `absent_marker_coverage_tests.rs`,
`argv_drift_tests.rs`, `dry_run_every_verb_tests.rs`, `lifecycle_coverage_union_tests.rs`,
`fanout_cap_reads_the_setting_tests.rs`, `config_root_is_absolute_tests.rs`,
`startup_budget_tests.rs`, and now `option_table_coverage_tests.rs` — are the mechanism that
makes everything above safe. **They get bigger, not smaller.**

---

## 6. Sequencing

*Status added 2026-08-04. The full account is the `2026-08-04` entry in
[`spec/history.md`](spec/history.md); this is the scoreboard.*

1. **Done 2026-08-03:** the option lookup, the dead parsers, the coverage test.
2. **The `KEYWORDS` ↔ Part II ratchet** — **DONE.** It found `generate:` missing from Part II's
   Statements table on its first run: the *fourth* prefix to ship unlisted, directly beneath the
   paragraph recording the other three. Q29's resource-kind half was ruled the same day —
   **open, more prefixes may be added** — so this is the load-bearing half of that ruling rather
   than a way to retire the question.
3. **Measure the `why` chain** — **DONE, and the answer is that it is already built.** Measured
   by running the verb against an adopted sandbox config, not by reading it: file and line,
   module, profile, introducing commit with date and message, provenance, artifact rule, lease,
   reverse dependencies, and each `when` with the current value of every variable it tests.
   Nothing was missing. The measurement was.
4. **The `ManagerConfig` exception ratchet** — **DONE**, and it is
   `tests/backend_is_data_not_code_tests.rs`. Every backend module is data or is named with what
   the generic machinery cannot express; "not converted yet" is rejected as a reason. Started at
   29 entries.
5. **The plan object, and argv assertions per backend** — **the object already existed**
   (`SyncReport`: serializable, passed around, carries `skipped`). The argv coverage it was
   wanted for is **done and larger than asked**: 62 backends rather than 52, any machine,
   milliseconds, nothing installed — through the widened argv table in §4's ratchet rather than
   through a serialized plan. Putting argv *into* `SyncReport` is not done: that changes
   `--json`, which is user-visible, and **Q26** is still deferred.
6. **`Ledger<T>`** — **DONE** as the `LockFile` trait, with a ratchet against a seventh ledger
   hand-rolling its own carrier. **The backend conversions are begun, not finished:** `krew` and
   `pubdart` are data now (390 lines of Rust → two rows), 6 remain marked `TO CONVERT`. Each
   conversion cost one new `ManagerConfig` field rather than a lost behaviour — `extra_probes`
   and `upgrade_reinstalls_each` — and both are now available to every backend.
7. **Re-measure and report the line count** — **DONE, and standing.** `src` 89,592 → 89,849 (same
   204 files); `tests` 15,759 → 16,893 (67 → 72 files). Net **+1,391**, and what it bought: argv
   assertions for 62 backends where 32 had them, five ratchets that did not exist, and ~560 lines
   of duplicated carrier and hand-written backend deleted.

**What the scoreboard is actually recording.** Three of the five open steps were already built
and simply unwritten, and five new ratchets were written of which **four found a live defect the
day they were written**. Neither of those is about carelessness. Every rule involved had been
found once and stated correctly — a paragraph instructing that a table "must be checked against"
the code, six correct copies of a dry-run rule, sixteen backends that sanitize — and then left to
be enforced by memory. *A prose instruction to check a copy against its authority is not a
check.* It is a copy of the authority's address, and it decays faster than the thing it protects,
because it reads as though the work has been done.

The legibility rules in §2 are not a step; they are a standard the steps are held to.
