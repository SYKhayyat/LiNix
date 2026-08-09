# Lamdan — LiNix, whole repo

**2026-08-09.** Third full pass. Eleven regions, every tracked file in exactly one, each read by a
separate reviewer with no knowledge of the others' conventions, then cross-examined by me against
the source. Scoped to **readme, code and tests** by request — `docs/` was not read this run.

**This run was given a ban-list.** Every finding from 2026-08-05 (`F-*`) and 2026-08-07 (`LX-*`)
was handed to each reviewer as *do not re-report*, along with the six things those runs defended as
correct. A third pass that reproduces the first two is selection bias wearing a review's clothes.
Nothing below is a repeat; where a prior finding's *fix* turned out to be partial, it says so and
says how far it reached.

**The standing owner ruling (2026-08-07) applies unchanged: no capability is lost.** Every `delete`
below means *delete the second implementation of something that already works*. No verb, backend,
statement kind or reachable behaviour is cut anywhere in this document.

---

## Coverage

| Region | Scope | Read |
|---|---|---|
| 1 | `src/config/**` — grammar, options, statements, settings | all 10, full |
| 2 | `src/model/**` — resolution, vars, profiles, edit | all 30, full |
| 3 | `src/backends/{registry,generic,capability,onboarder}` + shipped TOML | full, incl. `registry.rs`'s 4,856 |
| 4 | the 22 hand-written backends + `artifact/**` | all 30, full |
| 5 | `src/parsers/**` + the 47-file fixture corpus | all 14, full |
| 6 | `src/core/**` + `src/utils/**` | full |
| 7 | `src/app/sync/**`, `src/app/apply/**`, `context`, `adopt` | full — **including the 1,630 lines of `resolver.rs` the last run never reached** |
| 8 | `src/main.rs`, `src/cli/**`, `src/verbs/**` | all, full |
| 9 | `src/app/**` satellites | all 30, full |
| 10 | `tests/**` | **all 99 binaries, full** — last run's acknowledged gap, closed |
| 11 | CI, Docker, scripts, `Cargo.toml`, packaging | full |

**Excluded, deliberately:** `Cargo.lock` (queried, not read), `target/`, `docs/` (out of scope by
request), the 47 fixture bodies (read through the assertions that consume them), `.idea/`,
`.aider.*`, prior `lamdan/` output.

**Nothing was built or run.** Every claim is static reading. Every claim I lead with I re-verified
myself against the source rather than taking a reviewer's word — and three reviewer claims died
under that check. They are listed in *What I got wrong*, because a review that only reports its
survivors is not reporting its method.

---

## §1 — What I committed to before opening any implementation

*Written from the readme, the file list and the test names, before a single implementation file.*

> The want, laddered: *drive package managers* → *someone with four machines and no record of why
> anything is installed* → **a machine you can rebuild from a text file**. Minimum that satisfies
> it: a file of `manager:name` lines and a set difference.
>
> So: one binary. A `Backend` trait — `list_installed` / `install` / `remove` — with every backend
> a **data row**, never a Rust file, because the only genuinely hard part is that 60 managers print
> 60 different things. A line parser. A differ producing `(to_install, to_remove)`. An applier with
> a removal guard. ~10–12k lines. No DAG, no scripting engines, no TUI, no fleet/bisect/repl.
>
> And three things I predict will break that sketch: **(1)** you cannot compute removals from
> `installed − declared`, because that set contains the kernel — so there must be a third file
> recording what LiNix installed, and that file is a *liability*, not a convenience; **(2)**
> `ripgrep` is `BurntSushi.ripgrep` on winget, so bare names need a resolution order that must be
> *recorded* or the same file means two machines; **(3)** the removal set is the product and
> everything else is plumbing around one dangerous number.

**How it did.** The three predictions were right and the codebase argues them harder than I did —
`adopt.rs:19-21` says *"an over-broad adoption is not a cosmetic mistake; it is a queued mass
removal."* The data-row prediction was right and is now a shipped ratchet
(`tests/backend_is_data_not_code_tests.rs`), which took 29 hand-written backends to 18. The
"no WAL" prediction was **half right and I had the halves wrong** — see *What I got wrong*. The line
count was wrong by 8×, and §"Why not two txt files" is my accounting for where it went.

---

## Why this couldn't be two txt files and a list of added and deleted things

You asked the question directly, so it gets a direct answer rather than a defence of the status quo.

**The core is exactly that small.** The set difference is `planner.rs:687-736` — **fifty lines**.
Everything my sketch predicted is there and it is tiny.

Five things break the two-file model, and each one is a real cost, not an indulgence:

**1. The third file, and it is a liability.** `installed − declared` is your operating system.
So there must be a record of what LiNix installed. The alternative is not hypothetical — it ships,
as `purge-undeclared`, and `context.rs:797-801` records the measurement: **476 packages on stock
Ubuntu against 103 for `adopt`.** The registry is what makes that number 103. And because the
registry can be *wrong* — stale, mis-scoped, from another machine — everything called "the guard"
exists to survive it. That is not a feature bolted on; it is the mandatory consequence of the third
file.

**2. `installed` is not a set. It is 62 managers that can each fail to answer** — and the correct
default for "I could not ask" is *opposite in the two directions*. For a removal, unknown must mean
*yes, it's installed* (`planner.rs:604`) or one flaky manager silently stops LiNix removing anything
through it. For an install, unknown must be a hard error (`planner.rs:880-886`) or you schedule an
Install node for every managed package, each a trivial success, and one later failure rolls back the
set. A set difference has nowhere to put that.

**3. The removal set is not `managed − declared` either.** It is that, filtered four ways
(`planner.rs:511-548`) — backend in `priority`, backend on this machine, not protected, not already
scheduled — and each filter produces a **reported** `Declined`, not a silent `continue`. Two of
those were bare `continue`s once, and the machine kept a package forever while `sync` said success.

**4. A diff is a set; applying it is a sequence.** apt before cargo, because a crate needs a system
compiler and no system package has ever needed a crate. The fstab entry out before the volume is
destroyed, or the machine stops in initramfs. Repos before packages.

**5. `exec:` has no inverse.** A script that ran cannot be un-run. It is the one statement kind that
genuinely breaks the model, and the codebase knows it — `@undo=`, run counts, "a false `when` does
not mean undo".

**So the honest answer is: it can't be two txt files, but it could be about three files and
12–15k lines.** The interesting question is what the other 85k bought — and the answer, region by
region, is *not features*. Region 3 measured `registry.rs`: **4,856 lines, of which roughly 200 are
irreducible Rust and 444 are literally `field: None,`.** Region 5 measured the parsers: 1,772 lines
of code driving 62 managers, wearing 3,100 lines of prose and tests. The bulk is not complexity. It
is **the same fact written down in more than one place**, which is the subject of everything below.

---

## The strongest claim

**LiNix's characteristic move is to turn a property into a type. Its characteristic failure is that
the type stops at a module boundary and continues on the other side as a string or an integer — and
every single region found an instance without knowing the others had.**

That is not a metaphor. It is eight independent instances, and they are the top of the findings list:

| The property, as a type | Where it becomes a string or a number | What is lost |
|---|---|---|
| `GuardScope` | `scope_label()` → `&str` → `guard_scope()` (`verbs/sync.rs:41`, `apply/firewall.rs:275`) | **Both named arms are unreachable.** The producer emits `"an unattended watch tick"`; the consumer matches `"watch"`. |
| `max_removals`, a ceiling over the whole command | `also_removing: usize`, assembled by three callers, one of which passes `0` (`apply/firewall.rs:151`) | 4 packages + 4 ports under a limit of 5. **No guard call ever sees 8.** |
| `Phase`, exhaustive, compiler-enforced | three dispatches on a `&str` split out of a ledger key (`extras.rs:243`, `:330`) | `_ => warn; Ok(())` reports a teardown as done, and the caller clears the ledger row. |
| `Statement::key()`, one producer | nine sites re-split it on `:` — and `kind()`'s own doc forbids exactly that (`statement.rs:247`) | the format is now persisted in `locks/extras.toml`, so it is on disk. |
| `Commands`, an exhaustive enum | `READ_ONLY_COMMANDS`, a `&[&str]` of 21 names (`main.rs:627`) | `history` is on it, and its TUI runs a **full rollback + sync with no data lock held**. |
| the capability set, derivable from `ManagerConfig` | `.with_upgradable(…)`, hand-wired 22 times | winget and scoop declare `upgrade_args` and get no `Upgradable` — **and a test pins the loss as correct.** |
| `ParseResult` / `Unrecognised` — "I read bytes and understood none" | `or_unrecognised`'s JSON arm returns `Ok(empty)` (`parsers/mod.rs:124-132`) | five language backends report an empty machine on any schema change. |
| `Options`, a closed validated table | `__source`, `__gated_by`, `__from_regex` injected into the user's own namespace | keys `validate_options` would refuse on a user's line, then re-parsed with `rsplit_once(':')`. |

I counted the shape directly: **52 non-test dispatch arms in `src/` that end in `warn!`, `None`,
`Ok(())` or `continue`.** Each is a place where an exhaustive match upstream stops being exhaustive.

**Why this is the right frame and not just a list.** The repo's own doctrine is *"Prefer deleting to
fixing… when you find a second implementation of something, the task is to remove one."* That
doctrine is being followed — `Reaped`, `PlanScope`, `Phase`, `HostBackends`, the data-row ratchet are
all *exactly* this technique applied successfully. The disease is not that the team doesn't know the
cure. It is that **the cure is applied to one property at a time, and the boundary the type has to
cross is the one place nobody looks.**

---

## Lens 1 — what was built

**The artifact is right, and I am not relitigating it.** The 2026-08-05 argument that `firewall:`,
`service:`, `setting:` and storage are other products wearing a package manager's clothes is dead by
owner ruling. The 60-verb CLI beats my four. The hand-written line grammar beats a parser combinator,
and I checked: 70 `GrammarError::new` sites, 63 distinct hints, and for a config language those hints
*are* the product.

**Where lens 1 does not hold, it is the same finding four times: something was built twice, and the
second copy is the one without the reasoning.**

- **The parser data path exists, ships, is documented to strangers, and no built-in uses it.**
  `LX-4` fixed the *argv* half — conda is a row now, and the ratchet is real. It did not touch the
  *parser* half. `ParserSpec` (`onboarder.rs:70-105`) has four arms, correct fallibility, and
  `grep -rn ParserSpec src --include=*.rs | grep -v onboarder.rs` returns **one line, and it is a
  comment.** A stranger adding manager #63 writes six lines of TOML. The maintainer writes a Rust
  function, a registry closure, and a hand-typed test that — by this region's own published
  experiment — cannot detect the failure it exists to prevent.
- **The registry is 96% data with a type checker's ceremony around it**, and the ceremony is not
  guarding the thing that broke. `register_generic`'s three booleans are 24 hand-passed restatements
  of data three lines above them; `base_config` exists and 22 of 46 registrars ignore it, which is
  where the 444 `field: None,` lines live.
- **`model/` is a folder, not a layer.** Sixteen of its thirty files have no relationship to
  resolution — `resolve.rs` imports none of them. The filing rule is "pure-ish", and `cache.rs:70`
  deletes files, `vars_provider.rs:132` spawns subprocesses, `vars_embedded.rs:49` runs a Rhai engine
  with `http_get` enabled, and `script.rs:15` says outright *"Not pure."* Meanwhile `resolve.rs:744`
  calls `crate::app::profile_expr::evaluate` — the resolved model depending upward on the application.
- **The grammar can read a line and cannot write one.** There is no `Display for Statement`. So every
  writer reconstructs the format by hand, and the comment rule now has **three implementations, two
  of them wrong**: `grammar/mod.rs:336` requires whitespace before the `#` (correct, and it was
  written because `@content=#!/bin/sh` was being truncated); `model/edit.rs:610` uses
  `raw.find('#')`; `model/groups.rs:38` uses `line.split('#').next()`.

**The deletion test, where it bites hardest.** `bisect.rs:20-37`'s `first_bad` is pure, has five
tests, and has **zero production callers** — the loop that actually bisects is
`search_for_culprit:170-194`, reimplemented because the oracle is async, and it has no tests at all.
`ui/preview.rs:174` advertises `[b] Cycle Backend`; both call sites pass `HashMap::new()` for
`alternatives`, and `get_filtered_changes` never reads `backend_overrides` anyway. Dead at both ends,
live in the UI text.

---

## Lens 2 — architecture

### The guard's number is assembled by the caller, and one caller passes zero

This is the most expensive finding in the document, because the removal set is the product.

`verbs/sync.rs:318-322` states the rule, correctly, in a comment:

> *"The package removals already planned are passed in, so `max_removals` is a ceiling on the command
> rather than on each phase — a sync dropping three packages and three links removes six things, and
> a limit of five has to see six."*

Four lines below it, `extras().reconcile(state, scope, packages_being_removed)` honours that.
**Fourteen lines above it, `Phase::Firewall => app.firewall().apply(state, scope_label(scope))`
passes a label and no count** — and inside, `apply/firewall.rs:146-152` calls `enforce_extras(…,
&removals, 0, …)` with the count hardcoded, under a comment congratulating itself on *"calling the
one two hundred lines away that already counts, caps, protects and reports."*

It does call the counter. It hands it a zero.

The guard is reached. Protection holds. `Reaped` is honestly minted. **It is the number that leaks**,
and `also_removing: usize` is a parameter that exists solely so callers can do the addition
themselves. The fix is a `Reaping` value that accumulates `(kind, backend, name)` across phases and is
enforced once at the end of the command — the way `SyncChanges` already accumulates the package half —
after which `also_removing` disappears.

**And the scope never arrives.** `guard_scope(scope: &str)` (`apply/firewall.rs:275`) matches
`"purge-undeclared"` / `"watch"` / `_`. Its only producer, `scope_label` (`verbs/sync.rs:41`), emits
`"an unattended watch tick"` or `"sync"`. **Neither named arm is reachable.** A firewall teardown on
an unattended `watch` tick — which the file's own header calls "the dangerous one, because nobody is
there to read a refusal" — is guarded and reported as `sync`. `GuardScope` is `Copy`. Pass it.

**And the register makes this worse, not academic.** I checked `N7` expecting to find the watch path
was report-only, which would have made the dead arm harmless. The opposite: **`N7` (ruled owner,
2026-07-24) is *revert by default* on an unattended tick**, reporting instead only when the revert
would close the session's own port. So the unreachable `"watch"` arm is not a branch waiting for a
feature — it is the guard scope for a live, ruled, unattended path that closes ports with nobody
watching, and it silently resolves to `Sync`. The ruling is what makes the string round-trip
load-bearing.

### The test written for that exact promise asserts an enum's `as_str()`

`tests/a_firewall_teardown_is_a_removal_tests.rs` opens with nineteen lines that diagnose the original
bug better than I could, ending: *"Three custom guards were written rather than calling the one two
hundred lines away that already counts, caps, protects and reports."*

It contains two tests. The first asserts `Reaped::for_reason(GuardScope::Sync, …).scope() ==
GuardScope::Sync` — a getter returns what a constructor took. The second asserts
`GuardScope::Sync.as_str() == "sync"` for three variants, under a comment that says:

> *"The mapping is private, so this asserts the property through the public enum instead."*

**The private mapping it declined to test is `guard_scope`, and it is broken in exactly the direction
the file's own header calls dangerous.** Neither test touches `apply/firewall.rs`, which has zero
`#[cfg(test)]`, and no test anywhere in `tests/` exercises port-closing behaviour at all.

### Three of the oracle tests are themselves checks that cannot fail

This suite has largely inoculated itself against its signature defect. Eleven binaries carry a genuine
oracle — `phase_is_the_sync_order_tests.rs:183-259` feeds its own scanner an or-pattern arm, a
commented-out arm, an empty-body arm and a neighbouring phase's arm, and is the model. Ratchets are
bidirectional: `why_entries_are_attached_to_something_tests.rs:184-202` asserts `uncited <= 52` **and
`>= 52`**, so unlowered slack is as red as growth.

Three files copied the oracle's *name* and *doc comment* and dropped its mechanism:

```rust
// tests/output_is_sanitized_tests.rs:171   "A gate that has never failed is a claim, not a check."
let raw_read = "let s = String::from_utf8_lossy(&out.stdout).trim().to_string();";
assert!(raw_read.contains("from_utf8_lossy(&") && raw_read.contains(".stdout"));
```

It never calls the scan. Replace the scan's predicate with `if false` and this stays green.
`ledger_file_rules_tests.rs:172-183` is the same shape. `grader_refusal_exit_code_tests.rs:399-424`
quotes the standard verbatim — *"do not test your own oracle by assuming it works"* — and then asserts
that a `const &str` declared two lines above contains a substring it visibly contains.

**The repo's disease reproducing inside its own immune system.** Three more binaries are vacuous
outright (`security_and_resiliency_tests.rs:399-423` has zero assertions;
`shell_lifecycle_tests.rs:99-123` asserts on `str::lines`; `integration_test.rs:101-125` asserts
nothing), and `dag_test.rs:140-155` claims to prove per-backend lock isolation by a mechanism that a
single global mutex would also satisfy — and nothing else in the suite covers lock granularity.

### A hand-maintained list of 21 strings decides who takes the write lock

`READ_ONLY_COMMANDS` (`main.rs:627`) sits seventy lines from `Commands` and decides whether a run takes
a 120-second exclusive lock on `data/`. Its own test docstring records that **twelve of its
thirty-three entries once named commands the program did not have.** Two tests guard it and both guard
*invention*; nothing guards *omission* or *misclassification*, which is the dangerous half.

`"history"` is on the list. `handle_history` runs the TUI; `HistoryAction::Rollback` calls
`handle_rollback`, which does `git.checkout_files` and then `handle_sync` — **the entire install/remove
path, `state.save()` and all, with no data lock held.** The identical function reached through
`Commands::Rollback` is locked. One function, two doors, two locking regimes, decided by a string
list. Meanwhile `fleet`, which touches no local state at all, is absent from the list and takes the
writer lock for a purely remote report.

### Four deciders for "is this already true", and `service:` uses two of them in opposite directions

`core/manager.rs:99-121` argues, well, that a new resource's obligation is to *"be reachable only
through one of those two deciders."* There are four: `spec_is_missing` (`planner.rs:865`), `in_effect`
(`apply/extras.rs:320`), a read-before-write inside `setting.rs:345` (necessary, because
`SettingQueryable::fetch_installed` returns `Ok(vec![])` so decider 1 answers "missing" for every
`setting:` line, always), and `apply/firewall.rs:90-110`'s own set difference — `firewall` appears
**zero times** in `registry.rs`.

The proof isn't rhetorical: **`service:` installs through decider 2 and removes through decider 1.**
One resource, two brains, each taught separately what "already converged" means. `extras.rs:333-334`
records what that cost — adopting 150 running services made every later sync run 150 `sc start` calls,
because only one decider had been taught.

### Two backends silently lost a capability, and a test asserts the loss

winget declares `upgrade_args: ["upgrade","--all","--silent"]` and `update_args` (`registry.rs:945`).
scoop declares both (`registry.rs:1030`). Both are registered through a hand-written builder chain
with `installable`, `queryable`, `searchable`, `repo_manager`, `metadata_provider` — and **no
`.with_upgradable(…)`** (`registry.rs:997`, `:1077`). `context.rs:536` filters on `is_upgradable()`,
so `linix update` and `linix upgrade` skip both on every Windows machine.

`registry.rs:3038-3062` asserts exactly that capability set as correct.

The data already knows the answer — `onboarder.rs:848-875` derives the wiring from the same fields for
custom rows. The built-in half types it by hand 22 times, which is why the compiler never looked.

### The rollback engine and the WAL cover disjoint failure modes, and the two arms disagree

`Transaction::rollback`'s install arm was fixed to consult the manifest: a package that installed
cleanly and is still declared is left alone, because removing it *"is the one mechanism in the program
that provably un-converges"* (`transaction.rs:166-176`). The removal arm (`:1134-1151`) never reads
`self.declared` and structurally cannot — a removal's target is by definition absent from the declared
set.

So a failed sync leaves the installs applied and the removals reverted: **neither the pre-sync state
nor the declared state, and no state the machine was ever in.** Concretely — delete `apt:nano`, sync,
`ripgrep` fails on a typo, rollback reinstalls `nano`, you fix the typo, sync again, `nano` is removed
a second time. Two removals from one edit.

I put the obvious defence to the reviewer — *undoing a partially applied failed sync is the whole point
of a write-ahead log* — and it does not survive, for a reason worth stating plainly: **`Prior` is not
in the WAL.** It lives in `Transaction::history` (`:149`), in memory, and dies with the process.
`journal.rs` records *that* a mutation started, never what preceded it. Compensating rollback therefore
works only when the process is still alive — which is exactly the case where re-running `sync` also
works. For the case the WAL exists for, `heal` does not roll back at all; it replays forward.

**Then I checked the register, and the finding changed shape — for the better.** `U41` (ruled owner,
2026-07-27) settles this: *"An upgrade is compensated by the old version, not by an uninstall… A
rolled-back removal comes back pinned."* Both arms compensate, by ruling. So reinstating `nano` is
**not a defect** — it is the owner's answer, and I was about to report a closed decision as a bug.

What is a defect is what happened afterwards. `LX-3`'s fix (commit `e9a6ac4`, 2026-08-07) added
`reconciling()` and changed the **install** arm to consult the manifest — a deliberate, correct,
well-argued change that makes rollback stop un-converging the machine. It also **narrows `U41`'s
ruling on one arm and leaves it standing on the other**, and the register still records `U41` as
`ANSWERED` with no amendment. `CLAUDE.md` requires a ruling to ship in the commit that answers it;
the mirror obligation — a commit that *changes behaviour a ruling covers reopens it* — was not met.

So the finding is not "rollback is wrong." It is: **`U41` now has two answers in one function, and
the register knows about neither.** That is a smaller finding and a more actionable one.

---

## Lens 3 — implementation

Lens 3 is the cheapest to find and the least valuable to get right, so this section is short on
purpose. Two of these are large; the rest are named and priced honestly.

**"Is it installed?" is answered twice, and the expensive answer wins.** `installed_sets`
(`planner.rs:571-598`) builds `HashMap<backend, HashSet<name>>` once per backend and is used only by
the `absent:` loop. `identify_needed_actions` then throws it away and calls `q.info(&spec.name)` per
declared spec (`:873`) — and for the 18 backends implementing `info` as list-then-find, each call
clones the whole `Vec<Package>` out of the memo (`core/installed.rs:193`) and does a linear `.find()`.
On a 256-line winget config against a 280-package listing that is **~71,680 `Package` clones and 256
mutex acquisitions** for 256 answers that were in a hash set built forty lines earlier. The fix is to
have `installed_sets` return `HashMap<backend, HashMap<name, Package>>` — it already holds the
`Vec<Package>` at `:582` and discards everything but the name, and `spec_is_missing` needs exactly the
`.version` and `.properties` it discards.

**Seven `PackageSpec` clones and a whole-graph clone per sync.** `partition_by_presence`
(`planner.rs:124`) clones every spec to split on a field the spec already carries.
`apply_scope_filtering` (`:806-808`) does `return desired.clone()` when there is no scope — **the
whole-machine sync, the common case, deep-clones the entire desired map for nothing.**
`declared_specs` (`:995`) clones a third time into a differently-keyed map. At 298 declarations that is
~1,500 map allocations before the first `apt install`.

**Cheap, real, one line each:**
- `resolve.rs:821` — `reached.statements.iter().cloned()` deep-clones the entire configuration in
  order to iterate an owned value. `let Reached { statements, scopes } = reached;`.
- `go.rs:246-260` — one `go version -m` subprocess **per file** in the Go bin dir, inside the walk.
  `go version -m` takes a list.
- `psresource.rs:214-243` — one PowerShell process per package. Every other backend in that region was
  batched under Q45; this is the one the sweep missed, and PowerShell has the most expensive startup
  of any of them.
- `guard::essential_names` — a subprocess per backend (on apt, `dpkg-query -W` over the whole
  database), run **twice** per removal command, unmemoised, under a comment calling it *"cheap"*.
- `ci.yml:107` — the cargo cache key is `${{ runner.os }}-cargo-${{ hashFiles(...) }}`. `matrix.target`
  is used on the very next line and is **not in the key**, so `macos-13` (x86_64) and `macos-latest`
  (aarch64) collide on one entry. `actions/cache` does not save on an exact-key hit, so one Apple
  target rebuilds cold on every run, permanently.
- `Cargo.toml:133` — `lto = true, codegen-units = 1` applies to `cargo test --release`, so **99 test
  binaries each get their own fat-LTO link**, 34 of which never call the library API at all. A
  `[profile.ci]` inheriting release with `lto = false` changes nothing about the shipped artifact.

**Three sites download an unbounded response body into RAM** — `github.rs:637`, `web.rs:174`,
`appimage.rs:168` all call `response.bytes().await` with no `content-length` check and no cap, then
immediately write it to a temp file. Streaming is the same code shape. `core/download.rs` has scheme
policy and checksum policy and no size policy.

---

## Findings, ranked by wrongness × cost of leaving

The register owns the ID letters; these are placeholders.

**1 · The guard's ceiling is assembled by three callers and one passes `0`** — `rewrite`
*(verified)*. `apply/firewall.rs:151`. 4 packages + 4 ports under a limit of 5 is invisible to every
guard call in the run. Change: a `Reaping` accumulator enforced once per command; `also_removing`
disappears. Cost: three call sites, one new type.

**2 · `GuardScope` round-trips through a string whose two vocabularies don't overlap** — `rewrite`
*(verified)*. `verbs/sync.rs:41` produces `"an unattended watch tick"`; `apply/firewall.rs:275`
matches `"watch"`. Both named arms unreachable; the unattended tick is guarded as `sync`. Change:
pass the `Copy` enum. Cost: two signatures. **Do this one first — it is ten minutes.**

**3 · `linix history` can run a full rollback + sync with no data lock held** — `rewrite`
*(verified)*. `main.rs:638` exempts it; `verbs/history.rs:317` reaches `handle_rollback` →
`handle_sync`. Change: `fn Commands::writes(&self) -> bool`, an exhaustive match, as the *assertion*
the two tests currently approximate — keep the argv read for the default so a new subcommand is still
locked by default. Also removes `fleet`'s pointless writer lock. Cost: one match, two tests rewritten.

**4 · Three oracle tests cannot fail, and six more tests assert nothing** — `rewrite` *(verified)*.
`output_is_sanitized_tests.rs:171`, `ledger_file_rules_tests.rs:172`,
`grader_refusal_exit_code_tests.rs:399`. Change: each drives its own scanner over a planted offender
and a planted innocent, the way eleven siblings already do. Cost: an hour. **Do this before anything
in this document is fixed**, because three of the gates you would rely on to check the fixes are
currently claims.

**5 · The teardown dispatches on a `&str` and shrugs at what it doesn't know** — `rewrite`.
`extras.rs:243`'s `other => warn!; Ok(())` **reports a teardown as done to a caller that then clears
the ledger row**; `extras.rs:330`'s `_ => None` reads as *unverifiable*, which re-places on every sync
forever. Both failures have already happened and the comments record them. Change: those dispatches
take the `Statement` (or a `ResourceKind`), not a string — `in_effect` already *receives* the
`Statement` at `:323` and throws it away to match on a string parsed out of the key. Cost: two
signatures; the compiler finds the rest.

**6 · `or_unrecognised`'s JSON arm disables the LX-1 fix for five backends** — `rewrite` *(verified)*.
`parsers/mod.rs:124-132` returns `Ok(found)` — empty — whenever the output contains a parseable JSON
document, regardless of whether the reader extracted anything. So npm renaming `dependencies`, or pip
capitalising `name`, is a silently empty machine: install everything, own nothing. **Six sites in the
repo already hand-roll the correct rule** (`if found.is_empty() && !arr.is_empty()`) — the shared
helper has the weak one, so the five backends that use the helper are the unprotected ones. Change:
`or_unrecognised_json(backend, found, container_len)`, and delete the six literals. This also deletes
the `candidates[start..].join("\n")` full-output copy that runs on every pipx and yarn listing.

**7 · `composer`'s installed reader doesn't strip the banner its sibling strips** — `rewrite`
*(verified)*. `registry.rs:1931` wires `composer global show --format=json` to
`parse_composer_json` (`language.rs:271`), which does `serde_json::from_str(output)` on the whole
output. composer prints `Changed current directory to …` first. **The comment on the line below
(`registry.rs:1933`) explains that banner and is attached to the `outdated` probe**, whose parser does
`text.find('{')` at `language.rs:634`. Two lines apart, one fixed. Combined with finding 6, the
installed listing — the one that feeds `sync` — silently returns empty on every machine with a global
composer config, which is all of them.

**8 · `appimage:` re-downloads every declared AppImage on every sync, forever** — `rewrite`
*(verified)*. `appimage.rs:284-293` keys `fetch_installed` by `url.split('/').next_back()` while
install keys state by the full URL (`:216`), so `info(url)` never matches. `web.rs:415` returns the URL
and is correct. **`btrfs.rs:795-820` carries a test for this exact family**, dated 2026-07-30, whose
comment reads *"A name `list` does not return is a package `sync` believes is absent: it re-creates it
on every run, for ever."* Family diagnosed, named, tested, fixed in one member. One line:
`Package::new(url, "appimage")`.

**9 · `vscode:` asks the marketplace whether a package is installed** — `rewrite` *(verified)*.
`vscode.rs:192-218` — `info` POSTs to `marketplace.visualstudio.com` and returns `Some` for anything
that *exists*, with the marketplace's *latest* version; `fetch_installed` (`:170`) reads
`code --list-extensions` and is never consulted. So `linix install vscode:x` reports success and
installs nothing, a `@version=` pin reinstalls forever once upstream moves, and every plan makes one
rate-limited HTTPS POST per extension. **`mise.rs:183-192` carries this bug's obituary in a doc
comment**, found by the `tools` container on 2026-07-24, with an assertion at `:409` that the catalogue
is never consulted. The test that would catch vscode already exists, written against the wrong backend.

**10 · winget and scoop lost `Upgradable`, and a test pins it** — `rewrite` *(verified)*.
`registry.rs:997`, `:1077`, pinned at `:3038-3062`. Change: derive the capability set from the config
(the onboarder already does), or at minimum add the two builder calls and fix the matrix.

**11 · `Statement::key()` is a second wire format with nine hand-rolled readers, and it is on disk** —
`rewrite`. `statement.rs:247`'s own doc forbids re-splitting it; `extras_lock.rs:76`, `guard.rs:705`,
`apply/extras.rs:274`, `verbs/plan.rs:664`, `verbs/cleanup.rs:649`, `core/state.rs:106`,
`model/resolve.rs:1063`, `verbs/packages.rs:911` do. Change: `struct ExtraKey { kind, subject }` with
`Display` + `FromStr`. Same one producer, plus one parser.

**12 · Four atomic writers under a comment asserting there is one** — `rewrite`. `utils/file.rs:19`
says *"There were two of these… there is now one."* There are four: `atomic_write` (fsyncs),
`executor.rs:1625` `write_atomic` (**no fsync** — used by the scheduler and `link:`),
`executor.rs:1661` `write_secret` (**no fsync**), and `installed.rs:147` (correctly excluded — it's a
cache, and a torn file is a miss). Consequence: power loss after a sync leaves a zero-length systemd
unit while `registry.json` and the WAL, which went through `persist`, survive. `verbs/plan.rs:669`
cites the false singleton as proof of a different claim.

**13 · `LX-3` amended a closed ruling on one arm and the register was not told** — `rewrite`
*(verified against the register)*. `U41` (2026-07-27) ruled that both rollback arms compensate;
`reconciling()` (commit `e9a6ac4`) changed the install arm to consult the manifest instead, and
`decisions.md` still records `U41` as `ANSWERED` unamended. Change: re-open `U41` with one sentence
naming what `LX-3` changed, and rule which arm is now canonical. The code change may well be right —
that is not what this finding is about.

**14 · The smaller families** — each verified, each a one-liner:
- `install.sh:9` documents `LINIX_BIN_DIR`; **the variable is read nowhere in the repo.** This is the
  file users pipe from the internet.
- `release-check.sh:71` — `grep -q "^$MSRV"` with `$MSRV` empty is `grep -q "^"`, which matches every
  line; the `.ps1` twin got an explicit guard, the `.sh` did not.
- `Cargo.lock` holds **448** crates. Four files say 380, two say 452 — in a repo that wrote a 226-line
  script because two files tracked one number by hand.
- Two files carry `# shellcheck disable=` directives for a linter that **nothing runs**.
- `md5` is still a direct dependency for one cache key (`web.rs:237`) beside `sha2`.
- `model/edit.rs:321,470` rejoins with `\n` after `str::lines()`, so every `linix install` rewrites a
  CRLF module file to LF in full — in a grammar that goes out of its way to accept a BOM *because
  that is what Notepad writes*, and Notepad writes CRLF. Two halves of one courtesy, one delivered.
- `Fixture` is written **34 times** across the test suite and has already drifted three ways;
  `HOME`/`USERPROFILE` is set in 2 of 34, and the file that sets it records being **red on
  ubuntu-latest and macos-latest from the day it was committed**.
- Three test binaries justify a source scan with *"`verbs/` is private to the binary"*. `src/lib.rs:20`
  is `pub mod verbs;`, and `verbs_are_reachable_tests.rs:16` imports through it.
- `insight.rs:552-584` — `linix why` constructs a fresh `StateResolver` **inside** its match loop and
  calls `resolve_model()` and `resolve_vars_with_origins()`, so a name managed by two backends resolves
  the entire configuration twice.
- Two `Writes` enums, same name, same constructor, same semantics (`model/edit.rs:134`,
  `bundle.rs:75`) — and the first documents itself as *"the one place `--dry-run` becomes an editing
  mode, so no caller can decide it twice."* Dry-run is decided five ways across the satellites.
- `setting.rs:361-378` — removing a `setting:x@scope=system` line resets the **user** key.

---

## Compaction — how to lose 8,000 lines and 98 link units without losing a single capability

**The thesis, first, because it decides whether any of this is worth doing: compaction here is not a
diet. It is the same act as the fix.** Every finding above is a second copy of something, and the
second copy is always the one without the reasoning — `appimage.rs`'s missing mutex, `composer`'s
missing banner-skip, `vscode`'s missing `list_installed`, `guard_scope`'s missing enum. So the
question "how do I make this smaller" and the question "how do I stop shipping the same bug twice"
have one answer, and the line count is just the receipt.

Everything below preserves every verb, every backend, every statement kind, every reachable
behaviour. Counts are measured, not estimated; where I am guessing, it says so.

### A · The registry becomes the table it already is — **~2,400 lines**

`registry.rs` non-test body is 2,765 lines: **46 `register_*` functions, 379 `: None,` lines**, 24
hand-passed boolean triples that restate data three lines above them, and 22 registrars that spell
out all 36 fields because `base_config` was written and the older half never converted.

Ship `src/backends/builtin_backends.toml`, `include_str!`'d through `register_custom_backends`.
**This is not a proposal, it is the pattern four smaller tables in this same directory already
use** — and `setting_stores.toml:1` states the reason in its own header: *"an adapter mechanism the
built-ins bypass is one nobody has tested."* The one table with 62 rows is the one that bypasses it.

Capability cost: **zero**, and it is *capability-positive* — the winget/scoop `Upgradable` loss
(finding 10) becomes unrepresentable, because the capability set is derived from the row rather than
typed by hand beside it. `onboarder.rs:848-875` already does the derivation correctly for custom rows.

Residue that stays Rust: ~200 lines of genuinely irreducible wiring plus the five shapes a row still
cannot express (install conditional on a read; argv that is a program, not a template; removal that is
a filesystem op informed by a query; a `PropertyProbe` reaching *into* JSON; per-package identity that
includes the version).

### B · The parser half of the data path — **~700 lines**

`ParserSpec` (`onboarder.rs:70-105`) ships, is documented to third parties, is correctly fallible, and
has **one reference in the tree, which is a comment**. Of ~40 installed/search readers, ~30 are
expressible as a row today; two more arms (`FixedWidth{header_columns}`, `Tree{depth}`) take it to
~35. The ten survivors are the interesting ones and stay.

Add one column the current design lacks and this whole class closes: **`fixture`, required.** A parser
that cannot be registered without bytes from its own tool is the only mechanism that would have
prevented `ws_name_version` serving eight managers on helm's fixture alone.

Free riders: `parsers/utils.rs` (50 lines, zero callers, containing a proud performance comment about
a regex hoisted in a function nobody calls) and `parsers/pkgsrc.rs` (74 lines that now delegate to
`bsd.rs` — the deletion landed halfway).

### C · Three artifact downloaders become one — **~600 lines**

`appimage.rs` is 471 lines whose own test header reads *"its removal is `web.rs`'s removal with the D5
handoff taken out — same state file, same two deployed paths, same re-insert-on-failure rule."* The
`remove()` routine is the same ~60 lines in `github.rs`, `web.rs` and `appimage.rs`. `pip_search.rs`
(86) and `node_registry.rs` (105) are two HTTP search helpers exempted by name from the data ratchet,
predating the `SearchSource` variant that was added for exactly them.

Capability cost: **zero.** `appimage:`, `web:` and the two searches keep working; finding 8's
re-download-forever bug dies with the duplicate, because there is then one identity rule.

### D · The test suite: 99 link units → 1, and ~1,550 lines of copied harness — **~2,400 lines**

`tests/` is 24,076 lines across **99 binaries**, each fat-LTO-linked against a 100k-line crate under
`codegen-units = 1`. **36 of them never call the library API at all** — they only spawn
`CARGO_BIN_EXE_linix` — and link it for nothing.

- **`tests/main.rs` with `mod a_machine_converges;`** keeps every file, every filename-as-a-sentence,
  every doc comment, and collapses 99 link units to one. Zero source lines deleted. The suite already
  paid this cost once and wrote it down: `mock_providers/mod.rs:1-9` records a top-level
  `mock_providers.rs` becoming *"a 716 KB binary containing zero tests… compiled nineteen times."*
  The fix moved one file; the structural version moves ninety-nine.
  *(I did not measure the wall-clock win — no builds were run. The structural claim is that three full
  compilations of a 448-crate graph happen per CI leg and two of them do not need LTO.)*
- **`tests/harness/mod.rs`** — the `Fixture` block is written **36 times**, ~25 lines each, and has
  already drifted three ways. `HOME`/`USERPROFILE` is set in 2 of 36, and the file that sets it records
  being **red on `ubuntu-latest` and `macos-latest` from the day it was committed**, green only where
  the checkout happened to sit under `$HOME`. **~850 lines**, and it is the cheapest win in the repo.
- **`tests/ledger/mod.rs`** — **13 files** carry a const exemption table and each re-implements the
  same four assertions (every found site is in the table; every entry still names a real site; the
  reason is longer than N chars; a floor so an empty scan cannot read as clean). One
  `Ledger::audit(found, entries)` is **~700 lines**, and it would have prevented finding 4 outright,
  because the helper would own the drive-your-own-predicate-over-planted-inputs step instead of leaving
  it to be copy-pasted and dropped.
- **Five subsumed binaries** — `dag_test` (155), `integration_test` (178), `e2e_tests` (216),
  `shell_lifecycle_tests` (180), `help_map_tests` (162) = **891 lines**. Not blind deletion: fold
  `dag_test:16-87`'s ordering check and `help_map`'s *every command appears in the map* into their
  stronger siblings first, then delete. `e2e_tests` already concedes the point in
  `a_machine_converges_tests.rs:4-6`.

### E · Cross-cutting: one `confirm()`, one `Output` — **~300 lines**

**12 sites** re-type the same three steps (`is_terminal` check → refusal naming `--yes` → `dialoguer`).
`cleanup.rs:148-162` already *is* that function; it was extracted once and never reused. **25 files**
read `config.dry_run`, with 17 hand-written `if dry_run { …preview…; return }` branches in `verbs/`
alone, each writing its own "would" sentence — which is why a 614-line test binary has to exist to
check they all remembered. `--json` is a per-subcommand `bool` on ten variants with ~15 hand-built
`serde_json::json!` literals, and a 188-line test binary standing in for the missing
`enum Output { Human, Json }`.

Compacting these shrinks `dry_run_every_verb_tests.rs` and retires
`json_output_is_a_document_tests.rs` — a test whose whole job is to be the type that isn't there.

### F · Small, verified, no argument required — **~700 lines**

`retention.rs` (244 lines) opens *"shared by the three histories LiNix keeps"*; two of the three were
deleted, `select_deletions` has one caller, and `RetentionConfig` now wraps a single field. The
`gated.rs` block walker (~120 of 208 lines) is the second of five implementations of brace-and-`when`
handling. `utils/file.rs`'s three dead functions, `Journal::new` (zero callers — the *cause* of the
733 KB test-pollution bug is still exported), `Journal::removals_of` (written for a call site that
hand-rolls it instead), the second `Writes` enum, `md5`.

### G · The review apparatus stops being shell — **~300 lines**

Roughly 300 of `harness-logic-test.sh`'s 761 lines enforce invariants over YAML, Markdown and Rust by
grepping them — gate parity, orphan scripts, function-defined-before-called, CRLF, floor mounts, image
identity. Those belong beside the 27 Rust gates that already do exactly this, where they fail in
`cargo test` instead of in CI. *Function defined before called* is ShellCheck `SC2218`, and **two files
already carry `# shellcheck disable=` directives for a linter that nothing runs.**

The half that lifts shell functions out of the harness and drives them (`:33-405`) is correct shell for
a correct reason and stays — it is the only technique that tests what CI actually runs.

### The total, honestly

| | lines | capability lost |
|---|---|---|
| A registry as data | ~2,400 | none *(negative — closes finding 10 by construction)* |
| B parser rows | ~700 | none |
| C one downloader | ~600 | none *(closes finding 8)* |
| D test harness + ledger + folds | ~2,400 | none *(98 fewer link units; closes finding 4)* |
| E `confirm()` / `Output` | ~300 | none |
| F verified dead / second copies | ~700 | none |
| G shell → Rust | ~300 | none |
| **total** | **~7,400** | **none** |

Against `src/` at 99,794 and `tests/` at 24,076. **Roughly 6% of the tree, and it is the 6% that
contains most of this document's findings** — which is the actual argument for doing it. Nothing here
is "make it smaller because smaller is nicer"; every row is a place where one fact is written twice
and the second copy has already been observed to be the stale one.

**Sequencing matters more than the total.** Do D's ledger helper and the finding-4 oracle fixes first,
because the gates you would use to verify A through C are, today, partly claims. Then A, because it is
the largest and it makes B, C and finding 10 either trivial or impossible-to-reintroduce.

**One thing I will not recommend compacting.** `docs/` is 19,020 lines, of which `decisions.md` is
7,269 — and having now used the register to correct two of my own findings inside ten minutes
(`N7`, `U41`), I withdraw any instinct to cut it. It is the only artifact in this repository that
made a reviewer wrong on the record. `LX-6` already cut what was prose; what is left is load-bearing.

---

## What I could not beat

Six things I tried to design better and failed at, and two premises of my own that died on contact.

**`Reaped` is the answer to this document's strongest claim, applied to one property.** A token with a
private field, demanded by every effector's signature, so "did this removal pass the guard" is a
compile error rather than a convention. `guard.rs:25-44` records why the previous approach failed with
a receipt: the source scan matched `.remove(`, `.remove_repo(`, `.remove_shim(`, `.deprovision(` — and
`apply/firewall.rs` closed a port with `deny_command`, matching none of them. *"The fix for G-1
replaced a stale list of paths with a stale list of verbs."* Every `Reaped::for_reason` outside the one
recovery site carries the reason string *"a unit test of the effector itself."* The escape hatch demands
you write down why, and every use of it says "test". **This is the technique the other seven boundary
failures need, and it already exists in the tree.**

**`Phase`.** The order lives in `next()` and nowhere else; `all()` walks the chain rather than reading
a second list; `verbs/sync.rs:291` matches exhaustively, so a new phase cannot compile until sync says
what to do with it. `verbs/sync.rs:268-273` records four separate misses of the hand-written call
sequence it replaced — *"Four times is not four mistakes, it is one list nothing checked."*

**`tests/backend_is_data_not_code_tests.rs`.** A shrink-only ratchet requiring every hand-written
backend to carry a reason over 60 characters *and* a proof string that must literally appear in the
module — with `the_proof_check_can_actually_fail` planting a falsehood to confirm the checker rejects
it. It took 29 modules to 18 and caught three exemptions describing code that wasn't in the file they
excused. It is also the instrument that makes finding 4 embarrassing: this is how an oracle is written,
and it is in the same directory as three that aren't.

**The planner/resolver boundary, and I came in believing it was wrong.** Co-change said 17 shared
commits across two 1,800-line files with adjacent names — the textbook signal. The reviewer killed it
with evidence I should have gathered myself: all but one of those 17 touched ≥25 files (rustfmt at 133,
the `Options` type change at 94), and **the single tight co-change is the commit that created the
boundary.** The seam is real — `resolver.rs` asks `Searchable` (does this manager *carry* this name),
`planner.rs` asks `Queryable` (is it *installed*), and neither reads the other's world. The loose seam
is `planner.rs` ↔ `sync/mod.rs`, nine tight co-changes, and it is `SyncChanges` — the plan *value* —
living inside the file that *computes* plans.

**`installed.rs::cache_may_answer`** — an allowlist of subcommands a cross-run cached listing may
serve. It reads like a smell (the cache should know its own freshness) and it isn't. Three replacements
were tried and each loses; the rule it encodes has no cheaper expression: *a cached listing may inform
a report; it may never source a decision that outlives the run.* That is a property of what the caller
**does with the answer**, and nothing the cache can observe about itself will ever tell it.

**The container matrix, again, and I sent someone to kill it again.** Seven real package managers on
every push, nine images, `--privileged` loopback btrfs/lvm/zfs. `Dockerfile.gentoo`'s `SMOKE_ONLY=1` is
honest and correctly labelled — it buys argv construction and the planner against a real Portage
system and says so.

**Two premises of mine that died:**

1. *"The repo violates its own flagship rule — 20 sites split on `:` outside the grammar."* **False.**
   I read every one. They split *internally constructed keys* or parse a manager's `Key: Value` output,
   and the two most tempting sites carry comments explaining why they are not the forbidden thing
   (`context.rs:684`, `resolver.rs:1197`). `verbs/cleanup.rs:641` routes the user-typed spec through
   `config::parser::split_removal_target`, the one parser. The rule holds for user-written lines. What
   is true is narrower and is finding 11: the *extras ledger* is a second key namespace with three
   readers, and `key()`'s own doc comment is what forbids it.
2. *"`main.rs`'s 230-line pre-clap argv layer is a shadow copy of the CLI's vocabulary."* **Mostly
   false.** `ignore_errors(true)` still cannot hand you a `Commands` when the subcommand token is not a
   subcommand, so alias expansion genuinely must precede clap. And `global_value_flags` (`:698`) asks
   clap what takes a value rather than keeping a list, *because* the hand-written list it replaced named
   `-b`/`-g` after both were deleted. The layer survives. `READ_ONLY_COMMANDS`, which sits in it, does
   not — and that is finding 3, which is the sharper version of what I was reaching for.

---

## What I got wrong this run

Stated because a review that only reports its survivors is not reporting its method.

- **I claimed the `@expires` datetime validator and reader had drifted, and that a bare `YYYY-MM-DD`
  would validate and then never expire.** Wrong. `parse_absolute` handles `%Y-%m-%d` in a separate
  branch at `dated.rs:63`, outside the array I read, with a test at `:163`. The two accept the same set.
  I caught this myself one message after asserting it, which is one message too late.
- **A reviewer reported `snapshot_restore.rs:351` as an unguarded path that "reverts every managed
  package with no guard."** Wrong. `execute_restore` calls `snapshot_manager.restore(&id)` — a
  whole-filesystem rollback — and takes no package-removal path at all. No removal, no guard needed.
- **I was one edit away from reporting a closed owner ruling as a defect.** The original draft of
  finding 13 called `Transaction::rollback`'s removal arm anti-convergent and asked you to rule on it.
  `U41` (2026-07-27) had already ruled it — *"a rolled-back removal comes back pinned"* — and I only
  found that because you told me to check `docs/`. The finding survived in a smaller and better form
  (the register was not amended when `LX-3` changed the other arm), but the version I would have
  shipped without reading the register was wrong, and it would have been wrong in the most expensive
  way available: asking the owner to re-decide something he had already decided.
- **The same check made finding 2 stronger, which is the other half of the lesson.** I looked up `N7`
  hoping the unattended watch path was report-only, which would have made the dead `"watch"` arm
  harmless. It is *revert by default*. The register does not only kill findings.
- **A reviewer reported `harness-logic-test.sh:507-513` as conceding it is superseded by a Rust test
  that was kept anyway — a NO LEGACY violation.** Misread. The comment declares a deliberate division
  of labour: the shell predicate answers for *gate scripts*, job-level parity is
  `grade6_gate_parity_sees_whole_jobs_tests.rs`, and it says so **in the `ok` message itself**. That is
  good design. Whether the *third* implementation (`grader_gate_parity_tests.rs`) is redundant with the
  second is a separate question I did not settle.

---

## What I need from you

1. **`U41` needs re-opening, not re-deciding.** (Finding 13.) You ruled on 2026-07-27 that both
   rollback arms compensate. `LX-3` changed the install arm four days ago and the register was not
   amended. The question is one sentence: **does `reconciling()` extend to the removal arm, or is the
   asymmetry deliberate?** My recommendation is that it cannot — a removal's target is by definition
   absent from the declared set, so there is nothing for that arm to consult — which means the honest
   options are *keep the asymmetry and write down why*, or *drop `auto_rollback` on the reconciling
   path entirely* (convergent: a partial sync is closer to converged than the state before it, and it
   is the only answer a killed process can also honour, which is the failure mode the WAL exists for).
   The second deletes ~180 lines including the whole `Prior` capture. Either way `U41` gets a line.

2. **Is closing an undeclared port a removal, and does it count against `max_removals`?** I searched
   the register: `N1`–`N7` settle exclusivity, restoration, SSH lockout, dual ownership and the
   unattended tick. **None of them asks whether a closed port is countable.** It is not an unanswered
   decision — it is an unasked one, and it is the only thing blocking finding 1. A `Reaping`
   accumulator makes ten open ports and one `firewall:` line hit the default ceiling of 20
   immediately. The hardcoded `0` at `apply/firewall.rs:151` is currently answering it by accident,
   and `N7`'s *revert by default on an unattended tick* is what makes the accident load-bearing.

3. **What is the next feature?** Half of what makes a design wrong is the change it's about to face, and
   that is not in the repo. If the answer is more statement kinds, finding 5 is where the next bug is:
   ten files, and only one of them fails to compile.

4. **Still unanswered from 2026-08-07, and it still reorders everything: is anyone meant to use this
   yet?** `git tag -l` is still empty, `ci.yml:160` has still never fired, and `install.sh:37` still
   falls back to compiling 448 crates under fat LTO on a stranger's laptop while the readme calls it a
   "30-second first run". Forty backends stay unproven no matter how many review rounds land.

**One datum for whichever way you answer.** I measured how much of this tree has settled: **zero of
194 source files have gone three weeks without an edit.** In the eight days to today, 153 of 194 were
touched. Nothing here has calcified, which means every cost line in this document is at its floor and
rising. That is an argument for doing the cheap structural ones now, and against treating any of them
as too entrenched to move.

---

## Suggested order

Nothing here removes a capability. Items 1–4 are under a day, together.

1. **Fix the three oracle tests** (finding 4). An hour, and it must come first: three of the gates you
   would use to check everything below are currently claims rather than checks.
2. **Pass `GuardScope` instead of a string** (finding 2). Ten minutes. Two signatures, and an unattended
   `watch` tick starts refusing in its own name.
3. **`Commands::writes()` as an exhaustive match** (finding 3), keeping the argv read as the default so
   a new subcommand is still locked by default. Closes an unlocked full-sync path.
4. **The three verified backend defects** (findings 7, 8, 9) — one line each, all three with the fix
   already written in a sibling file, all three currently telling `sync` something false about the
   machine.
5. **`or_unrecognised_json`** (finding 6), and delete the six hand-rolled copies. This is the second
   half of `LX-1`; the first half shipped and this is where it stops.
6. **Ask the `max_removals` question** (need 2), then land the `Reaping` accumulator (finding 1). This
   is the one that makes `readme.md:358`'s sentence true instead of nearly true.
7. **The dispatch types** (findings 5 and 11) — `ResourceKind` instead of `&str`, `ExtraKey` instead of
   nine `split_once(':')`. After these, an 11th resource kind costs a compile error instead of a
   `warn!`.
8. **Amend `U41`** (finding 13) — one sentence in the register naming what `LX-3` changed. Free, and
   it is the difference between a ruling and a ruling nobody can trust.
9. **Then compaction D and A, in that order** — the shared test harness and ledger helper first
   (~2,400 lines, and it is what makes every gate below it trustworthy), then the registry as a TOML
   table (~2,400 lines, and it closes finding 10 by construction). These two are half the compaction
   total and they are the two that make the rest cheap.
10. **Then the structural ones with no deadline and a rising price:** compaction B (the parser data
    path — `ParserSpec` has one reference in the tree and it is a comment), compaction C, and the
    plan/differ split in `planner.rs`, which is where the co-change signal actually points.
11. **And the free ones whenever:** `[profile.ci]` without fat LTO, `matrix.target` in the cache key,
    ShellCheck in CI for the 7,133 lines of shell that already carry its suppression comments, and
    `LINIX_BIN_DIR` either implemented or deleted from the file users pipe from the internet.
