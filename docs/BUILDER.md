# YOU ARE THE BUILDER

**Your job: raise LiNix from C+ to A by writing code.** Work through the numbered work orders in
this document, in tier order. Everything you need is here or named here.

You are working in the LiNix repo at the path given to you. Start by reading, in this order:

1. `docs/READINESS-2026-07-27.md` **§5** (why the bugs cluster where they do) and **§8.1** (the
   rubric you are being graded against). Do not skip these — they are why the work orders are
   shaped the way they are.
2. `CLAUDE.md` and `docs/SPEC.md` — the repo's binding rules.
3. This document, in full, before you touch anything.

**You are not being asked to make the tests pass. You are being asked to make the checks capable
of failing, and then make them pass.** Those are different jobs, and this codebase has repeatedly
done the first while believing it did the second. A green suite here has co-existed with every
defect on the list.

**There is a separate Grader.** When you finish, a different agent gets `docs/GRADER.md` and the
repo — without your report — and independently re-runs every original reproduction. You cannot
satisfy it by writing a passing test; it re-runs the *old* repro and checks it no longer
reproduces. Build accordingly.

**When you are done**, report in the format in §"How to report" at the end, and hand off.

---

## 0. Rules that bind every work order below

From `CLAUDE.md`, and they are not optional:

1. **Test-first.** Write the failing test, *run it, watch it fail*, then fix. A test you did not
   watch fail is not a test — three assertions in this repo have shipped unable to fail.
2. **Fix the whole family.** Each order below has a **Siblings** line. A fix that patches only the
   reported line has hidden the bug, not fixed it. When you report, name the sibling sites you
   checked **including the ones you cleared, and why**.
3. **No legacy.** When a thing is replaced, the old thing is deleted in the same change —
   config keys, docs and tests included.
4. **Verify:** `cargo build --all-targets` → `cargo test` → `cargo clippy --all-targets
   --all-features -- -D warnings` → `cargo fmt -- --check`. Unverified is not done.
5. **Commit per work order**, message saying what changed and what it does *not* yet do.

### 0.1 STOP AND ASK — do not implement these unilaterally

`CLAUDE.md` requires a ruling for: anything with a register ID (`D* W* K* N* T* U*`), anything a
user would notice, anything that removes a feature, anything where Part II looks wrong.

| order | why it needs a ruling |
|---|---|
| **W1** (manifest withdrawal) | Changes what happens after a failed install. The two harnesses disagree *in writing* about the intent — see `READINESS` §3.1. **Get the ruling, write it into `decisions.md` in the same commit.** |
| **W12** (exit codes) | Changes a published contract (`readme.md:708`) that scripts branch on. |
| **W13** (`check health` severity) | Changes what every user sees on a healthy machine. |
| **W16** (supported vs experimental) | Re-labels backends users are relying on today. This is the highest-value change in the document and the most user-visible. |

Everything else in this brief is internal correctness. **Build it without asking.**

### 0.2 Reading order

**Work the tiers in order; the `W` numbers are labels, not a sequence.** Tiers 1 and 2 are
prerequisites for trusting anything later — until Tier 2 lands, a green run is not evidence.
All 36 defects `E1`–`E34` in `READINESS` are covered by exactly one work order; the mapping is
complete, so nothing is silently dropped.

### 0.3 Definition of done

A work order is done when **all** hold:

- The test you wrote fails on the pre-fix tree and passes on the post-fix tree. Paste both runs.
- The siblings are named and addressed or cleared with a reason.
- The four verify commands are green.
- **An independent reproduction exists** — a command someone else can run to see the fix, stated
  in the commit message. Not "tests pass".

---

## Tier 1 — Correctness of the core promise

These break `sync` itself. Do them first.

### W1 · `E1` — a permanently-failed install wedges the config

**Symptom.** `linix install scoop:<typo>` fails, the line is written to
`modules/imperative.txt` anyway, and every later command that parses the model then fails. The
only escape is hand-editing a file nothing names.

**Root cause (verified).** `src/verbs/packages.rs:110-119` withdraws the line only when the error
downcasts to `Error::Unresolvable`. A qualified `backend:typo` **resolves fine** — the backend is
real — so the failure is `CommandFailed` and the line stays. The design intent (transient
failures keep the line so a retry works) is right; the classification is incomplete.

**The fix is already computed elsewhere.** `src/core/exit_policy.rs` returns
`Retryability::{Transient, Permanent, Unknown}`, and scoop's policy at `:194` already marks this
exact failure `Permanent`. Widen the condition:

```rust
// withdraw when the name can never be satisfied — unresolvable, OR a failure the
// backend's own policy classifies as permanent.
if matches!(e.downcast_ref(), Some(Error::Unresolvable { .. }))
    || retryability_of(&synced) == Retryability::Permanent
{ … }
```

**Also required, regardless of the ruling:** when a line *is* deliberately left behind, the error
must name `modules/imperative.txt` and suggest `linix unmanage <name>`. A wedge with an exit is
not a wedge.

**Test first.** `tests/` — install a `backend:nonexistent` against a mock backend whose policy
marks the failure permanent; assert the manifest is unchanged **and** that a second command still
parses the model. Then the transient case: assert the line *is* kept.

**Siblings.** `linix add`, `adopt`, `activate` also write manifest lines — `READINESS` §3.1 clears
all three with reasons; re-verify rather than trust. Check `handle_uninstall` still removes
correctly.

**Acceptance.** From a clean config: `linix install scoop:definitely-not-real-xyz123 -y` then
`linix check` — `check` must succeed.

### W2 · `E6` — `go`'s `list` is blind to a package that is installed

**Symptom.** `PASS go installed … for real` / `PASS go: hello is on PATH` / **`FAIL go: list shows
hello`**. Verified against disk: `~/go/bin/hello.exe` exists and is on `PATH`.

**Why it matters most.** `sync` compares desired against `list`. A `list` that cannot see an
installed package produces permanent phantom drift — LiNix reinstalls forever and `check` reports
a problem that is not there.

**Root cause.** Not isolated. Start at the `go` backend's list parser and its `manual`/listing
mode in `src/backends/go.rs` and `src/backends/registry.rs`. `go install` records nothing in a
manifest; the listing is derived from `$GOBIN`/`$GOPATH/bin` contents, and the module path
(`golang.org/x/example/hello`) is not the binary name (`hello`). **Suspect the identity mapping,
not the parser.**

**Test first.** Fixture-driven: real `go` listing output plus a real `$GOPATH/bin` layout →
assert the installed module is reported under the name `sync` will look for.

**Siblings.** Every backend whose install name ≠ binary name: `go`, `github` (`owner/repo`),
`pub`, `krew`, `cargo` (crate vs bin). `cargo` passes today — use it as the reference.

**Acceptance.** `linix install go:golang.org/x/example/hello -y && linix list -b go | grep hello`.

### W3 · `E6b` — `nimble` install produces no binary and `list` cannot see it

**Symptom.** Two hard failures: `list shows nimjson` and `nimjson is on PATH`.
`~/.nimble/bin` **is** on `PATH` and was never created.

**Root cause is open — do not guess it.** Investigate whether `nimble install` needs
`--accept`/`-y`, whether it installed only the dependency (`jsony` was present), and whether
nimble on Windows places binaries elsewhere. **Report the diagnosis before the patch.**

**Acceptance.** Full round-trip green in the native sweep, asserted while installed.

### W4 · `E6c` — a successful install the user cannot invoke

**Symptom.** `pub` installs correctly and `list` is correct, but the binary is unreachable:
`~/.pub-cache/bin` is not on `PATH`. LiNix reports success and says nothing.

**Fix.** After a successful install, if the backend's bin directory is not on `PATH`, say so —
naming the directory and the shell line to add. One warning, not a refusal.

**Siblings.** Every per-user-bin ecosystem: `pub`, `nimble`, `go`, `cargo`, `gem`, `luarocks`,
`mix`, `stack`, `krew`, `pipx`, `composer`. Implement once on the shared path, not per backend.

**Acceptance.** With `~/.pub-cache/bin` off `PATH`, `linix install pub:sass` warns by name.

---

## Tier 2 — Make the checks capable of failing

Until these land, every later result is unreliable — including your own.

### W5 · `E2` — the assertion that deletes its own evidence

`docker/integration/run-in-container.sh:261-267` and `scripts/integration-windows.sh:269-274`
`grep -v` the line out of the manifest and then assert it is absent. **Delete the scrub. Let the
assertion test the product.** It will go red until W1 lands — that is correct and is the point.

**Siblings.** Audit both harnesses for any mutation (`mv`, `rm`, `sed`, `grep -v`, `unmanage`,
`|| true`) within ~5 lines above an `ok`/`nok`. Then run the mutation experiment in
`GRADER` §2.1: stub `linix` to exit 0 always, and treat **every still-passing check as a
check that tests nothing.**

**Acceptance.** With a do-nothing `linix` stub, both harnesses fail loudly instead of passing.

### W6 · `E5` — the catch-all that launders real defects

Both harnesses soften *any* install failure to
`soft "<backend>: install of <pkg> failed (ecosystem/network variance)"` and **skip that
backend's whole remaining lifecycle**. It fired four times in one run and not once was it network
variance: one was LiNix correctly refusing, two were real backend defects (`helm`, `luarocks`).

**Fix — classify instead of assuming.** LiNix already exposes the distinction:

| observed | verdict |
|---|---|
| `Retryability::Transient` | `soft`, and retry once |
| `Retryability::Permanent` | **hard fail** |
| exit code 3 / `Error::Refused` | its own outcome — a refusal is not a failure |
| otherwise | hard fail |

**Acceptance.** Point a backend at a package that cannot exist → the harness reports a failure,
not a soft pass. Point it at something LiNix should refuse → reported as a refusal.

### W7 · `E3`,`E4` — the ship gate is weaker than CI

`release-check.ps1` and `release-check.sh` rate `cargo fmt --check` *informational*;
`.github/workflows/ci.yml:72` rates it fatal. Make both **hard**. Then run `cargo fmt` (26 diffs
across 10 files today) and diff *every* local gate step against every CI step, reporting
asymmetries in both directions.

### W8 · `E26` — 10 commits CI has never seen

5,082 inserted lines including a 711-line executor rewrite, compiled and tested on Windows only.
After W7, **push and get a verdict.** The last time this repo carried a large unpushed backlog,
the first CI run that saw it failed on all three platforms on two distinct bugs.

### W9 — run the native sweep in CI

Every hard failure in this assessment came from `release-check.ps1`'s section 12, and **no
automated gate runs it on any platform.** Add a Windows job. Keep it off the per-push path if it
is slow, but it must run nightly and be consulted.

---

## Tier 3 — Backend correctness

### W10 · `E7`,`E8`,`E9` — scoop's exit code is always 0

**Root cause (verified).** Windows `PATHEXT` has no `.PS1`, so `which::which("scoop")` resolves
`scoop.cmd`; `windows_effective_command` (`src/core/executor.rs:231`) takes the `"cmd"|"bat"`
branch; `cmd /C scoop.cmd …` **does not propagate the child's exit code**. Measured:
`cmd /C scoop.cmd install <bad>` → prints the error, exits **0**.

Consequences: the careful `.ps1` branch at `:188-217` is **dead code on a default box**, and
scoop's entire success/failure determination falls to `ExitPolicy::signals_failure` with
**one** marker (`"find manifest for"`, `exit_policy.rs:194`). Any other scoop failure reads as
success — confirmed: `scoop uninstall <not-installed>` prints `ERROR 'x' isn't installed.` and
exits 0.

**Fix.** Prefer the `.ps1` shim when one exists alongside the `.cmd` (do not rely on `PATHEXT`),
so the verified-correct PowerShell branch is actually reached. Then widen scoop's markers.

**Siblings — this is the important half.** Enumerate every backend whose `ExitPolicy` carries
fewer than three `failure_markers` *and* whose program is reached through a shim. For each,
produce one real failure outside the existing markers and assert LiNix reports it. Also fix the
message: `` `scoop` failed (exit 0) `` is incoherent — when the verdict comes from output rather
than status, say so.

**Acceptance.** `linix uninstall scoop:<not-installed>` reports failure, not success.

### W11 · `E10` — `psresource` reports healthy and cannot run

`src/backends/psresource.rs:120` probes **PowerShell**, which always exists, not **PSResourceGet**,
which supplies its cmdlets and does not ship with Windows PowerShell 5.1. Result: `[READY]`
followed by `Get-InstalledPSResource : The term … is not recognized` on every operation.

**The codebase has the right pattern** — `src/backends/krew.rs:43` probes both `kubectl` and
`kubectl-krew`. Copy it: probe module availability, not host availability.

**The general fix, which is worth more than the specific one.** Add a gate: **every backend
reporting `[READY]` must successfully answer its cheapest real read (`list`).** A backend that
claims health and cannot answer `list` is lying, whatever the reason. This would have caught
`psresource` without anyone thinking about PowerShell.

**Siblings.** Audit every `is_available`/`check_health` for "probes the host, not the tool":
`psresource` (shell vs module), `setting` (adapter), `service` (init), `web`/`github`/`link`
(always true — correct, they are built in), `appimage` (`cfg!(target_os)` only).

### W12 · `E11`,`E12`,`E13` — three argv defects, and the gate that would have caught all three

| id | defect | fix |
|---|---|---|
| `E13` | `pixi global upgrade-all` was **removed upstream**; pixi 0.73 says use `global update` | `src/backends/registry.rs:1381` |
| `E11` | `helm` install fails: `plugin source does not support verification. Use --verify=false` | helm backend argv |
| `E12` | `luarocks` fails: `No results matching query were found for Lua 5.5` — no version pinned | luarocks backend argv |

**Do not stop at the three.** Build the **argv-drift gate** (`GRADER` §3.2): for every
manager installed on the runner, assert that every subcommand LiNix will invoke still appears in
that manager's own help output. This is the single highest-leverage test in the programme — it
converts silent upstream drift into a named failure, and it is the difference between fixing
`pixi` today and fixing its successor automatically.

### W13 · `E16`,`E17` — two search parsers emit junk

`pixi` is routed to `names_only` (`src/parsers/ecosystem.rs:60`), documented as "search prints
bare identifiers". **Real `pixi search` output is a detail record** (`Name`/`Version`/`Build`/…
plus a build table), so the parser emits `-`, `...` and bare version numbers as package names —
19 junk rows in one search. `choco` turns its own `5 packages found.` summary into a package
named `5`, and its banner into `Chocolatey v2.7.3`.

**The general defect:** `names_only` serves five managers and its only test uses a **spack**
fixture. The test passes and says nothing about the other four.

**The rule to enforce repo-wide:** *a parser is tested against output captured from the tool it
parses, and from no other tool.* Capture real `list`/`search`/`info` output per backend into
`tests/fixtures/<backend>/<verb>.txt`, and include **the empty, single-result, not-found and
error cases** — three of those four are where junk rows come from.

### W19 · `E18`,`E19` — one condition, two message families, both naming the wrong program

Two implementations render "this backend's program is missing", and a user sees **both in one
screen**:

```
[FAIL] cabal — `cabal` is not on PATH, so the `cabal` backend cannot run     <- generic.rs:219
[FAIL] snap  — Binary for snap not found in PATH                             <- manager.rs:40
```

`src/backends/generic.rs:219-236` is the better one — it distinguishes an absolute path from a
`PATH` lookup. The `src/core/manager.rs:40` default, inherited by the ~13 backends that implement
`BackendCore` directly, also **names the backend rather than the binary it actually probed**:

| backend | message says | actually probes |
|---|---|---|
| `lvm` | `Binary for lvm not found in PATH` | `lvs` |
| `xbps` | `Binary for xbps not found in PATH` | `xbps-install` |
| `krew` | `Binary for krew not found in PATH` | `kubectl` **and** `kubectl-krew` |
| `appimage` | `Binary for appimage not found in PATH` | nothing — it is a `cfg!(target_os)` gate |

**Fix by deletion, per `CLAUDE.md`'s "prefer deleting to fixing".** Remove the `manager.rs`
default in favour of `generic.rs`'s, parameterised by the program(s) actually probed. Two
implementations of one sentence is the "two of everything" disease in miniature.

**Acceptance.** `linix check health` on a machine missing several managers emits one message
shape, and each names the program it looked for.

---

## Tier 4 — The human path

### W14 · `E14`,`E15` — `info` takes 98s to return a wrong answer

`linix info cargo:ripgrep` → `Package 'cargo:ripgrep' not found in any available backend.` in
**1m37s**, while `linix search ripgrep` in the same tree returns `cargo ripgrep 15.2.0`. Two
commands in one program contradict each other and the wrong one is the slow one. The explicit
`cargo:` qualifier appears not to narrow the probe — start there.

**Then add latency budgets as a gate**: read-only commands under 2s, a qualified `info` under 5s.
Nothing measures latency today, which is how a 98-second `info` shipped.

### W15 · `E20`,`E21` — "23 critical" on a healthy machine ⚠️ needs a ruling (0.1)

`check health` opens with `Backends: 25 OK, 0 degraded, 23 critical (of 48 total)` on an ordinary
Windows box. Nothing is wrong — those are managers the user has not installed. Meanwhile the
`check` rollup says `ok health 25 backend(s) ready`. **The rollup and the detail view disagree
about the same machine.** Propose: *absent* as its own state, distinct from *critical*.

### W16 · `E27`,`E28`,`E30`,`E22`,`E23`,`E12` — the first hour

- First-run `linix sync` explains the `priority` format by hand and **never mentions `linix init`**
  (`E27`). It is the first command a new user runs.
- `linix init --help` promises "a starter module"; `modules/` is created empty (`E28`).
- One failure prints **three times** — `WARN linix::core::journal` with a 32-hex WAL id, `ERROR
  linix::core::transaction` naming a "Node", then `Error:` (`E30`). Print it once, in the user's
  words. Audit for leaked internals: `WAL`, `Node`, `DAG`, module paths, UUIDs.
- `purge-unmanaged` (`src/verbs/cleanup.rs:262`) and the auto-remediation confirm
  (`src/app/diagnostics.rs:147`) lack the `is_terminal` refusal their siblings have — measured,
  they fail *safe* with a bare `IO error: not a terminal` instead of the actionable message
  (`E22`,`E23`). Use the pattern at `src/verbs/cleanup.rs:338`.

**The rule worth adopting:** every user-visible failure names (a) what failed in the user's words,
(b) the file or command to act on, (c) exactly one place to look. Then test for it.

### W17 · `E24`,`E25` — the published exit-code contract is violated ⚠️ needs a ruling (0.1)

`readme.md:708` publishes four codes and says *"a script can branch on them"*. Measured:
`linix nosuchcommand`, `linix --nosuchflag` and `linix sync --badflag` all exit **2** — which the
table defines as *"a read-only command looked and found work to do"*. Clap's usage-error
convention is 2 and it exits before LiNix's `finish()` mapping runs. **A CI job following the
documented table reads a typo as drift**, which defeats the stated purpose of code 2.

Separately, a `purge-unmanaged` ratio refusal exits **1**, not the documented **3** — it is raised
with `anyhow::bail!` rather than `Error::Refused`. Audit every refusal for the same slip; the
native harness asserts these with `nok`, which accepts any non-zero code and cannot tell 1 from 3.

### W20 · `E29`,`E31`,`E32` — documentation that has drifted from the register

Small, but this repo's stated thesis is that undetected drift is what cost it 84 unanswerable
decisions. Each of these is a one-line fix plus a check that stops it recurring.

- **`E31`** — `docs/SPEC.md:16` says *"All 107 decisions. 105 answered, 2 parked"*;
  `docs/spec/decisions.md:64` says *"All 104 are ruled: 102 ANSWERED, 2 PARKED, 0 OPEN."* The
  register is the authority; the map is stale. **Then make the count generated, not typed** — two
  files that both track one number will disagree again.
- **`E32`** — `PRODUCTION-READINESS-REVIEW.md` uses `U1`/`U2`/`U3` for its own findings, but
  `CLAUDE.md` reserves `U*` for register IDs requiring an owner ruling, and the register's real
  `U1` is *"where does a custom backend definition live"* (ruled 2026-07-23). Rename that
  document's labels. It costs nothing and removes a live trap for exactly the agent reading these
  briefs.
- **`E29`** — both harnesses exempt `undo` from the coverage audit; `linix undo` does not exist
  (renamed to `snapshot`/`rollback`). Delete it, and **assert that every exempted name appears in
  `--help`** — `harness-logic-test.sh` already checks that every *invoked* subcommand exists and
  should do the same for every *exempted* one. That asymmetry is why this survived.

---

## Tier 5 — The structural change (this is the one that matters)

### W18 · `E33` and the whole of §5 — redefine "supported" ⚠️ needs a ruling (0.1)

Everything above fixes instances. **This fixes the generator.**

Measured: `check health` registers 48 backends on Windows, 56 on Ubuntu; the Ubuntu lifecycle
reports **7 real lifecycles against 49 plan-smokes**. A plan-smoke proves an argv was
*constructed*, never that it is *correct* — `pixi global upgrade-all` passes one and does not
exist. Every defect in this assessment lives in that remainder.

**Do this:**

1. A backend is **supported** only when it has passed a real install → `list` → binary → remove
   round-trip, asserted while installed, in an automated gate.
2. Everything else is **experimental**, and says so — in `check health`, in the `priority` file
   `init` writes, and in the readme.
3. `linix init` currently writes **23 managers** into `priority` on a fresh Windows box, most
   never run. Scaffold only supported ones by default.
4. **Stop adding backends until (1) is true for the current set.**

This converts an unbounded invisible-bug surface into an honest bounded claim. It is the only
item in this document that stops §5.1 from regenerating.

---

## Tier 6 — What A+ additionally requires

Tiers 1–5 reach **A**. A+ needs test *kinds* this repo does not have. Specified in full in
`docs/GRADER.md` §4–§6; summarised here so the bar is in one place:

- **Argv-drift gate** (W12) running nightly against every installed manager.
- **pty coverage on every read command**, not just `list`; all four stdin/stderr handle
  combinations for mutations; real `sudo` with a password in a container.
- **Destructive effectors in disposable VMs** — btrfs/zfs/lvm restore on loopback filesystems,
  `dpkg -i`/`rpm -U`, U30 storage removal. Currently argv-tested and unrun.
- **Crash recovery**: `SIGKILL` mid-transaction at every step boundary, then `heal`, asserting the
  machine and the journal agree. This is the WAL's entire reason to exist and nothing tests it
  under a real crash.
- **`--dry-run` performs nothing** — every mutating command against a fully instrumented fake
  backend, asserting **zero** mutating calls. This repo's flagship historical bug was a
  `--dry-run` that performed the removal.
- **Guard enumerated from the code**, not from a list, with a refusal test per removal path —
  recovery paths especially, since nobody is watching when they run.
- **Property and model-based tests**: `sync`∘`sync` = `sync`; `install`∘`uninstall` = identity on
  the manifest; `bundle`∘`restore` = identity; a reference model of desired state compared against
  `eval` after random command sequences.
- **Grammar fuzzing** — nothing panics, everything is a named refusal.
- **Latency budgets** enforced per command class (W14).

---

## What NOT to do

- **Do not fix a harness by making it green.** If deleting the scrub in W5 turns a run red, that
  is the fix working. Report red honestly; W1 makes it green legitimately.
- **Do not widen an exemption list to pass.** Both harnesses already exempt `undo`, a subcommand
  that no longer exists (`E29`); exemptions are unvalidated. Assert that every exempted name
  exists, and delete the stale one.
- **Do not implement a §0.1 item without a ruling**, however obvious it looks.
- **Do not add a backend.** See W18.
- **Do not trust a green suite as evidence of anything.** 1,359 tests were green while every
  defect in `READINESS` was live. Green is a floor.
- **Do not check state at the wrong moment.** The harness uninstalls each package immediately
  after listing it; inspecting the machine afterwards proves nothing. Two findings in `READINESS`
  were initially wrong for exactly this reason, and the correction is recorded there.

---

## How to report

For each work order: the failing test and its **pre-fix red output**, the fix, the **post-fix
green output**, the sibling sites checked *including those cleared and why*, and the independent
reproduction command.

At the end, a table of `E1`–`E34` with one of: **fixed** (+ reproduction), **not applicable**
(+ why), **deferred** (+ what it needs), **needs a ruling** (+ the question in plain words, with
a recommendation and no jargon — `CLAUDE.md` rule 3).

Then hand the tree to a **different** agent with `docs/GRADER.md` and no sight of your work.
If it cannot independently reproduce your acceptance criteria, you are not done — and that
handoff, not your own green run, is what the grade in `READINESS` §8.1 is measured against.
