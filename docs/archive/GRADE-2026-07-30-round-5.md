# LiNix — independent grade, round 5, 2026-07-30

> Graded at `0cdeca2` (`grade/2026-07-29`, clean tree, nothing unpushed) · Windows binary built
> from that tree, plus an `ubuntu` integration image rebuilt at the same commit and a full CI run
> dispatched at it ([30553790717](https://github.com/SYKhayyat/LiNix/actions/runs/30553790717)).
> Method: each round-4 finding re-run by **its original reproduction**, on Windows and again by
> hand inside a Linux container where the committed test cannot run; then a from-the-sink
> enumeration of every path that reaches a backend's `remove`/`purge`; a hostile-manifest sweep of
> the grammar; a `SIGKILL` mid-transaction followed by `heal`; and a byte-level look at what a
> refusal prints. **`docs/BUILDER.md` was not read at any point.** Every disposition below is a
> measurement; where a claim could not be measured it says so and says why.

---

## 0. Verdict

**Overall: B−, unchanged — and unchanged for entirely different reasons than last round.**

Everything round 4 raised is closed, and closed at the mechanism rather than at the symptom. The
blocker is gone along with its whole family. What holds the grade is that hunting past the list
turned up a defect of the same weight as the one that was fixed: **thirteen of the fourteen
grammar keywords, written on a line by themselves, are silently reinterpreted as package names,
resolved against real package indexes, and queued for install — and `linix check` tells you to run
the sync that installs them.** A user who types `link` and forgets the rest of the line is one
`linix sync` from having the `link` crate from crates.io on their machine.

**What the round got right, verified rather than accepted.** B-1 is closed at the writer, not at
the call site: `--dry-run adopt` now writes nothing, and says so twice — `[DRY-RUN] would write
…/registry.json` and `[DRY-RUN] 111 managed package(s) and 0 hold(s) were not recorded` — with the
past tense gone from the summary. `hold` and `unhold`, the two siblings the round-4 report named,
behave the same way, each with a control proving the case is meaningful. P-1's false refusal
warning is gone and `plan` now predicts exactly what `apply` performs, including the count
objection at rc=3 in the right noun. P-2's re-placement loop is gone: three consecutive syncs
leave three files and no `.linix-backup`. P-3 answers a bare name from the config rules, names the
rule, and states what it did *not* consult. P-4's `exposes` row is gone. The gate that could not
see B-1 now walks config, data **and** the working directory, and drives the three verbs it used
to excuse — the exemption list was rewritten so that a reason describes what the fixture cannot
supply rather than what the instrument cannot see. The builder edited four of my five red test
files; all four diffs are rustfmt plus one logically identical De Morgan rewrite. Nothing was
weakened.

**The safety core came through an adversarial pass intact.** Enumerated from the sink — every
`.remove(`/`.purge(` on a backend capability, not from a list of commands — all nine removal paths
have a guard upstream, including the two that matter most because nobody is watching: the
ephemeral-shell exit teardown (`app/shell/mod.rs:295` → `GuardScope::ShellExit`) and the lease
expiry sweep. `SIGKILL` on a five-package sync left one `InProgress` entry; `heal` reconciled it
against the machine. Asked the adversarial version — an `InProgress` install for a package that
does not exist — `heal` **tried the recovery, failed, and refused to close the entry**, which is
the right answer and the one a "just mark everything done" implementation would have got wrong.

**Four things hold it at B−.**

**The keyword defect (R-1).** Above. `link`, `service`, `setting`, `shim`, `schedule`, `repo`,
`when`, `absent`, `if`, `else`, `end`, `import`, `include` — every one becomes a package. Only
`use` refuses. With their punctuation (`link:`, `when linux {`) the grammar refuses correctly, so
this is not a broken parser; it is an ambiguity in the language, and the fix is a ruling rather
than a patch.

**`cargo test` is red on Linux and macOS and green on Windows.** Three targets, three causes, and
only one of them is nobody's fault: `grade2_flag_drift_blindspot_tests` is red pending the owner's
ruling on **Q14**; `grade3_resource_idempotency_tests` is red because of a defect in **my own**
round-4 test; and `grade2_info_tests` is red because of a **live macOS defect** — `linix list`
reports `service:com.apple.SafariHistoryServiceAgent` and `linix info` about that exact name
answers *"is not installed on this machine, so there is nothing to describe."* The rubric's own
words for that class: *a `list` that disagrees with the machine breaks the one thing it promises.*

**`adopt` re-declares what you already declared, and explains its own count with a reason that is
never the reason (R-2).**

**The classification is computed and then not consulted (R-3, R-6).** `Error::RateLimit` is
`Transient` in `error.rs:226`, and the message layer tells the user *"Nothing classified the
failure above."* `heal` knows an npm 404 is `Permanent, absent_name: true` and advises re-running
`sync` anyway — while printing the Rust struct that carries those fields at the user.

**What a user can rely on today:** the package engine; the removal guard, on every path, from the
code; protection that `--allow-mass-removal` cannot override; refusals that exit 3; `--dry-run` as
a promise that nothing changed, on every verb reachable from a fixture and on the three that used
to be excused; the WAL surviving a `SIGKILL` and `heal` reconciling honestly rather than
optimistically; `check`/`plan`/`apply` agreeing about resources; and 32 of 56 backends with a real
install → list → binary → remove somewhere in this CI run.

**What they cannot:** a manifest line that is a bare word — the grammar will take a keyword as a
package and every preview will agree with it; `adopt` not to duplicate a declaration you wrote by
hand; an error message to name the file when the offending character is invisible; `heal`'s exit
code to mean anything; and `linix info` on macOS to agree with `linix list`.

---

## 1. Baseline, regenerated

```
cargo build --all-targets                                    ok
cargo fmt -- --check                                         ok
cargo clippy --all-targets --all-features -- -D warnings     ok   (clean)
cargo test                                                   ok on Windows: 1542 tests, 49 targets
                                                             RED on ubuntu-latest and macos-latest
git rev-list --left-right --count origin/main...HEAD         0    55      (nothing unpushed)
```

Four of five green. The fifth is green only where the developer sits — which is the S33/S34 class
this repo has been bitten by twice, and the reason the number above is written per platform.

### 1.1 Coverage ledger

The registry is platform-conditional, so the denominator is too: `check health` says **56 total**
on Linux and **48** on Windows.

| host class | real lifecycles | backends with a real install → list → binary → remove |
|---|---|---|
| `container-linux-tools-local` | 25 | apt asdf bun cabal cargo composer conda dotnet emacs gem github go helm krew luarocks mise mix nimble npm opam pipx pixi pub spack uv |
| `container-linux-fedora-local` | 9 | dnf bun cargo gem github npm pip pipx uv |
| `container-linux-ubuntu-local` | 7 | apt cargo gem github npm pipx uv |
| `container-linux-arch-local` | 7 | pacman cargo gem github npm pipx uv |
| `container-linux-alpine-local` | 7 | apk cargo gem github npm pipx uv |
| `windows-native-windows-ci` | 10 | scoop cargo dotnet gem github go helm npm pipx yarn |
| `windows-native-darwin-ci` | 8 recorded, **7 today** | brew cargo dotnet gem github npm pipx yarn |

**Union: 32 distinct backends.** That is better than the brief expected (*"roughly 22 of 52 real"*)
and worth crediting. The 24 with no real lifecycle anywhere are the ones needing hardware or a
daemon — btrfs, zfs, lvm, snap, flatpak, nix, appimage, psresource, vscode, stack, pkg, pkgin,
pkg_add, xbps, choco, winget, storage, link, service, setting, web, and the rest.

**One gap in the ledger itself:** nothing in the repo computes that union. `scripts/lifecycle-floor.txt`
records a per-host maximum and the per-run audit asks only *lifecycle **or** plan-smoke*, so the
question "how many backends have ever had a real round-trip" is not answerable from the repo. I
computed the row above by hand from six CI job logs.

### 1.2 Parser fixtures

Fourteen backends now have a fixture captured from their own tool, and I checked provenance rather
than assuming it: real paths, real timestamps, a real `pipx_metadata_version`. The four cases §3.3
asks for (empty, single, not-found, error) are covered for `choco`, `pixi`, `nimble` and `asdf`;
the rest have one case each.

---

## 2. Disposition of the round-4 findings

Each re-run by its **original** reproduction, not by a new test.

| id | verdict | evidence |
|---|---|---|
| **B-1** `--dry-run adopt` writes the registry | **closed** | Fresh config+data dir: `[DRY-RUN] would adopt 111 package(s)`, file set byte-identical before and after, `check drift` → *"the machine matches your files"*. Siblings `hold`/`unhold` closed too, each against a control that does write. |
| **P-1** `plan` predicts a refusal `apply` will not make | **closed** | Three `link:` lines undeclared: no warning, `apply` rc=0, all three undone. With `max_removals = 1`: `plan` warns, and `apply` **does** refuse at rc=3 with *"it removes 3 managed resources"* — right noun, right prediction. Re-verified on Linux. |
| **P-2** `sync` re-places every declared resource | **closed** | Three consecutive syncs: `already up to date`, three files, **no `.linix-backup`**. Re-verified on Linux by hand. |
| **P-3** `protected <bare name>` is wrong for every name | **closed** | `jq` → *no config rule matches (no backend named, so this machine's essential list was not consulted — ask `<backend>:jq` for that)*; `sudo` → *protected by config rule `sudo`*; glob `libfoo*` matches; `unprotected_packages` reported as *exempted by …rule*. |
| **P-4** pixi's `list` parser invents a package | **closed** | Real `pixi global list` has the nested `exposes: rg` row; `linix list -b pixi` now returns `ripgrep 15.2.0` and nothing else. |

**Correction to my own round 4.** I asserted, from reading `inspect_removals`, that
`unprotected_packages` could not release a protected *resource* because the `Extra` branch never
calls `unprotect_rule`. I then measured it, and it does — `config.protection_rule()` folds the
unprotect check in before the match. The finding was wrong and never left this session, but the
method that nearly shipped it is the one this document exists to police: I read two functions and
almost reported the third.

---

## 3. Findings beyond the list

Severity is user impact.

### R-1 — a grammar keyword on a line by itself becomes a package, and `check` tells you to install it — **high**

`link`, meaning the user started a `link:SRC @target=DEST` line and stopped:

```
$ cat config/modules/kw.txt
link

$ linix eval
  "present": [ { "backend": "cargo", "name": "link", "source": "modules/kw.txt:1" } ]

$ linix --dry-run sync -y
Planned changes:
  install 1   remove 0   (total 1 change(s))
  backends: cargo

$ linix check
ok  config      1 package(s) declared
->  drift       1 to install, 0 to remove, 0 to place, 0 to undo
                   run `linix sync`
```

Thirteen of fourteen keywords tested behave this way, and each resolves to a **real** backend that
claims a **real** package of that name — the resolver searched live indexes to get these:

```
PACKAGE   when       -> cargo:when          PACKAGE   repo       -> cargo:repo
PACKAGE   absent     -> pip:absent          PACKAGE   if         -> gem:if
PACKAGE   link       -> cargo:link          PACKAGE   else       -> npm:else
PACKAGE   service    -> cargo:service       PACKAGE   end        -> cargo:end
PACKAGE   setting    -> cargo:setting       PACKAGE   import     -> gem:import
PACKAGE   shim       -> scoop:shim          PACKAGE   include    -> cargo:include
PACKAGE   schedule   -> cargo:schedule      refused   use        rc=1
```

**Why, and why this is a ruling and not a patch.** A package name is one bare word (II.2), so a
bare keyword is a grammatically valid package line and the parser is behaving as specified. Written
with their punctuation the same keywords refuse correctly and legibly — `link:`, `service:`,
`shim:`, `when linux`, `when linux {` all exit 1 with a located `Configuration error`. So the
grammar is sound where a delimiter exists; the ambiguity is confined to the bare word. What to do
about it — reserve the words, warn on them, require a backend prefix for any name that collides —
is a language decision, and per `CLAUDE.md` belongs in `decisions.md` rather than in code. **Opened
as `Q16`** in the register in the same commit as this document, with the four options and the
measurements above; the grader's lean is stated there and the ruling is the owner's.

**The symptom is reportable regardless of the ruling:** nothing anywhere says *"`link` is a
keyword; did you mean `link:PATH`?"*, and `check` actively recommends the sync.

**Family.** All fourteen keywords tested bare; the seven likeliest near-misses tested with
punctuation and all refuse. Not tested: a keyword as the *name half* of a qualified line
(`cargo:link` is presumably fine and intended).

**Side effect worth its own line:** resolving one of these costs **10–27 seconds**, because the
bare name has no backend and the resolver asks every manager in priority order. Same fixture with
`cargo:ripgrep` instead: 0.2s. A typo is 50× slower than a correct line.

### R-2 — `adopt` re-declares what you already declared, and its summary explains the skips with a reason that is never the reason — **medium**

`src/app/adopt.rs:117-121`, `:154`, `:281`, `:315`. One package declared, and installed:

```
$ cat config/modules/mine.txt
cargo:ripgrep
$ linix check config
OK: every module and profile checks out … 1 present, 0 absent, 0 repo/shim/service/link/schedule line(s).

$ linix adopt -y
Adopted 111 package(s).
Manifest:  …/config/modules/adopted.txt
Left alone: 185 (listed in the manifest)          <- the manifest has one line
…
Deleting a line UNINSTALLS that package on the next sync.

$ grep '^cargo:ripgrep' config/modules/adopted.txt
cargo:ripgrep                                      <- now declared in two modules
```

Two defects with one root.

**The filter reads the wrong file.** `discover()` keeps a candidate when
`!state_guard.is_managed(&pkg.backend, &pkg.name)` — the managed-state **registry**. Nothing in
`discover` reads the manifests at all. So a package declared by hand and not yet synced is offered
again.

**The label describes a filter that does not exist.** `found.skipped` has exactly two push sites —
`:154` (the OS reports it essential) and `:315` (`hold_back_what_cannot_be_written`) — and the
summary prints `found.skipped.len()` under *"(listed in the manifest)"*. That reason is wrong for
100% of the items, always. Each `Skipped` already carries a correct per-item `reason`; the rollup
discards them for one that is never true.

**Consequence, driven to the end.** The user deletes their own line, as the paragraph three lines
below the count instructs:

```
$ : > config/modules/mine.txt
$ linix --dry-run sync -y
already up to date
$ linix why cargo:ripgrep
  declared:    at …/config/modules/adopted.txt:44 (module:adopted, profile:Main)
```

Deleting a line does not uninstall the package, because `adopt` wrote a second declaration without
saying so. **It fails safe** — nothing is removed — which is what keeps this at medium rather than
higher.

### R-3 — a failure LiNix classifies `Transient` is reported as "Nothing classified the failure above" — **medium**

`src/verbs/packages.rs:274-286`. Observed live on the macOS runner in this run:

```
Error: API rate limit: api.github.com is rate limiting this machine and does not reset for
1236s, past the 30s ceiling. Raise `rate_limit_max_wait_secs` … or set GITHUB_TOKEN …
 WARN `github:sharkdp/fd` is still declared in …/imperative.txt, so `sync` will try it again.
      Nothing classified the failure above, so if it repeats unchanged the cause is not a
      passing one — run `linix unmanage github:sharkdp/fd` if you did not mean it.
```

`error.rs:226` says the opposite, in as many words:

```rust
// The whole point of a rate limit is that the window moves.
Error::RateLimit(_) | Error::Http(_) => Retryability::Transient,
```

`why_kept` branches on `Refused`, `Exhausted` and `says_a_name_is_absent`, then falls through to
`Unclassified`. **There is no `Transient` branch**, and `WhyKept` has no variant for "this is known
to be temporary, and here is the window" — even though the window is in the error text one line
above. The advice that follows inverts the truth: a rate limit repeats unchanged *because* it is
passing.

**It cascades into two red CI jobs.** `classify_install` in the sweep harness tests transience by
running the install a second time immediately — a proxy that cannot pass for a 1236-second window
— so it scored `defect`, the macOS job went red, the real-lifecycle ratchet fell 8 → 7 and went red
as a second failure. The brief asked for exactly this to be driven off LiNix's own `Retryability`
(§2.2); LiNix computes it correctly and no one downstream can see it. The next person to "fix" the
ratchet by lowering the floor to 7 will have ratcheted macOS coverage down over a rate limit.

**Family, from the code.** Two production consumers of `retryability()` outside tests:
`transaction.rs:551` (`give_up = … == Permanent`, correct — it retries everything non-permanent)
and `why_kept`. R-6 below is the same root in a third place that does not consult it at all.

### R-4 — `linix list` and `linix info` contradict each other on macOS — **medium-high, and unfixed**

`tests/grade2_info_tests.rs:179`, red on `macos-latest` in this run:

```
the control failed — `info service:com.apple.SafariHistoryServiceAgent` denies a package `list`
just reported … 'service:com.apple.SafariHistoryServiceAgent' is not installed on this machine,
so there is nothing to describe.
```

The test takes the first thing `list` reports and asks `info` about it — the right instrument, and
it is doing its job. The `service` backend enumerates launchd agents that `info` cannot then
resolve. **Not reproduced by me** — I have no Mac; this is CI's result read from the log, and it is
the one CI red that is a product defect rather than an open question or a broken fixture.

### R-5 — a refusal that names no file, about a character you cannot see, and echoes it into your terminal — **medium**

A 60-line module with one bad name at line 40:

```
cargo:<U+202E>reversed   ->  Error: Validation error: Invalid characters in package name: ‮reversed
cargo:<ESC>[31mred…      ->  Error: Validation error: Invalid characters in package name: [31mred[0m
cargo:aaa…(300 chars)    ->  Error: Configuration error: …/big.txt:40: …          <- located
cargo:rip<TAB>grep       ->  Error: Configuration error: …/big.txt:40: …          <- located
```

Two error classes: the grammar's refusals name `file:line` and are excellent (*"expected a package
(`apt:curl`), one of `absent:`, `repo:` … `cargo:rip\tgrep` is none of those"*). The character
validator's refusals name **neither the file nor the line** — and the offending character is a bidi
override, a NUL or an escape, so you cannot find it by looking either.

And it echoes. Byte-level, in the container, so neither a pipe nor a terminal could be inventing it:

```
=== the manifest, in bytes:
0000000   c   a   r   g   o   : 033   [   3   1   m   r   e   d 033   [ …
=== what linix prints, in bytes:
0000060   a   c   k   a   g   e       n   a   m   e   :     033   [   3   1   m   r   e   d 033 …
0000060   a   c   k   a   g   e       n   a   m   e   :     342 200 256   r   e   v   e   r …
```

`342 200 256` is U+202E RIGHT-TO-LEFT OVERRIDE — the trojan-source character — reprinted raw by the
message whose subject is that the characters are invalid. Manifests can arrive from a shared config,
which is what lifts this above self-inflicted.

### R-6 — `heal` reports an unrecovered operation at rc=0, in Rust's `Debug` syntax, with advice its own classifier contradicts — **medium**

A valid `InProgress` install planted for a package that does not exist, then `linix heal -y`:

```
ERROR could not recover npm:definitely-not-installed-zzz — Some(CommandFailed { message: "`npm`
failed (exit 1): npm ERR! code E404 …", retry: Permanent, absent_name: true }). The system may be
in a partial state for this package; re-run `linix sync`.
 WARN 1 operation(s) could NOT be recovered: npm:definitely-not-installed-zzz. Re-run `linix sync`.
heal: reconciled locks/versions.json (1 entries)
heal: refreshed backend metadata
heal rc=0
```

**The behaviour underneath is right** and I want that on the record: `heal` attempted the recovery,
failed, and **left the entry `InProgress`** rather than closing it, and `list` does not claim the
phantom. That is the answer a "mark everything done" implementation gets wrong, and it is correct
here.

Three defects in how it says so:

1. **rc=0** after *"1 operation(s) could NOT be recovered"*. `linix heal && echo ok` prints ok. U21
   gave this program an exit vocabulary and the recovery path does not use it.
2. **`Some(CommandFailed { … retry: Permanent, absent_name: true })`** — `{:?}` on an
   `Option<Error>` printed at the user. `absent_name` is an internal field the N-1 fix introduced
   this month.
3. **The advice contradicts the struct it just printed.** `absent_name: true, retry: Permanent`
   means the name does not exist; `packages.rs` has a whole `WhyKept::NameAbsentElsewhere` branch
   whose wording is *"`sync` will keep failing the same way until the line naming it is corrected"*.
   `heal` says *"re-run `linix sync`"*.

Last words on screen are two successes and rc=0.

### R-7 — the mutation gate has a ceiling and no floor — **low**

`scripts/harness-mutation-test.sh` fails when survivors exceed the budget (92 container / 86
Windows). Nothing asserts `CAUGHT`. Proven rather than argued — pointed at a harness with three
checks:

```
   1 check(s) caught the do-nothing binary
   2 check(s) passed anyway …
 ok: 2 survivors, within the budget of 92; 1 checks did their job.        exit 0
```

It cannot tell "the checks got stronger" from "the checks were deleted". A *total* collapse is
caught by the lifecycle ratchet and the subcommand audit; an assertion-strength collapse — deleting
the effect assertions while still invoking every subcommand — passes all three gates.

### R-8 — one orphan fixture — **low**

`tests/fixtures/cargo/install-list.txt` was captured from the real tool in `08790c3` and is read by
no test. I swept all 30 fixtures; it is the only one. The cargo parser is correct — I checked
against the live tool — so this is a false signal of coverage rather than a hidden bug, in the same
commit that established the rule fixtures exist to serve.

### R-9 — nothing measures latency, and it is worth measuring — **medium (as a budget, not a bug)**

Release binary, five samples, this machine (24 ready backends, ~470 installed packages):

```
linix list           min  6.13s   median 20.43s   max 40.41s
linix check health   min  8.47s   median 18.58s   max 35.92s
linix check          min 17.02s   median 18.71s   max 55.40s
linix policy / vars / eval / check config          ~0.25s across the board
```

The split is clean and diagnostic: config-only commands are instant; anything that queries the
managers is 6–55 seconds with a 6× spread on the *same command*. §9's suggested budget is 2s for
read-only commands. Treat this machine as an upper bound — but the variance is the finding, because
nothing measures it and so nobody knows which end a given user gets.

### Things that held up, checked adversarially rather than assumed

- **Every path to a backend `remove`/`purge` passes a guard.** Enumerated from the sink (nine
  sites), not from the command list: extras teardown, lease sweep, sync engine, transaction
  executor, `purge-unmanaged`, `remove-orphans`, `uninstall`, `apply`, rollback. Every builder of a
  `GraphAction::Remove` node — including `app/shell/mod.rs:295`, the ephemeral-shell exit — routes
  through a `GuardScope`. All twelve scopes have call sites.
- **Rollback checks protection before removing what it installed** (`transaction.rs:691`) and names
  the package it left behind.
- **`SIGKILL` mid-sync leaves a reconcilable journal**, and `heal` reconciles it against the machine.
- **A corrupt WAL is handled beautifully**: names the file, the missing field, the line and column,
  moves it to `.corrupt`, starts fresh, and says an interrupted operation cannot be auto-recovered.
- **EPIPE**: `list`, `search`, `check health` and `policy` all survive `| head -1` at rc=0, no panic.
- **No panics anywhere in the hostile-manifest sweep** — 18 cases including 10k lines, a 100k-char
  name, embedded NULs, a BOM, CRLF, cyclic `use`, 60-deep `when` nesting, Windows reserved names.
- **All eleven verbs exempted as "read-only" write nothing** — whole-fixture hash comparison over
  config, data and the working directory.
- **The five softs in the ubuntu container each name a structural reason** (FUSE, PEP 668,
  dependent statement, no public canary). The "ecosystem/network variance" catch-all is gone from
  that leg.
- **The container pre-flight self-tests its own CRLF detector** before trusting it, because MSYS
  `grep` normalises away the byte it was written to find.

---

## 4. What I could not test, and what it would take

- **macOS, directly.** R-4 is read from a CI log, not reproduced. Needs a Mac or a runner I can
  drive interactively.
- **`sudo` on any privileged path.** Untested here and untested in the repo. Needs a container with
  a password-protected user and a pty.
- **The destructive effectors** — btrfs/zfs/lvm snapshot restore, `dpkg -i`/`rpm -U` handoff, U30
  storage removal. Needs disposable VMs with real filesystems; a container cannot do it.
- **`SIGKILL` at *every* step boundary in a loop.** I did one kill at one boundary. The brief asks
  for the loop; that is a harness, not a session.
- **Property and model-based testing** over the declarative core. None exists; none written.
- **`--dry-run export`.** Inconclusive — it emits from managed state, and a fixture with nothing
  installed has nothing to emit. `--dry-run bundle` **does** write its nine files and prints
  *"Bundle written to …"* with no marker, which is data for **Q15**, not a verdict.
- **`psresource`, `snap`, `flatpak`, `nix`, `vscode`, `stack`, `conda` and the BSD backends** — no
  real lifecycle anywhere, and none available on this machine.
- **Untrusted input arriving *from a manager*** (a package whose name in the tool's own output
  carries a newline or an escape). I tested the manifest direction only.
- **The four handle combinations** for a mutation (stdin tty/pipe × stderr tty/pipe). Needs a real
  pty harness on Linux.
- **An automated test for R-6.** Planting a WAL entry whose recovery fails needs either a network
  round-trip or a backend that is present on every runner, and my first attempt at one landed in the
  corrupt-WAL branch instead: a hand-written `Install` entry omitting `options` is rejected before
  recovery is attempted. Worth recording because the dry-run gate's `heal` fixture
  (`tests/dry_run_every_verb_tests.rs`) plants a `Remove` entry of the same hand-written shape — that
  one *does* parse, which I checked rather than assumed, so the gate is sound; but the margin
  between "exercises recovery" and "exercises the corrupt-WAL path" is one absent field.

---

## 5. The failing tests

Committed red, and each one watched failing on **both** Windows and Linux before it was committed.
Every file carries a green control alongside the red assertions, so a future green cannot mean the
instrument stopped looking.

| file | red | green control | for |
|---|---|---|---|
| `tests/grade4_keyword_is_not_a_package_tests.rs` | 2 | 1 — the same keywords *with* their colon are refused | R-1 |
| `tests/grade4_adopt_respects_the_manifest_tests.rs` | 2 | `check config` confirms the package is declared before `adopt` runs | R-2 |
| `src/verbs/packages.rs` (`mod tests`) | 2 | 1 — the sibling asserting a transient failure keeps its line | R-3 |
| `tests/grade4_refusal_names_the_line_tests.rs` | 2 | 1 — the grammar's own refusal names the line, in the same fixture | R-5 |

R-3's tests live in the source's own test module because `why_kept` and `kept_line_advice` are
private and `verbs` is not in the library crate; they run under `cargo test --bin linix`.

**No committed test for R-4 or R-6.** R-4 needs a Mac. R-6 needs a planted WAL entry whose recovery
fails without a network round-trip, and every construction I tried was either host-dependent or
tested the corrupt-WAL branch instead of the recovery branch — which is itself worth knowing, and is
recorded in §4.

**And one repair, disclosed rather than filed as a finding — it is mine.**
`tests/grade3_resource_idempotency_tests.rs`, from round 4, had **two** environmental accidents in
it, and the second only surfaced after fixing the first:

1. Its `link:` targets sit wherever the checkout does, and placement asks before writing outside the
   home directory. On this machine the checkout is under `C:\Users\Administrator`, so it passed.
   Fixed by handing the child fixture root as `HOME`/`USERPROFILE`, which makes the targets inside
   home *by construction* rather than by luck.
2. Its central assertion counted occurrences of `Link:` in the output — which is the text of the
   **Windows-only** cross-drive-fallback warning. On Linux the count is zero whatever `sync` does.
   Replaced with modification times, which every platform has, plus a self-test that writing a file
   really does move its mtime on the filesystem under test — because otherwise the comparison could
   not fail.

That is the S33 shape twice over with my name on it, and it is exactly the failure this document
spends its first section on: *a check that examines the wrong thing and reports success.* Both
halves are now green on Windows and in a Linux container; the assertions' subject is unchanged, and
the behaviour they cover I re-verified by hand before touching them.

---

## 6. Grade against `READINESS` §8.1

| area | round 4 | now | why |
|---|---|---|---|
| Core engine & safety model | A− | **A−** | Guard verified from the sink across all nine removal paths, recovery paths included. WAL survives `SIGKILL`; `heal` verifies against the machine rather than closing entries. Held at A− by R-6: the recovery path reports failure at rc=0. |
| Declarative model & grammar | B+ | **B−** | R-1. A bare keyword becomes a package, resolves against a real index, and `check` recommends the install. The model is coherent everywhere a delimiter exists — and the most likely typo class produces a wrong model that every downstream check faithfully agrees with. |
| Backend layer | D → C+ (r4) | **C+** | 32 of 56 with a real round-trip, up from the brief's expected ~22 of 52; the `tools` image passed 353 checks with 0 failures; parsers now have fixtures from their own tools. Held down by R-4 — a `list`/`info` contradiction on a shipping platform is the exact failure the rubric weights heaviest. |
| Test & gate infrastructure | D− → C (r4) | **C+** | The dry-run gate walks the data dir and drives its three excused verbs; exemption reasons now describe the fixture, not the instrument; the lifecycle ratchet caught a real coverage collapse today without being asked. Held by R-7 (no floor under `CAUGHT`), R-8, and a suite that cannot go green on two of three platforms. |
| Documentation vs. reality | B− | **B** | Q14 and Q15 are open in the register rather than answered in code, which is the discipline working. `lifecycle-floor.txt` explains its own design. Docked for R-2's "listed in the manifest". |

**Overall: B−.**

**Against the rubric's own bars.** The **B** bar asks that no check in any harness can pass without
examining the thing it names; the measured survivor counts are 92 and 86, so roughly a third still
can. It asks that the native sweep run in CI on Windows and macOS — **met**, both ran. The **C** bar
asks for tests green, and they are green on one platform of three. The **A** bar needs 56 of 56, and
the **A+** bar's last line — *no user-visible failure without a file or command to act on* — is
contradicted by R-5 and R-6 in this document.

### What moves each area up one letter

- **Core engine → A.** Give `heal` an exit code that means something and a sentence that does not
  contain `CommandFailed { … }`. The behaviour is already right; only the report is wrong.
- **Declarative model → B+.** Rule R-1 in `decisions.md`, then build the ruling. Any of the three
  candidate answers closes it; leaving the ambiguity open does not.
- **Backend layer → B−.** Fix R-4, and get a real lifecycle for the ten backends that could have one
  in a container today but do not (`snap`, `flatpak`, `nix`, `conda`, `stack`, `vscode` …).
- **Test infrastructure → B−.** A floor under `CAUGHT`; drive the harness's transience off
  `Retryability` instead of "did an immediate retry succeed"; get `cargo test` green on all three
  platforms, which needs Q14 ruled.
- **Documentation → B+.** One pass over every rollup count that explains itself with a reason
  belonging to one of its inputs. `adopt`'s is the one I found; `unhold`'s *"0 hold(s) were not
  recorded"* after releasing one is the same shape.

---

## 7. The two things to do first

1. **Rule R-1.** It is the only finding here that installs software the user never named, and it is
   the only one that cannot be fixed without a decision. Every preview in the program agrees with the
   wrong answer, so no further gate will catch it.
2. **Make `cargo test` green on three platforms.** Not because green is the goal — this document's
   first line is that green is a floor — but because a suite that is red for three different reasons
   teaches everyone to stop reading it, and one of those three reasons (R-4) is a real defect that is
   currently indistinguishable from the two that are not.
