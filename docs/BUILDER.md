# YOU ARE THE BUILDER

> **Note on IDs.** This document's work orders are numbered `B1`, `B2`, … They were once
> `W1`–`W43`, which collided with the decision register head-on: `CLAUDE.md` reserves `W*` for
> register IDs you must **stop and ask** about, while a work order here is something to
> **build**. The two series were disjoint in subject and identical in spelling — the register's
> `W9` is *"interpolation outside `when`"*, this document's was *"run the native sweep in CI"* —
> so a builder handed "W9" could not tell from the ID whether to write code or stop.
>
> `docs/archive/PRODUCTION-READINESS-REVIEW.md` hit this first and renumbered itself to `PR*`, with a note
> explaining why. This document did it anyway, forty-three times.
> `tests/id_namespaces_do_not_collide_tests.rs` is now what stops the third one.

**Your job: raise Shall to an A by writing code.** Work through the numbered work orders in this
document, in tier order. Everything you need is here or named here.

**Where the grade stands: C+ when this document was written, B− after round 5.** The per-round
ledgers that recorded the movement — `docs/archive/GRADE-*.md` and the readiness reviews — were
deleted on 2026-08-08 (`Y21`) along with the rest of the record corpus. They are in git, and
every finding they raised was dispositioned into `docs/spec/bugs.md` and the register before they
went. **This document's work orders still stand on their own; its citations to those files are
provenance, not homework.**

You are working in the Shall repo at the path given to you. Start by reading, in this order:

1. `CLAUDE.md` and `docs/SPEC.md` — the repo's binding rules.
2. `docs/spec/bugs.md` — what is known broken and what was carried forward.
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
4. **Verify:** `cargo build --all-targets` → `cargo test --no-fail-fast` → `cargo clippy
   --all-targets --all-features -- -D warnings` → `cargo fmt -- --check`. Unverified is not done.
   The last of those is also a pre-commit hook now — `git config core.hooksPath .githooks`, once
   per clone. Round 8 closed `E3` by declaring a hook out of scope; the next rename proved that
   wrong across nine CI runs, and the owner ruled it in on 2026-08-11.
5. **Commit per work order**, message saying what changed and what it does *not* yet do.

### 0.1 STOP AND ASK — do not implement these unilaterally

`CLAUDE.md` requires a ruling for: anything with a register ID (`D* W* K* N* T* U*`), anything a
user would notice, anything that removes a feature, anything where Part II looks wrong.

Every order below whose heading carries ⚠️ is in this table, and every row names the heading it
belongs to. *(Corrected round 6: three of the four original rows pointed at numbers that had since
moved — "B12 (exit codes)" is `B17`, "B13 (check health severity)" is `B15`, "B16 (supported vs
experimental)" is `B18`. A table of what to stop and ask about, naming the wrong thing, in the
document about checks that name the wrong thing.)*

| order | why it needs a ruling |
|---|---|
| **B1** (manifest withdrawal) | Changes what happens after a failed install. The two harnesses disagree *in writing* about the intent — see `READINESS` §3.1. **Get the ruling, write it into `decisions.md` in the same commit.** |
| **B15** (`check health` severity) | Changes what every user sees on a healthy machine. |
| **B17** (exit codes) | Changes a published contract (`readme.md:708`) that scripts branch on. |
| ~~**B18** (supported vs experimental)~~ | 🚫 **RULED AND REJECTED as `Q4`, 2026-07-27** — three days before round 6 asked for the ruling again. No label exists; the coverage is the work and its absence is a release blocker. **Do not build it.** See the correction at B18. |
| **B22** (refusal exit codes and the hook) | Same published contract as B17, on the security refusals. |
| **B29** (coverage ratchet threshold) | Ruled as `Q12`; the threshold was delegated to the builder. |
| **B31** (`--backend <typo>`) | Changes what a mistyped flag does, which scripts may depend on. |
| **B33** (a bare keyword as a package) | ✅ **Ruled 2026-07-30 as `Q16`** — refuse the bare word; `list:NAME` still means the package. Build it. |

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

### B1 · `E1` — a permanently-failed install wedges the config

**Symptom.** `shall install scoop:<typo>` fails, the line is written to
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
must name `modules/imperative.txt` and suggest `shall unmanage <name>`. A wedge with an exit is
not a wedge.

**Test first.** `tests/` — install a `backend:nonexistent` against a mock backend whose policy
marks the failure permanent; assert the manifest is unchanged **and** that a second command still
parses the model. Then the transient case: assert the line *is* kept.

**Siblings.** `shall add`, `adopt`, `activate` also write manifest lines — `READINESS` §3.1 clears
all three with reasons; re-verify rather than trust. Check `handle_uninstall` still removes
correctly.

**Acceptance.** From a clean config: `shall install scoop:definitely-not-real-xyz123 -y` then
`shall check` — `check` must succeed.

### B2 · `E6` — `go`'s `list` is blind to a package that is installed

**Symptom.** `PASS go installed … for real` / `PASS go: hello is on PATH` / **`FAIL go: list shows
hello`**. Verified against disk: `~/go/bin/hello.exe` exists and is on `PATH`.

**Why it matters most.** `sync` compares desired against `list`. A `list` that cannot see an
installed package produces permanent phantom drift — Shall reinstalls forever and `check` reports
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

**Acceptance.** `shall install go:golang.org/x/example/hello -y && shall list -b go | grep hello`.

### B3 · `E6b` — `nimble` install produces no binary and `list` cannot see it

**Symptom.** Two hard failures: `list shows nimjson` and `nimjson is on PATH`.
`~/.nimble/bin` **is** on `PATH` and was never created.

**Root cause is open — do not guess it.** Investigate whether `nimble install` needs
`--accept`/`-y`, whether it installed only the dependency (`jsony` was present), and whether
nimble on Windows places binaries elsewhere. **Report the diagnosis before the patch.**

**Acceptance.** Full round-trip green in the native sweep, asserted while installed.

### B4 · `E6c` — a successful install the user cannot invoke

**Symptom.** `pub` installs correctly and `list` is correct, but the binary is unreachable:
`~/.pub-cache/bin` is not on `PATH`. Shall reports success and says nothing.

**Fix.** After a successful install, if the backend's bin directory is not on `PATH`, say so —
naming the directory and the shell line to add. One warning, not a refusal.

**Siblings.** Every per-user-bin ecosystem: `pub`, `nimble`, `go`, `cargo`, `gem`, `luarocks`,
`mix`, `stack`, `krew`, `pipx`, `composer`. Implement once on the shared path, not per backend.

**Acceptance.** With `~/.pub-cache/bin` off `PATH`, `shall install pub:sass` warns by name.

---

## Tier 2 — Make the checks capable of failing

Until these land, every later result is unreliable — including your own.

### B5 · `E2` — the assertion that deletes its own evidence

`docker/integration/run-in-container.sh:261-267` and `scripts/integration-windows.sh:269-274`
`grep -v` the line out of the manifest and then assert it is absent. **Delete the scrub. Let the
assertion test the product.** It will go red until B1 lands — that is correct and is the point.

**Siblings.** Audit both harnesses for any mutation (`mv`, `rm`, `sed`, `grep -v`, `unmanage`,
`|| true`) within ~5 lines above an `ok`/`nok`. Then run the mutation experiment in
`GRADER` §2.1: stub `shall` to exit 0 always, and treat **every still-passing check as a
check that tests nothing.**

**Acceptance.** With a do-nothing `shall` stub, both harnesses fail loudly instead of passing.

### B6 · `E5` — the catch-all that launders real defects

Both harnesses soften *any* install failure to
`soft "<backend>: install of <pkg> failed (ecosystem/network variance)"` and **skip that
backend's whole remaining lifecycle**. It fired four times in one run and not once was it network
variance: one was Shall correctly refusing, two were real backend defects (`helm`, `luarocks`).

**Fix — classify instead of assuming.** Shall already exposes the distinction:

| observed | verdict |
|---|---|
| `Retryability::Transient` | `soft`, and retry once |
| `Retryability::Permanent` | **hard fail** |
| exit code 3 / `Error::Refused` | its own outcome — a refusal is not a failure |
| otherwise | hard fail |

**Acceptance.** Point a backend at a package that cannot exist → the harness reports a failure,
not a soft pass. Point it at something Shall should refuse → reported as a refusal.

### B7 · `E3`,`E4` — the ship gate is weaker than CI

`release-check.ps1` and `release-check.sh` rate `cargo fmt --check` *informational*;
`.github/workflows/ci.yml:72` rates it fatal. Make both **hard**. Then run `cargo fmt` (26 diffs
across 10 files today) and diff *every* local gate step against every CI step, reporting
asymmetries in both directions.

### B8 · `E26` — 10 commits CI has never seen

5,082 inserted lines including a 711-line executor rewrite, compiled and tested on Windows only.
After B7, **push and get a verdict.** The last time this repo carried a large unpushed backlog,
the first CI run that saw it failed on all three platforms on two distinct bugs.

### B9 — run the native sweep in CI

Every hard failure in this assessment came from `release-check.ps1`'s section 12, and **no
automated gate runs it on any platform.** Add a Windows job. Keep it off the per-push path if it
is slow, but it must run nightly and be consulted.

---

## Tier 3 — Backend correctness

### B10 · `E7`,`E8`,`E9` — scoop's exit code is always 0

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
produce one real failure outside the existing markers and assert Shall reports it. Also fix the
message: `` `scoop` failed (exit 0) `` is incoherent — when the verdict comes from output rather
than status, say so.

**Acceptance.** `shall uninstall scoop:<not-installed>` reports failure, not success.

### B11 · `E10` — `psresource` reports healthy and cannot run

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

### B12 · `E11`,`E12`,`E13` — three argv defects, and the gate that would have caught all three

| id | defect | fix |
|---|---|---|
| `E13` | `pixi global upgrade-all` was **removed upstream**; pixi 0.73 says use `global update` | `src/backends/registry.rs:1381` |
| `E11` | `helm` install fails: `plugin source does not support verification. Use --verify=false` | helm backend argv |
| `E12` | `luarocks` fails: `No results matching query were found for Lua 5.5` — no version pinned | luarocks backend argv |

**Do not stop at the three.** Build the **argv-drift gate** (`GRADER` §3.2): for every
manager installed on the runner, assert that every subcommand Shall will invoke still appears in
that manager's own help output. This is the single highest-leverage test in the programme — it
converts silent upstream drift into a named failure, and it is the difference between fixing
`pixi` today and fixing its successor automatically.

### B13 · `E16`,`E17` — two search parsers emit junk

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

### B19 · `E18`,`E19` — one condition, two message families, both naming the wrong program

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

**Acceptance.** `shall check health` on a machine missing several managers emits one message
shape, and each names the program it looked for.

---

## Tier 4 — The human path

### B14 · `E14`,`E15` — `info` takes 98s to return a wrong answer

`shall info cargo:ripgrep` → `Package 'cargo:ripgrep' not found in any available backend.` in
**1m37s**, while `shall search ripgrep` in the same tree returns `cargo ripgrep 15.2.0`. Two
commands in one program contradict each other and the wrong one is the slow one. The explicit
`cargo:` qualifier appears not to narrow the probe — start there.

**Then add latency budgets as a gate**: read-only commands under 2s, a qualified `info` under 5s.
Nothing measures latency today, which is how a 98-second `info` shipped.

### B15 · `E20`,`E21` — "23 critical" on a healthy machine ⚠️ needs a ruling (0.1)

`check health` opens with `Backends: 25 OK, 0 degraded, 23 critical (of 48 total)` on an ordinary
Windows box. Nothing is wrong — those are managers the user has not installed. Meanwhile the
`check` rollup says `ok health 25 backend(s) ready`. **The rollup and the detail view disagree
about the same machine.** Propose: *absent* as its own state, distinct from *critical*.

### B16 · `E27`,`E28`,`E30`,`E22`,`E23`,`E12` — the first hour

- First-run `shall sync` explains the `priority` format by hand and **never mentions `shall init`**
  (`E27`). It is the first command a new user runs.
- `shall init --help` promises "a starter module"; `modules/` is created empty (`E28`).
- One failure prints **three times** — `WARN shall::core::journal` with a 32-hex WAL id, `ERROR
  shall::core::transaction` naming a "Node", then `Error:` (`E30`). Print it once, in the user's
  words. Audit for leaked internals: `WAL`, `Node`, `DAG`, module paths, UUIDs.
- `purge-unmanaged` (`src/verbs/cleanup.rs:262`) and the auto-remediation confirm
  (`src/app/diagnostics.rs:147`) lack the `is_terminal` refusal their siblings have — measured,
  they fail *safe* with a bare `IO error: not a terminal` instead of the actionable message
  (`E22`,`E23`). Use the pattern at `src/verbs/cleanup.rs:338`.

**The rule worth adopting:** every user-visible failure names (a) what failed in the user's words,
(b) the file or command to act on, (c) exactly one place to look. Then test for it.

### B17 · `E24`,`E25` — the published exit-code contract is violated ⚠️ needs a ruling (0.1)

`readme.md:708` publishes four codes and says *"a script can branch on them"*. Measured:
`shall nosuchcommand`, `shall --nosuchflag` and `shall sync --badflag` all exit **2** — which the
table defines as *"a read-only command looked and found work to do"*. Clap's usage-error
convention is 2 and it exits before Shall's `finish()` mapping runs. **A CI job following the
documented table reads a typo as drift**, which defeats the stated purpose of code 2.

Separately, a `purge-unmanaged` ratio refusal exits **1**, not the documented **3** — it is raised
with `anyhow::bail!` rather than `Error::Refused`. Audit every refusal for the same slip; the
native harness asserts these with `nok`, which accepts any non-zero code and cannot tell 1 from 3.

### B20 · `E29`,`E31`,`E32` — documentation that has drifted from the register

Small, but this repo's stated thesis is that undetected drift is what cost it 84 unanswerable
decisions. Each of these is a one-line fix plus a check that stops it recurring.

- **`E31`** — `docs/SPEC.md:16` says *"All 107 decisions. 105 answered, 2 parked"*;
  `docs/spec/decisions.md:64` says *"All 104 are ruled: 102 ANSWERED, 2 PARKED, 0 OPEN."* The
  register is the authority; the map is stale. **Then make the count generated, not typed** — two
  files that both track one number will disagree again.
- **`E32`** — `docs/archive/PRODUCTION-READINESS-REVIEW.md` uses `U1`/`U2`/`U3` for its own findings, but
  `CLAUDE.md` reserves `U*` for register IDs requiring an owner ruling, and the register's real
  `U1` is *"where does a custom backend definition live"* (ruled 2026-07-23). Rename that
  document's labels. It costs nothing and removes a live trap for exactly the agent reading these
  briefs.
- **`E29`** — both harnesses exempt `undo` from the coverage audit; `shall undo` does not exist
  (renamed to `snapshot`/`rollback`). Delete it, and **assert that every exempted name appears in
  `--help`** — `harness-logic-test.sh` already checks that every *invoked* subcommand exists and
  should do the same for every *exempted* one. That asymmetry is why this survived.

---

## Tier 5 — The structural change (this is the one that matters)

### B18 · `E33` and the whole of §5 — redefine "supported" 🚫 **RULED AND REJECTED — DO NOT BUILD**

> **Corrected 2026-07-30 by the round-6 builder.** This order asks for a ruling it already had,
> and the ruling was **no**. `Q4`, ruled by the owner on **2026-07-27**, three days before this
> document's round-6 section repeated the request: *"Are unverified backends labelled
> experimental? **NO.** They are tested instead, and nothing ships until they are."*
>
> The owner's reason is a rule about the project, and it is binding: *this codebase does things;
> it does not cover for not doing them.* A label converts an unfinished job into a permanent
> disclaimer, and a disclaimer nobody has to retire is one nobody does. The gap would be
> *documented*, which reads like *handled*, and the untested backends would still be untested a
> year later with a caption explaining why that is fine (`V.93`).
>
> **What the ruling binds, all of it:** no `experimental` or `supported` label anywhere — not in
> `check health`, not in `priority`, not in the readme, nothing to grep for. **`shall init` keeps
> scaffolding every manager it finds**, because scaffolding fewer is the same disclaimer written
> as a default. A backend with no real lifecycle in an automated gate is a **release blocker**,
> not a caption. No new backend until the current set passes.
>
> So items 1–4 below are superseded: **only item 4 survives**, and item 1's round-trip is the
> work rather than the label. The three sentences elsewhere in this document calling B18 "the
> highest-value change" and "still untouched" describe a change that must not be made. The
> register is the authority and the map was stale — which is `E31`'s finding, on the document
> that reported it.

Everything above fixes instances. ~~**This fixes the generator.**~~ *(What follows is the
original text, kept because the measurement in it is still true and still the argument for
doing the coverage work. The proposed remedy is what was rejected.)*

Measured: `check health` registers 48 backends on Windows, 56 on Ubuntu; the Ubuntu lifecycle
reports **7 real lifecycles against 49 plan-smokes**. A plan-smoke proves an argv was
*constructed*, never that it is *correct* — `pixi global upgrade-all` passes one and does not
exist. Every defect in this assessment lives in that remainder.

**Do this:**

1. A backend is **supported** only when it has passed a real install → `list` → binary → remove
   round-trip, asserted while installed, in an automated gate.
2. Everything else is **experimental**, and says so — in `check health`, in the `priority` file
   `init` writes, and in the readme.
3. `shall init` currently writes **23 managers** into `priority` on a fresh Windows box, most
   never run. Scaffold only supported ones by default.
4. **Stop adding backends until (1) is true for the current set.**

This converts an unbounded invisible-bug surface into an honest bounded claim. It is the only
item in this document that stops §5.1 from regenerating.

---

## Tier 6 — What A+ additionally requires

Tiers 1–5 reach **A**. A+ needs test *kinds* this repo does not have. Specified in full in
`docs/GRADER.md` §4–§6; summarised here so the bar is in one place:

- **Argv-drift gate** (B12) running nightly against every installed manager.
- **pty coverage on every read command**, not just `list`; all four stdin/stderr handle
  combinations for mutations; real `sudo` with a password in a container.
- **Destructive effectors in disposable VMs** — btrfs/zfs/lvm restore on loopback filesystems,
  `dpkg -i`/`rpm -U`, U30 storage removal. **U30 storage removal is no longer in this list as of
  2026-08-18**: the `storage` leg destroys a btrfs subvolume, an LVM volume and a ZFS dataset
  through Shall on every run and asserts each is gone. Still unrun: the two restores, the
  local-file handoff, and a **protected** storage object refused by the guard in front of a real
  destroy — which has no argv to test, so a live run is its only possible proof.
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
- **Latency budgets** enforced per command class (B14).

---

# ROUND 2 — added by the Grader, 2026-07-28

Round 1 landed: **27 of the 34 defects are closed and independently reproduced**. The full
disposition, the coverage ledger and the evidence are in `docs/archive/GRADE-2026-07-28.md`. Read that
before starting; the numbered findings below are `G-n` in it.

**Grade: B−**, up from C+. The rubric's **B** bar is not met on two of its four clauses.

**The failing tests already exist.** Round 1 asked you to write the test first; for most of what
follows that is done — `tests/grader_*.rs` are committed and **red**, each watched fail, each
carrying a control where a control was needed. Your job is to make them pass *by fixing the
cause*, and to extend them to siblings they do not yet reach. Do not delete a red test to make a
tier green.

**The one-line diagnosis of this round.** Every finding below is a sentence that quantifies over
paths and was never re-derived from the code:

| the sentence | where | reality |
|---|---|---|
| "every path that removes anything goes through one guard" | `readme.md:266` | 10 sites, **9** guarded |
| "the one point every refusal in the program passes through" | `src/main.rs:185` | 23 sites, **15** conforming |
| "`--dry-run` performs nothing" | the flag's whole purpose | 5 verbs still act |

`READINESS` §5.3 named this exact class. It is not a new disease; it is the same one, one layer in.

---

## Tier 1 (round 2) — the safety model is incomplete

### B21 · `G-1` — a removal path with no guard, no count, and no plan line

**Symptom.** With `[guard] max_removals = 1` and `protected_packages = ["f3"]` — confirmed
effective by `shall protected` — undeclaring five `link:` lines and running `shall sync -y`
deleted all five target files **including the protected one**, exited 0, and printed
`already up to date`. `shall --dry-run sync` printed `already up to date` as well.

**Root cause (verified).** `src/app/apply/extras.rs:105` calls `inst.remove(...)` directly for
`service`/`link`/`setting` (and the sibling arms handle `shim`, `schedule`, `repo`). The word
`guard` does not appear in that file. The teardown runs outside the transaction, so it is not
covered by the `guard::enforce` that `sync/mod.rs:141` applies to the package plan.

The `dry_run` check at `extras.rs:66` **is** correct — a preview genuinely performs nothing. The
defect is that neither the preview nor the real run *reports*, and that nothing is guarded.

**Fix.** Give extras teardown its own `GuardScope`, route it through `guard::enforce`, and fold
the count into the plan so `max_removals` sees it and `--dry-run` names it. A `link:` whose target
is a decrypted secret or a live `service:` deserves the same refusal a package gets.

**⚠️ Needs a ruling (0.1)** — *only* on the scope question, in plain words for the owner:

> Today, deleting a `link:`/`service:`/`setting:` line makes `sync` remove that resource without
> counting it, without listing it, and without checking your protected list. Should those
> resources be protected by the same `[guard]` rules as packages — meaning a big teardown gets
> refused the same way a big uninstall does — or should the guard stay packages-only and these
> just be *reported* before they happen? Recommendation: **guard them the same way.** The blast
> radius is a user's dotfiles and running services, which is not smaller than a package.

**Test first.** Already red: `tests/grader_extras_guard_tests.rs` (2 tests — one for the guard,
one for visibility).

**Siblings.** All six extra kinds, not just `link:` — `shim_manager().remove_shim`,
`scheduler.deprovision`, the `repo` arm, and `service`/`setting`. I verified the other **nine**
backend-removal call sites *are* guarded and named each in `GRADE` §3 G-1; re-verify rather than
trust me.

**Acceptance.** The reproduction in `GRADE` §3 G-1, run from a clean config: `sync` must refuse,
or at minimum list the five removals and count them against `max_removals`, and must never delete
a protected name.

### B22 · `G-10` — the security refusals exit 1, and the refusal hook never fires ⚠️ needs a ruling (0.1)

**Symptom.**

```
$ shall install 'web:http://example.com/tool.tar.gz' -y
Error: Validation error: refusing to download … over plain HTTP
EXIT=1                                    # readme.md:708 says 3 means refused
$ shall reset </dev/null
EXIT=3                                    # correct, same binary
```

**Root cause (verified).** Eight sites whose own message says *"refusing to…"* are not built as
`Error::Refused`, so `main.rs:183` never sees them:

| site | refuses | rule |
|---|---|---|
| `core/download.rs:46` | plain HTTP | SEC2 |
| `core/download.rs:69` | unverified, no `@sha256` | SEC2 |
| `core/executor.rs:396` | a secret nothing protects | T5 |
| `backends/link.rs:68` | decrypt into the git repo | T2 |
| `app/hooks.rs:55` | unapproved hooks | II.12 |
| `app/shim_manager.rs:98`, `utils/file.rs:174` | deploy over a foreign file | SEC1 |
| `app/apply/dotfiles.rs:67` | files outside `$HOME` | SEC3 |

**The refusals that behave correctly are the ones about removing packages. The ones that do not
are the entire SEC/T series.**

**The exit code is the lesser half.** `src/main.rs:185` states that the `Error::Refused` arm is
*"the one point every refusal in the program passes through, so no command can be added that
refuses without the hook hearing about it."* That is false for all eight: `on_guard_refusal` never
fires for an unverified download, an unprotected secret, or an unapproved hook. **Delete the
comment or make it true** — a comment asserting something about paths it never enumerated is the
mistake `spec/history.md` calls the costliest in this repo.

**It also un-blinds the harness.** `classify_install` keys `refused` on rc=3, so Shall refusing
correctly (`github:sharkdp/fd`, target exists and Shall did not create it) arrives as rc=1 and is
scored *"a defect, not ecosystem variance"*. `READINESS` §3.4 complained a correct refusal was
laundered into a soft pass; it is now laundered into a **false hard failure**. Fixing this fixes
the harness for free — do not patch `classify_install` instead.

**⚠️ Needs a ruling (0.1)** — same reason B12/B17 did: it changes a published contract.

> `readme.md` promises exit 3 means "Shall refused on purpose". Right now that is true when it
> refuses to remove too many packages, and false when it refuses to download over plain HTTP or
> to write a secret nothing protects — those exit 1, the same code as a crash. Should all of them
> return 3? Recommendation: **yes.** Scripts branch on this, and "I refused" and "I broke" are the
> two answers that must never be confused.

**Test first.** Already red: `tests/grader_refusal_exit_code_tests.rs` (2 — one behavioural with
`reset` as control, one enumerating every "refusing to…" site from the code).

**Siblings.** The five sites my scan could not classify — `snapshot_restore.rs:139`,
`firewall.rs:168`, `health.rs:126`, `rehearsal.rs:47`, `rehearsal.rs:68`. Classify each and say
which way it went.

**Acceptance.** Every command whose output contains "refusing" exits 3, and
`on_guard_refusal` fires for each. The enumerating test goes green without its floor being lowered.

---

## Tier 2 (round 2) — `--dry-run` still acts

### B23 · `G-2` — five verbs perform their action during a preview

**Symptom.** Measured on a fresh config, each against a control that runs the same command without
the flag:

| command | what the preview did |
|---|---|
| `--dry-run activate <p>` | switched the active profile · **printed nothing** |
| `--dry-run deactivate <p>` | emptied `active` · **printed nothing** |
| `--dry-run lock` | wrote `locks/versions.json` + `locks/hooks.toml` |
| `--dry-run git init` | created the repo **and committed** |
| `--dry-run config init` | wrote `preferences.toml` |

**Root cause.** The flag is consulted per-verb. Round 1's W-order fixed `uninstall`, `unmanage`,
`module create` and `schedule add`; these five were never enumerated. `activate` guards and
`deactivate`, its twin four lines away, does not — the S6 shape `spec/history.md` already records.

**`activate`/`deactivate` are the serious pair**: they decide which modules are in the model, so
they decide what the next `sync` installs and removes.

**Fix.** Stop consulting the flag per-verb. Put the check where the *write* happens — one place
that every verb's config mutation passes through — so a verb added tomorrow cannot forget. That is
the only version of this fix that stops the class.

**Test first.** Already red: `tests/grader_dry_run_siblings_tests.rs` (5, each with a control).

**Siblings.** Enumerate every config-mutating verb from the code, not from this table. My probe
covered 13 of the 61 subcommands; the other 48 are unexamined.

**Acceptance.** For every mutating verb: snapshot the config dir, run under `--dry-run`, snapshot
again, assert byte-identical — and assert the control *did* change it.

---

## Tier 3 (round 2) — argv, and the gate that does not cover it

### B24 · `E11` reopened + `G-8` — the argv-drift gate checks subcommands, not flags

**Symptom.** `E11` is **not closed**. On this Windows host it failed with `plugin already exists`
(residue from an earlier run) which masked the real argv; in a **fresh** container:

```
Error: `helm` failed (exit 1): Error: unknown flag: --verify
```

**Root cause (verified).** `capability.rs:34` — `VERIFIES_ITSELF = [("helm", "--verify=false")]` —
is emitted unconditionally for `@unverified`. This box runs helm **v4.2.3**, which has the flag;
the container runs helm 3, which does not. The flag came from helm 4's own error text. So
`@unverified` works on helm 4 and breaks every helm 3.

**The gate that exists for this cannot see it.** `tests/argv_drift_tests.rs:103` says in its own
words: *"A token that is a subcommand rather than a flag"*. It verifies **72 subcommands** against
live tools and **zero flags**. The first flag added after the gate shipped is one a real manager
rejects, and the gate was green throughout.

**Fix.** Two parts, and the second is the one that matters. (a) Make the helm flag conditional on
what the installed helm accepts. (b) **Extend the drift gate to flags and operands** — for each
manager, assert every flag Shall may pass appears in that manager's help. This is the highest-value
item in Round 2 for the same reason B12's gate was in Round 1.

**Test first.** A fixture from helm 3's `plugin install --help` (no `--verify`) and one from
helm 4's (with it); assert Shall builds a different argv against each.

**Siblings.** Every conditional flag Shall emits from a capability table, not just `VERIFIES_ITSELF`.

**Acceptance.** `shall install 'helm:secrets@url=…,unverified'` succeeds in the `tools` container.

### B25 · `G-9` — four backends fail a real install, found the first time anyone ran them

From the `tools` image, which I built and ran (**324 pass, 16 fail**). Each failed **twice**, so
the classifier correctly called them defects, not variance:

| backend | error | reading |
|---|---|---|
| `asdf` | `No such plugin: --` | a bare `--` reaches the plugin-name position |
| `mix` | `Could not find an SCM for dependency :hex` | wrong verb; `hex` installs via `mix local.hex` |
| `spack` | `Spec ~~zlib has no name` | the version-pin syntax is doubling a `~` |
| `opam` | `No switch is currently set` (exit 50) | needs an initialised switch; nothing sets one or says so |

Three of four are malformed argv — the same family as B24. `nimble` additionally installs and
lists correctly while its binary never reaches `PATH` (B4's family, in the container).

**Acceptance.** The `tools` image reaches `fail=0`, or each remaining failure is named in
`decisions.md` as an environment limitation with a reason.

### B26 · `E7`,`E9` still open — scoop's exit code is still lost

B10 did not land. Measured, same failing command down each branch on this host:

```
cmd /C …\scoop.cmd install <bad>   -> exit 0     # the branch Shall takes
powershell -Command "…; exit $LASTEXITCODE"  -> exit 1     # the branch it never reaches
```

`which::which("scoop")` returns `scoop.cmd` because the default `PATHEXT` has no `.PS1`, so
`windows_effective_command` takes the `cmd /C` arm and the careful `.ps1` arm twenty lines above
is dead code. Every scoop verdict rests on `ExitPolicy` string-matching stdout.

**Prefer the `.ps1` when one sits beside the resolved shim.** Do **not** assume every `.cmd` is
lossy — measured, `npm.cmd` propagates its exit code correctly, and calling it lossy would be a
finding manufactured from a file suffix.

**Test first.** Already red: `tests/grader_shim_exit_code_tests.rs` (2; the second *measures* both
branches rather than inferring from the extension).

### B27 · `E12` / `G-3` — `Transient` is a claim nothing tests

`luarocks install luafilesystem` fails identically three times in a row while
`curl https://luarocks.org/manifest-5.5` returns **200** — luarocks' own downloader is what fails,
because the `wget` first on PATH is a scoop shim. `exit_policy::luarocks()` lists
`"failed downloading"` as transient, so Shall keeps the declaration and tells the user `sync` will
try it again. It will fail identically forever.

The policy's doc comment **names this exact cause** and classifies it as the network anyway.

**Fix.** The container harness already knows how to answer this: it retries once and calls a
repeat a defect. The product asserts the same thing from a string and never tests it. Either make
`Transient` empirical where it is cheap to be, or stop telling the user a retry will help.

**Test first.** Already red: `tests/grader_transient_claim_tests.rs`.

---

## Tier 4 (round 2) — checkers that examine the wrong thing

### B28 · `G-4` — gate parity compares basenames, not gates

`harness-logic-test.sh:291` greps `ci.yml` for `scripts/*.sh` and asserts each **basename** appears
in both release scripts. CI runs `harness-mutation-test.sh` **twice** — once bare (Windows
harness) and once against `run-in-container.sh`, which then carried `SURVIVOR_BUDGET=92`. Both
release scripts run it once. Parity passes because the string appears.

Compounding: the container harness's budget lived only in `ci.yml`, so the script's own documented
invocation failed on a clean tree (90 survivors against a default of 86). Both halves are fixed —
the thresholds are per-harness inside the script, and `ci.yml` overriding any of them is now
itself a test failure.

**Fix.** Compare invocations, not names. Move each harness's budget beside the harness.

**Test first.** Already red: `tests/grader_gate_parity_tests.rs` (2).

### B29 · `G-11` — the coverage audit has a floor for an empty registry and none for collapsed coverage ⚠️ needs a ruling (0.1)

The clean Windows sweep reports `backends: 4 real lifecycle, 12 install-attempted, 44 plan-smoked`
and `PASS every registered backend got a lifecycle or a plan-smoke`. Four — not because anything
broke, but because **8 of 15 canaries were already installed on this host** and the harness
correctly refuses to remove software the user already had.

So **the gate's coverage is inversely proportional to how much the host is used**, and a run with
4 and a run with 15 are both `PASS`. G2 gave this audit a floor for an empty *registry*; it has
none for collapsed *lifecycles*.

**⚠️ Ruling needed** because the threshold is a judgement, not a fact:

> On a developer's own machine the sweep really tests about a quarter of what a clean CI runner
> does, because it skips anything already installed — and it still says PASS. Should it fail when
> real lifecycles drop below some number, and what number? Recommendation: **a ratchet, like the
> mutation budget** — record the count per host class, fail when it falls, never when it rises.

**No red test from me**, deliberately: picking the threshold is the owner's call, and a test that
encodes my guess would be the wrong kind of check.

### B30 · `G-5` — `scripts/grader-red-tests.sh` is a gate nobody runs that cannot go green

131 lines, run by **no** CI job and **neither** release script. Two problems: almost every check is
a **grep over source text** rather than a behaviour (`G6` passes if the word `is_terminal` appears
anywhere in a file), and **`G1` can never pass** — it reproduces the buggy `grep -c … || echo 0`
idiom inline in the test itself, so the file always exits 1. Its `E12` check is also stale against
the ruling that superseded it.

**Fix.** Delete it, or make its checks behavioural and wire it into the release scripts. A
permanently-red, un-run file of source greps is worse than nothing: it is the shape of check this
whole effort exists to remove.

### B31 · `G-7` — `list --backend <typo>` succeeds silently ⚠️ needs a ruling (0.1)

`install` refuses an unknown backend with an excellent message naming the file and the fix. `list`
answers the same question with exit 0 and no output — for `nosuchbackend`, `aptt`, `APT`, and the
empty string. That is byte-identical to a real backend with nothing installed.

It also **disarms the rubric's own A-bar check**: "every `[READY]` backend can answer `list`" passes
for all 24 on this host, but only **11** returned rows; the other 13 cannot be told from a typo. I
recorded 24/24 before testing my own oracle, and corrected it.

**⚠️ Ruling needed** — user-visible behaviour change:

> `shall list -b aptt` currently prints nothing and succeeds, which reads as "that manager has
> nothing installed" rather than "there is no such manager". Should it refuse the way `install`
> does? Recommendation: **yes**, copying `install`'s message verbatim.

**Test first.** Already red: `tests/grader_unknown_backend_tests.rs`.

### B32 · `E26` — still 10 commits CI has never seen

`origin/main` is at `213973a`; HEAD is 10 commits and 1,978 inserted lines ahead, including a
350-line Windows-cron rewrite. I verified the tree **compiles** on Linux (the ubuntu image build
runs `cargo build --release` and succeeded); `cargo test` on Linux and anything on macOS remain
unverified. B8 asked for this in Round 1. Push it.

---

## What Round 2 does *not* change

`E15` (search at 145 s) and `E33` (psresource's truncated error, untestable without PSResourceGet
installed) are unchanged and unaddressed; B14's latency budgets are still the item that would
move the grade most. *(Corrected 2026-07-30: the other one named here, B18's
supported/experimental split, was ruled and REJECTED as `Q4` on 2026-07-27 — see B18.)* Nothing above supersedes Tier 5 or
Tier 6 — they remain the path from B to A.

---

# Round 6 — the work orders from the round-5 grade

Source: `docs/archive/GRADE-2026-07-30-round-5.md`, graded at `0cdeca2`. **Every round-4 finding
(`B-1`, `P-1`–`P-4`) is closed and verified by its original reproduction on two platforms — do
not re-open them.** What follows is new, and the red tests for most of it are already committed
and failing.

Read the grade document's §3 before starting: each order below is a summary of an entry there
that carries the measured output, and the measurement is the specification.

**Two of these are old orders, unfinished.** B35 is the half of `B27` that was never built, and
B36 extends `B17`'s audit from refusals to failures. Neither is a regression; both are a family
that was fixed at the reported instance and left live one layer over.

---

## Tier 1 (round 6) — a typo installs software

### B33 · `R-1` / **`Q16`** — a bare grammar keyword is a package name ✅ **ruled 2026-07-30**

A module containing the single word `link` — which is what a half-typed
`link:SRC @target=DEST` line looks like — declares a package. `shall eval` reports
`{"backend": "cargo", "name": "link"}`, `--dry-run sync` plans one install, and `shall check`
says *"1 to install … run `shall sync`"*. Thirteen of fourteen keywords do this, and each
resolves to a **real** backend holding a real package of that name: `when`→`cargo:when`,
`absent`→`pip:absent`, `shim`→`scoop:shim`, `if`→`gem:if`, `else`→`npm:else`. Only `use` refuses.

**This is not a parser defect.** A package name is one bare word (II.2), so a bare keyword is a
valid package line, and with their punctuation the same words refuse correctly and legibly
(`link:`, `when linux {` all exit 1 with a located error). It is an ambiguity in the language.

**RULED (owner, 2026-07-30): refuse the bare word, and keep every package reachable.** A line
containing only a keyword is a parse error naming both ways to mean it:

```
modules/dev.txt:4: `link` is a keyword, not a package name
  to link a file:                      link:/path/to/source @target=…
  to install a package by that name:   list:link   (or pin one: cargo:link)
```

**No new syntax, and no quoting.** A bare `NAME` is already defined in II.2 as short for
`list:NAME`, so `list:link` means precisely what the bare form used to — the escape hatch the
owner asked for already existed. Quoting was considered and rejected: **V.10** rejected it because
`"` needs `\"` needs `\` needs a newline rule, and nothing here disturbs that. The ruling adds
a refusal and removes nothing.

It binds the bare **word**, not the prefix — `link:` with its colon and nothing after it was
already a legible refusal and stays exactly as it is. The rule is in **II.2**, the reason in
**V.103**.

**Test first.** Already red: `tests/grade4_keyword_is_not_a_package_tests.rs` (2 red, 1 green
control). It reads `known_prefixes()` rather than a copied list, so a prefix added later is
covered without anyone remembering.

**Siblings.** The seven near-miss forms with punctuation were measured and all refuse correctly —
do not touch them. Not measured: a keyword as the *name half* of a qualified line (`cargo:link`),
which is presumably fine and intended.

**Also fix, and it needs no ruling:** resolving one of these costs **10–27 seconds**, because a
bare name has no backend and the resolver asks every manager in priority order. The same fixture
with `cargo:ripgrep` is 0.2s. Whatever `Q16` rules, a name that no backend claims should not cost
half a minute to find that out. See B41.

### B34 · `R-2` — `adopt` re-declares what the manifest already names, and mislabels its own count

One package declared and installed, and `adopt` writes a second declaration for it in
`adopted.txt` — so *"Deleting a line UNINSTALLS that package on the next sync"*, printed two
lines below, is false for exactly the packages the user wrote by hand. Measured: delete your own
line, and `--dry-run sync` says `already up to date`.

**Root cause, and it is two things with one root.** `discover()` (`app/adopt.rs:117`) keeps a
candidate when `!state_guard.is_managed(&pkg.backend, &pkg.name)` — the managed-state *registry*.
**Nothing in `discover` reads the manifests at all.** And `found.skipped` has exactly two push
sites, `:154` (the OS reports it essential) and `:315` (`hold_back_what_cannot_be_written`), while
the summary at `:281` prints `found.skipped.len()` under *"(listed in the manifest)"* — a reason
that is wrong for 100% of the items, always. A one-line manifest produced *"Left alone: 185
(listed in the manifest)"*.

**Fix.** Subtract what the resolved model already declares, not only what the registry manages;
and render the summary from the per-item `reason` each `Skipped` already carries, rather than
collapsing three causes into one sentence that names none of them.

**Test first.** Already red: `tests/grade4_adopt_respects_the_manifest_tests.rs` (2). It asks
Shall itself what this host can adopt, so it names a real package rather than guessing, and a
host with nothing to adopt is **skipped and named** rather than passed.

**Siblings.** Every other rollup count that explains itself with a reason belonging to one of its
inputs. One more is known: `--dry-run unhold` prints *"would release 1 hold(s)"* and then
*"0 hold(s) were not recorded"* about the same hold.

---

## Tier 2 (round 6) — the classification is computed and then not consulted

### B35 · `R-3` — a `Transient` failure is reported as "Nothing classified the failure above"
#### *(this is the unbuilt half of `B27`)*

`error.rs:226` classifies a rate limit `Transient` and says why: *"The whole point of a rate limit
is that the window moves."* `why_kept` (`verbs/packages.rs:274`) branches on `Refused`,
`Exhausted` and name-absence, then falls through to `Unclassified` — **there is no `Transient`
branch**, and `WhyKept` has no variant meaning *"known to be temporary, and here is the window"*.
Observed live on the macOS runner, with the window printed one line above the advice:

```
Error: API rate limit: … does not reset for 1236s, past the 30s ceiling. …
 WARN … Nothing classified the failure above, so if it repeats unchanged the cause is not a
      passing one …
```

The advice inverts the truth: a rate limit repeats unchanged *because* it is passing. B27 asked
you to *"stop telling the user a retry will help"* when it will not; the same sentence needs to
stop telling them nothing looked, when something did.

**Fix.** A `WhyKept::Transient` carrying what the error already knows. The wording for
`Exhausted` is the model to follow — it is precise about what was tried.

**Test first.** Already red: `verbs::packages::tests::a_transient_failure_is_not_reported_as_unclassified`
and `…_is_not_advised_as_if_it_were_permanent` (`cargo test --bin shall`). Its sibling,
`a_transient_or_unclassified_failure_keeps_the_line`, stays green — the line must still be kept.

**And fix the harness half, which is costing two red CI jobs.** `classify_install` in
`scripts/integration-windows.sh` tests transience by retrying the install *immediately*, which
cannot succeed inside a 1236-second window — so it scored `defect`, the macOS leg went red, and
the real-lifecycle ratchet fell 8 → 7 and went red behind it. `GRADER` §2.2 asked for this to be
driven off Shall's own `Retryability`; Shall computes it correctly and the harness cannot see it.
**Surface the classification** (a machine-readable line, or a distinct exit code) and have the
harness read it, rather than guessing from a repeat.

**Do not** fix the red ratchet by lowering `windows-native-darwin-ci` to 7. That would ratchet
macOS coverage down over a rate limit, permanently. It is the one edit `scripts/lifecycle-floor.txt`
exists to make visible in a diff.

**Siblings.** Both production consumers of `retryability()` were enumerated: `transaction.rs:551`
(`give_up = … == Permanent`) is correct and needs nothing. B36 is the third place that has the
classification and ignores it.

### B36 · `R-6` — `heal` reports an unrecovered operation at rc=0, in Rust `Debug` syntax
#### *(this extends `B17`'s audit from refusals to failures)*

**The behaviour underneath is right and must not change**: given an `InProgress` install for a
package that does not exist, `heal` attempts the recovery, fails, and **leaves the entry
`InProgress`** rather than closing it, and `list` does not claim the phantom. That is the answer a
"mark everything done" implementation gets wrong. Three defects in how it says so:

```
ERROR could not recover npm:… — Some(CommandFailed { message: "…404…", retry: Permanent,
absent_name: true }). … re-run `shall sync`.
 WARN 1 operation(s) could NOT be recovered: npm:… . Re-run `shall sync`.
heal: reconciled locks/versions.json (1 entries)
heal: refreshed backend metadata
heal rc=0
```

1. **rc=0** after *"1 operation(s) could NOT be recovered"*. `shall heal && echo ok` prints ok.
   B17 audited *refusals* for the exit-code contract; failures were not in scope and are not
   covered.
2. **`{:?}` on an `Option<Error>` printed at the user** — `Some(CommandFailed { … })`,
   `retry: Permanent`, `absent_name: true`. `absent_name` is an internal field the N-1 fix added
   this month. `GRADER` §4: flag every place internal vocabulary leaks.
3. **The advice contradicts the struct it just printed.** `absent_name: true, retry: Permanent`
   means the name does not exist, and `packages.rs` has a whole `WhyKept::NameAbsentElsewhere`
   branch whose wording is *"`sync` will keep failing the same way until the line naming it is
   corrected"*. `heal` says *"re-run `shall sync`"*.

The last two lines a user sees are successes.

**Test first — not yet written, and here is why**, because the difficulty is the useful part:
planting a WAL entry whose recovery fails needs either a network round-trip or a backend present
on every runner, and a hand-written `Install` entry that omits `options` lands in the
**corrupt-WAL branch** instead of the recovery branch. Build the entry from a real one (run a
sync, then edit the journal in place) rather than by hand.

**Siblings.** Every recovery path that can partly fail: `heal`, rollback compensation, the lease
sweep, the shell-exit teardown. Check each for an exit code that ignores its own failures.

---

## Tier 3 (round 6) — messages, and one platform defect

### B37 · `R-4` — `shall list` and `shall info` contradict each other on macOS

`tests/grade2_info_tests.rs` is red on `macos-latest`: `list` reports
`service:com.apple.SafariHistoryServiceAgent` and `info` about that exact name answers *"is not
installed on this machine, so there is nothing to describe."* The `service` backend enumerates
launchd agents that `info` cannot then resolve.

The rubric weights this heavily: *a `list` that disagrees with the machine breaks the one thing it
promises*. **Not reproduced by the grader** — it has no Mac; this is CI's result. Reproduce it on
a Mac or a runner before fixing, because the round-3 lesson about checking state at the wrong
moment applies to a listing that is a snapshot of a daemon table.

**Test first.** Already red and already correct — `grade2_info_tests` takes the first thing `list`
reports and asks `info` about it. Do not narrow it to skip services.

**Siblings.** Every backend whose `list` enumerates something its `info` resolves differently.
`service` on Linux (systemd units) is the obvious twin and was not measured.

### B38 · `R-5` — a refusal that names no file, about a character nobody can see, echoed raw

Two error classes over one 60-line module with one bad line at 40:

```
cargo:<U+202E>reversed  ->  Validation error: Invalid characters in package name: …    (no location)
cargo:<ESC>[31mred…     ->  Validation error: Invalid characters in package name: …    (no location)
cargo:aaa…(300 chars)   ->  Configuration error: …/big.txt:40: …                       located
cargo:rip<TAB>grep      ->  Configuration error: …/big.txt:40: …                       located
```

The grammar's refusals name `file:line` and are excellent. The character validator's name neither
— and the offending character is a bidi override, a NUL or an escape, so it cannot be found by
looking either. **And it echoes**: verified at byte level, the refusal reprints `342 200 256`
(U+202E, the trojan-source character) and raw `033` escapes into the terminal. Manifests arrive
from shared configs, not only from the user's own hand.

**Fix.** Route the character validator's refusals through the same `Origin` the grammar's use, and
escape non-printing characters in the message — name the codepoint (`U+202E`) rather than emitting
it. `rustc` refuses to compile a doc comment containing this character; Shall prints it.

**Test first.** Already red: `tests/grade4_refusal_names_the_line_tests.rs` (2 red, 1 green control
asserting the grammar's own refusal in the same fixture still names the line).

**Siblings.** Every `Error::Validation` construction site — the class is "refusals that skipped the
location decoration", not this one message.

---

## Tier 4 (round 6) — gates, and the number nobody computes

### B39 · `R-7` — the mutation gate has a ceiling and no floor

`scripts/harness-mutation-test.sh` fails when survivors exceed the budget (92 container / 86
Windows) and asserts nothing about `CAUGHT`. Proven rather than argued — pointed at a harness with
three checks it reports `ok: 2 survivors, within the budget of 92; 1 checks did their job` and
exits 0. It cannot tell *"the checks got stronger"* from *"the checks were deleted"*.

**Fix.** A floor under `CAUGHT`, ratcheted the same way and in the same file, per harness.

**Siblings.** A *total* coverage collapse is caught by the lifecycle ratchet and the subcommand
audit; an assertion-strength collapse — deleting the effect assertions while still invoking every
subcommand — passes all three gates today. Check the other two for the same one-sidedness.

### B40 · `R-8` — one orphan fixture

`tests/fixtures/cargo/install-list.txt` was captured from the real tool and is read by no test —
the only orphan of the 30. The cargo parser is correct, so this is a false signal of coverage
rather than a hidden bug, in the same commit that established the rule fixtures exist to serve.
Wire it up (assert the indented binary lines are not packages, which is `pixi:exposes`' family) or
delete it. **Then add the check**: a test that every file under `tests/fixtures/` is referenced.

### B41 · `R-9` / **`B14` reopened** — nothing measures latency

Release binary, five samples, a machine with 24 ready backends:

```
shall list           min  6.13s   median 20.43s   max 40.41s
shall check health   min  8.47s   median 18.58s   max 35.92s
shall check          min 17.02s   median 18.71s   max 55.40s
shall policy / vars / eval / check config          ~0.25s
```

The split is diagnostic: config-only commands are instant, anything that queries the managers is
6–55 seconds **with a 6× spread on the same command**. B14 fixed one 98-second `info`; the budget
it asked for was never built, so nothing would notice the next one. §8.1's A+ bar names latency
budgets per command class explicitly.

**Fix.** A budget per command class, enforced in CI, and a number in the output when it is
exceeded. Start by measuring on a runner rather than trusting the figures above — that machine is
an upper bound, not a norm.

### B42 · make `cargo test` green on three platforms

It is green on Windows (1542 tests, 49 targets) and red on Linux and macOS. Three targets, three
causes, and they need three different things:

- `grade2_flag_drift_blindspot_tests` — **`Q14` is ruled** (2026-07-30): helm 3 does not verify
  plugins at all, so `@unverified` there is accepted in silence and no flag is built. The test
  asserts the one-directional version and is red for a reason that was never drift. **Replace it**
  with the two-directional assertion — a flag where the tool verifies, none and no warning where it
  does not — which can go red on either helm version. Do not skip it.
- `grade3_resource_idempotency_tests` — **already repaired** by the grader, in the commit before
  this document. Green on both platforms now.
- `grade2_info_tests` — B37, a real defect.

Not because green is the goal — this whole document's premise is that green is a floor — but
because a suite red for three unrelated reasons teaches everyone to stop reading it, and one of
those reasons is a live defect currently indistinguishable from the two that are not.

### B43 · **`Q15`** — `bundle` and `export` honour `--dry-run`; `plan` does not ✅ **ruled 2026-07-30**

`shall --dry-run bundle --out X` writes all nine files and prints *"Bundle written to X"* with no
marker — a preview that manufactured the artifact it was asked to describe, and said so in the past
tense. `--dry-run plan` writes `shall-plan.json`, the same as `plan`.

**RULED (owner, 2026-07-30): split them, and `export` goes with `bundle`.** The line is not *did
the user name the path* — they name it for `bundle` too — but **is the file the preview or the
result**:

- **`bundle` and `export` honour the flag.** They print what they would write and to where, with
  the `[DRY-RUN]` marker, and write nothing. Their product outlives the run and can be carried to
  another machine; one made by a preview is indistinguishable from one made deliberately.
- **`plan` is exempt, and it is the whole exemption.** Its file *is* the preview. A `--dry-run plan`
  that wrote nothing would be a command with no output.

**`export` was never measured** — the grader's fixture had no package to export, so neither run
wrote anything and there was no control. It is ruled with `bundle` on the reasoning, not on a
measurement. **Measure it before you build it**, and if it turns out to behave differently from
`bundle` say so rather than assuming this order was right.

**Test first.** Extend `tests/dry_run_every_verb_tests.rs` rather than adding a file: `bundle` and
`export` become driven cases instead of exemptions, and `plan`'s exemption changes from *"writes to
a path the user names"* — which was the unruled guess — to the ruled reason, that its file is the
preview. Note the existing rule in that file: an exemption says what the fixture cannot supply,
never what the instrument cannot see.

**Siblings.** Every verb currently exempted there for writing to a user-named path. `sbom` takes no
output flag and prints to stdout, so it is not in this family — confirm that rather than assume it.

The rule is in **II.8b**; the reason is in **V.105**.

---

## What Round 6 does *not* change

The safety core is in good shape and was checked adversarially rather than assumed — every path
to a backend `remove`/`purge` has a guard, enumerated from the sink; `SIGKILL` mid-sync leaves a
reconcilable journal; `heal` verifies against the machine rather than closing entries. **Do not
refactor any of it while fixing B36**, which is entirely about what `heal` prints and returns.

Still untouched and still the thing that would move the grade most: **real lifecycles for the
24 of 56 backends that have never had one.**

*(Corrected 2026-07-30 by the round-6 builder: this paragraph named two items, and the other —
B18's supported/experimental split — had been ruled and **rejected** as `Q4` on 2026-07-27,
three days before this section was written. Under that ruling the missing coverage is a release
blocker rather than something a label makes acceptable, so the remaining item is the coverage
itself and there is no second one.)*

**The register is at zero open.** `Q14`, `Q15` and `Q16` were all ruled on 2026-07-30 and are
built into B33, B35/B42 and B43 above — so nothing in this document is waiting on the owner, and
three of the ⚠️ rows in §0.1 that predate round 6 still are.

---

## What NOT to do

- **Do not fix a harness by making it green.** If deleting the scrub in B5 turns a run red, that
  is the fix working. Report red honestly; B1 makes it green legitimately.
- **Do not widen an exemption list to pass.** Both harnesses already exempt `undo`, a subcommand
  that no longer exists (`E29`); exemptions are unvalidated. Assert that every exempted name
  exists, and delete the stale one.
- **Do not implement a §0.1 item without a ruling**, however obvious it looks.
- **Do not add a backend.** `Q4`, and unchanged by its rejection of the label: no new backend
  until the current set has a real lifecycle in an automated gate.
- **Do not trust a green suite as evidence of anything.** 1,359 tests were green while every
  defect in `READINESS` was live. Green is a floor.
- **Do not check state at the wrong moment.** The harness uninstalls each package immediately
  after listing it; inspecting the machine afterwards proves nothing. Two findings in `READINESS`
  were initially wrong for exactly this reason, and the correction is recorded there.
- **Do not test a stale artifact** *(added round 2, after it nearly produced four false findings)*.
  The container images bake `shall` at **build** time — a cached image tests yesterday's binary, so
  rebuild before believing a container result. `target/release/shall.exe` is **not** rebuilt by
  `cargo build --all-targets`, and on Windows `cargo build --release` fails with `Access is denied`
  if any `shall` process is running — it will report the failure and leave the old binary in place.
  `release-check.ps1` builds release first and is safe; invoking the harness directly is not.
  **Verify the artifact, not the build log**: run one known reproduction through it first.
- **Do not run two integration sweeps at once.** Both harnesses use fixed paths
  (`/tmp/shall-it-win-config`, `/tmp/shall-it-win-state`), so concurrent runs corrupt each other —
  producing 120-second lock timeouts and `Profile … already exists` failures that look like product
  defects and are not. Killing a wrapper process does not kill the `bash` script; check with
  `ps`/`Get-CimInstance` that it is actually gone.
- **Do not test your own oracle by assuming it works.** "All 24 `[READY]` backends answer `list`"
  was measured and true and **meaningless**, because a backend that does not exist answers `list`
  the same way. Before trusting a check you wrote, feed it something it must reject.
- **Do not let a test depend on where the checkout is, or on a message only one platform prints**
  *(added round 6, by the grader, about the grader's own test)*.
  `grade3_resource_idempotency_tests` had both faults and was red on Linux and macOS from the day
  it was committed while passing on its author's box: its `link:` targets were inside `$HOME` only
  because the repo happened to live under `C:\Users\Administrator`, and its central assertion
  counted `Link:`, the text of the **Windows-only** cross-drive-fallback warning — a count that is
  zero on Linux whatever `sync` does. Both are the S33 shape. Ask the filesystem, not the output,
  when the question is "did the work happen"; and make the environment your fixture needs *by
  construction* (hand the child its own `HOME`) rather than inheriting it and hoping.

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

---

# Round 7 — the coverage round, 2026-07-30

**Not a grader's round.** This one came from the owner: *"you need to build the test and harness
for all of it to make sure it really works."* It is the direct execution of `Q4`, which made a
backend with no real lifecycle a **release blocker**, and of `Q17`, which ruled on how the
remaining coverage is obtained.

## What the number actually is

Both harnesses' `canary()` and `no_lifecycle_reason()` tables, cross-referenced against the union
of the two registries (Windows 48 + Linux 56 = **60 distinct backends**), measured 2026-07-30:

| | count |
|---|---|
| never completed a real lifecycle in **any** harness | **20** |
| in **neither** table of **either** harness — no coverage and no stated reason | **12** |

The twelve were `lvm zfs pkg pkg_add pkgin eopkg guix paru slackpkg xbps yay zypper`.

**Why no gate could see this.** Each sweep audits only its own registry. `winget`, `choco` and
`psresource` exist only on Windows and were excused there; they are absent from Linux entirely.
So the question "is `winget` ever lifecycled anywhere?" was asked by nothing, and the answer was
no. An excuse on the only harness that can run a backend is indistinguishable from coverage.

## What round 7 changed

- **`primary_manager_image()`** (container harness) — a distro's own manager is lifecycled by
  section 5 of the image built for it, and the gap audit was counting all of them as uncovered.
  The table is a claim about runs this process cannot see, so it is **verified on the run of the
  image it names**: no row can excuse a backend on the strength of a sweep nobody performs.
  `emerge` is deliberately absent, because gentoo is SMOKE_ONLY and installs nothing.
- **`Dockerfile.opensuse` (zypper) and `Dockerfile.void` (xbps)**, both in the **default** matrix.
  Opt-in is how a backend stays untested with a Dockerfile sitting next to it.
- **`Dockerfile.storage`** — the first `--privileged` image, for `btrfs`, `lvm` and `zfs` on
  loopback devices. Authorised by the owner (`Q17`).
- **Windows: `winget` has a real lifecycle**, canary `ajeetdsouza.zoxide` — measured by hand
  first, with Shall's exact argv, unelevated, no `--scope` flag: install, list, uninstall, gone.
  `choco` and `psresource` are now skipped only for a **detected** reason (shell not elevated /
  host has no PSResourceGet cmdlets), the way `pip` already handled PEP 668.
- **`LIFECYCLE_GAP_CEILING=11`** for the container harness — measured on the openSUSE run and
  lowered again once btrfs got a device. It may only go down.
- **CI runs all of it.** `ci.yml` hardcoded a four-distro matrix, so a new image in `run.sh`'s
  default would never have run there — the same "opt-in is how a backend stays untested" trap,
  one layer up. `opensuse` and `void` are in the **fast** matrix now (every push); `storage` was
  a nightly job for one day because it was **deliberately red on `Q18`**, and it joined the fast
  set on 2026-07-31 when that ruling landed and it went green. It stays a job of its own rather
  than a matrix leg because it is the only one needing `--privileged` and the host's
  `/lib/modules`, and a matrix cannot vary `docker run` flags.
- **`tests/lifecycle_coverage_union_tests.rs`** — the gate that would have caught `winget`. It
  reads both harnesses' tables and asks the one question neither sweep can: *is this backend
  reachable anywhere?* Ceiling 15, may only go down. It rejected its own author's first draft
  (`brew` is reachable via the Windows harness, which the macOS leg runs).

## The two defects the coverage work uncovered

Both were found by *building the missing harness and running it*, which is the point.

1. **`psresource` was compiled on Windows only.** `pub mod psresource` carried
   `#[cfg(target_os = "windows")]`, so its code did not exist on Linux or macOS — which also
   made it the one OS-native backend that could not appear in the argv table, because the row
   would not compile where it is most needed. `registry.rs`'s own doc comment says this class was
   fixed on 2026-07-26; it was fixed for `mas` and `apt` and `psresource` survived. It therefore
   had **no argv check off Windows and no lifecycle anywhere**. Un-gated; six rows added
   (`psresource`, `yay`, `paru`, `asdf`, `cabal`, `stack`); the table's remove column is now
   `Option<&str>`, and `None` asserts that removal *refuses* rather than silently running
   something. `tests/os_native_argv_coverage_tests.rs` fails when the next registrar arrives
   without a row or a written reason.

2. **The generic dependency parser took the first word of every line.** The first real `zypper`
   run in the project's history could not install a single package. `zypper info --requires jq`
   opens with `Loading repository data...`, reports `Installed : No`, and prints a paragraph of
   prose; the parser returned **25 "dependencies" of which 4 were real**, including
   `---------------------------`, `x86_64`, `150.4` and `you`. The planner adds every dependency
   as an install node and then asks *that* node for its dependencies, which returned the same
   words — so the sweep died on a `requires` cycle between three adverbs:

   ```
   zypper:No requires zypper:Loading
   zypper:Loading requires zypper:Reading
   zypper:Reading requires zypper:No
   ```

   And because `sync` syncs the whole model, that one broken manager failed **every other
   backend's lifecycle in the same image**. Fixed in two places, instance and generator:
   `zypper` no longer re-derives a closure its own installer resolves (`depends_args: None`,
   like apt, dnf and pacman — it was the only system manager that asked), and
   `parse_dependency_output` now takes only what a dependency label introduces.

   **Corrected 2026-08-06 (`Y9`):** "the only system manager that asked" was true of the
   `ManagerConfig` rows and false of the tree. `dnf` and `pacman` are hand-written modules and
   do not read `depends_args` at all — both ran a real dependency query (`dnf repoquery
   --requires --resolve`, `pacman -Si`) for another six days, alongside brew, flatpak, snap,
   vscode and xbps. Setting the row was a fix scoped to the artifact under review; the property
   — nothing that plans asks a backend what a package depends on — is now the rule, and
   `tests/a_plan_installs_only_declarations_tests.rs` is the gate.

## What is still open, and what it needs

`LIFECYCLE_GAP_CEILING=11` on the container harness, 0 on Windows. Lower them, do not raise:

| backend(s) | what it needs |
|---|---|
| `btrfs`, `lvm` | the storage image, wired and run — **in progress** |
| `zfs` | a host kernel with the module. `modprobe -n zfs` says no on the WSL2 kernel; out-of-tree. Detected, not assumed. |
| `eopkg` | no Solus image exists on any public registry (probed 2026-07-30) |
| `guix` | an image built from an install script rather than a published base |
| ~~`slackpkg`~~ | **landed 2026-08-14.** `Dockerfile.slackware` bootstraps the toolchain through slackpkg itself. Finding it a real manager cost two defects: it sorted below the language managers (VI.7) and it was sent a `--` it reads as the search pattern (VI.9). |
| ~~`yay`, `paru`~~ | **landed 2026-08-14.** The non-root leg exists: the arch image runs its harness as an unprivileged user and Shall escalates through `sudo -n`. It also exposed VI.6 and VI.8. |
| `pkg`, `pkg_add`, `pkgin` | FreeBSD / OpenBSD / NetBSD userlands. A container shares the host's **Linux** kernel; these need VMs. |
| `mas` | a signed-in Mac |
| `brew` | a container canary (it has a Windows one and no Linux one) |
| `emerge` | stays smoke-only by design; a source-building lifecycle costs hours |

**Do not close any of these by writing a reason.** `Q4` and `Q17` both say the same thing: an
exemption must be something the harness genuinely cannot do — no such userland, no such device,
no account to sign in with — and it must be **detected at run time**, never assumed. "It touches
the real machine" is not a reason; every package manager does.

### What the privileged image found the moment it existed

The `storage` image gave `btrfs`, `lvm` and `zfs` their first run. Both installable ones failed
immediately, for two *different* reasons, and neither is subtle:

- **`btrfs:` could not be written at all.** `Validator::is_path_oriented_backend` lists
  `link | web | github | appimage` — every backend whose name is a path — and omitted the one
  whose name is *most* literally a filesystem path. `btrfs:/mnt/data/vol` was rejected as
  `Path traversal detected in name`. **Fixed** (`btrfs` added to the list; `..`, the character
  allowlist and injection blocking all still apply, and the test asserts `lvm`, `zfs` and
  `setting` are *not* widened, since their names carry a separator and never a leading one).
- **`lvm:` could not be written at all.** It requires `@size`, and II.2's option table did not
  permit it — the backend's own error message instructed the user to write a line the parser
  refused. That was **`Q18`**, raised rather than fixed, because Part II said both things and
  rule 4 forbids the builder fixing Part II. `btrfs` and `zfs` had the same problem confined to
  their options (`@quota`, `@mount`, `@options`), so they installed by name and could never be
  sized or mounted. **RULED 2026-07-31: the table was wrong.** The keys are legal on the backends
  that read them and refused by name elsewhere, `@options` is spelled `@mount_options`, and the
  same audit found three more keys in the identical state — `snap`'s `@classic`, and `@shim` /
  `@sandbox`, which R3 had made the *only* way to ask for a shim.

**The lvm canary was written the way Part II said it should be, and the sweep failed on it by
name, every run, on purpose.** It passes now. The rule it was standing in for still holds: never
remove an option to get a green sweep, because that hides a defect in the program behind a change
to the test.

**One observation, not yet a work order.** `tests/dry_run_every_verb_tests.rs` dominates the wall
clock of a full `cargo test` — 30–45 minutes on the machine this project is developed on, against
seconds on a clean runner. The cause is `adopt -y`: its preview asks every installed manager to
list everything, and this box has twenty-four of them. That makes the suite's runtime — and
possibly its coverage — **a function of what the developer happens to have installed**, which is
the same shape as `G-11`, the finding that a used machine silently gets a weaker sweep. Worth
measuring before touching: narrowing the fixture's `priority` would make it fast and
deterministic, and might also stop it exercising the backends where a preview is most likely to
write something.

Two harness facts learned the hard way, both worth keeping:

- A container borrows the host's kernel but **not its module files**, so `modprobe btrfs` inside
  one searches the image's empty `/lib/modules` and fails. The harness correctly reported "this
  kernel has no btrfs" on a kernel that has it. `run.sh` now mounts `/lib/modules` read-only for
  the storage image.
- **Do not edit a bind-mounted harness script while a container is running it.** `sh` reads a
  script incrementally; changing its bytes mid-run shifts every offset after the read head. Any
  run overlapping such an edit is untrustworthy and must be repeated.

### What the Windows sweep found once winget was in it

`winget` completed a real lifecycle and the real-lifecycle ratchet moved **4 → 5** on
`windows-native-windows-local`. One assertion had to be corrected — and *not* by weakening it:

- **A winget portable package is not on the running shell's PATH, and that is correct.** It lands
  in `%LOCALAPPDATA%\Microsoft\WinGet\Links` and winget adds that directory to the **persisted**
  user PATH (`Path environment variable modified; restart your shell to use the new value`). A
  shell that is already running cannot see it, so `on_path` asks a question whose honest answer
  is *"yes, in your next shell"*. The off-PATH fallback does not apply either: it reads Shall's
  own *"installs its executables into DIR, which is not on your PATH"* warning, and Shall is
  right not to print that here — the directory **is** on PATH, just not on this process's copy.
  The canary asserts no binary, with that reason written down, and `list --backend winget`
  remains the presence assertion.

**Three failures on this host that round 7 did not introduce**, listed so the next builder does
not attribute them to this work. The recorded floor of **4** encodes that this host has been
getting four real lifecycles for some time, so these have been failing here before tonight.

- **`luarocks:luafilesystem` — checked, and it is not a Shall defect.** This host's luarocks
  targets **Lua 5.5**, and no rock manifest is published for 5.5: all three mirrors 404, and the
  summary is `No results matching query were found for Lua 5.5`. Shall classifies the download
  failures transient, retries, and `falsify_transience` downgrades them to `Exhausted` — so the
  run hard-fails and withdraws nothing, which is correct. **Do not add the summary line to
  `luarocks()`'s permanent markers**: `exit_policy.rs:314` explains at length why it is
  deliberately in neither list, and marking it permanent would beat the download failures
  printed above it *in the same output*, turning a broken mirror into a promise that a rock will
  never exist. The fix here is the host's Lua version, not the code.
- `nimble:nimjson` (fails permanently) and `pub:sass` (not on PATH, nothing said where it went)
  are unexamined. Each wants the treatment the rest of this round got: run it, read the error,
  fix the cause — and check the exit-policy comment first, because one of these three already
  had the answer written down.

**`heal recovers an uninterrupted transaction` fails on both Windows and Void, and on Void it was
a cascade.** Once `pnpm` installed correctly the heal check passed with no change to heal. Treat a
heal failure in a sweep with other failures as a symptom until the others are cleared — one broken
manager produces failures that look like defects in unrelated verbs.

### Three exemptions that survive on the old standard and not on Q17's

These are *already in* `no_lifecycle_reason()` and therefore invisible to the ceiling, which is
exactly how a cost gets recorded as an impossibility. Read each against Q17 before trusting it:

- **`stack`** — *"its first install downloads a whole GHC toolchain (~2 GB)"*. That is a **cost**,
  not an impossibility, and the fix is the one already used for every other manager in the
  `tools` image: bake the toolchain in at build time, so the lifecycle pays milliseconds.
- **`flatpak`** — *"the smallest app pulls a multi-GB runtime, and there is no session bus here"*.
  Two claims welded together: the runtime size is cost, the session bus is real. Split them, and
  test whether `flatpak --user` on a `dbus-run-session` closes it.
- **`appimage`** — *"needs FUSE, which a plain container does not have"*. True of a *plain*
  container, and there is now a `--privileged` one. `/dev/fuse` is reachable there. What is left
  is the second half of its reason — no stable public canary — and that is a smaller problem
  than the one the sentence is doing duty for.

Each is a sentence that was true when written and was never re-derived, which is the disease
this whole round is about.

---

# Round 8 — the round-6 grade, worked (2026-07-31)

Source: `docs/archive/GRADE-2026-07-31.md` (graded at `a510728`, **B−**). **All eight findings `G-1`–`G-8`
are closed, plus `E3`**, which had re-reproduced. One commit per finding; each carries its own
red-before / green-after evidence and an independent reproduction. The narrative is
`spec/history.md`, session 2026-07-31 (third).

| finding | what shipped |
|---|---|
| `E3` | `cargo fmt` clean. The gate was fixed in round 1 and not run — nothing yet stops the next one, and a pre-commit hook is a change to how this repo is worked, not to the program |
| `G-1` | `@shim`/`@sandbox` resolve to a `shim:` extra, so the ledger/guard/preview answer for them. `reconcile_all_shims` **deleted**. II.2 + **V.111** |
| `G-2` | `<known backend>:` shields a line from set math, so a name with `\` is a package. Set math between qualified packages still parses |
| `G-3` | `assert_binary_reachable` takes the prior resolution, both harnesses, all four call sites; three predicates in `harness-logic-test.sh` |
| `G-4` | `--no-fail-fast`, `opensuse`/`void`/`storage` in the local matrix, and a reachability row for every CI job |
| `G-5` | a failed command's output is picked by the manager's own vocabulary, capped, counted, and sanitised through `validator::printable` |
| `G-6` | `shall-failure-class:` only on a pipe |
| `G-7` | four captured `winget` fixtures; the parser's hand-built table retired; an empty fixture directory now fails the gate |
| `G-8` | `refuses_with_3` beside `nok`; the fail-everything stub is part of the mutation gate with its own ratchet. Found and fixed a live `B22` sibling: `restore`'s refusal exited 1 |

**Built past the eight, because each pointed at a class rather than a line:** `nok_saying`
(a negative check asserts Shall's sentence, not only a non-zero code — the fail-everything
survivors went 12 -> 7 on Windows and 17 -> 12 in the container, both ratchets lowered);
every grammar refusal drawn through `validator::printable`, so a config saved by Notepad no
longer refuses with two identical-looking names; a width cap on a failed command's output,
because a stream with no newlines is one line; and the punctuation family pinned so the next
`\ | & (` case needs no third discovery.

**The one question was asked and ruled the same day — `Q22`:** a UTF-8 BOM, what Windows
Notepad writes, was read as part of the first backend name. **RULED: read the file.** Stripped at
the parser (never at the read — `edit.rs` rewrites these files and II.16 forbids changing their
encoding), only at the start, and a mid-line U+FEFF is still refused by name. II.1 + V.112.

**What this round did NOT do**, so the next grader does not have to re-derive it:

- **Nothing was pushed.** The unpushed backlog (`E26`) is larger, not smaller, and no CI has seen
  any of it. The pty half of `G-6` and both container harnesses' changes are unrun on Linux.
- **Twelve checks still survive the fail-everything stub** on the Windows harness. Each is an
  honest `nok` over a command that really does fail with 1; strengthening them means asserting
  the message, not the code.
- **The grammar refusals exit 1**, measured, and stay `nok`. Whether a parse refusal ought to be
  exit 3 is a published-contract question — a ruling, not a fix.
- The grader's two source-level assertions in `grade6_option_edit_reaches_the_machine_tests.rs`
  were **replaced by behavioural ones** driven through the shipped binary. That file's own
  `expect()` said to re-derive the finding if `reconcile_all_shims` went; it went.

---

# Round 9 — the coverage round, 2026-07-31 (owner-directed)

**Not a grader's round.** The owner's instruction: *"build the @ first, then build as many of the
real tests as you can, test them, and fix. iterate"*, with a standing direction for every
judgement call — **more features, more extensibility, more intuitive** — and no questions.

## What shipped

| | |
|---|---|
| `Q23` | a leading `@` is part of a package name. Every scoped npm package was unwritable; both CI `Build` jobs were red on it |
| `Q22` | a byte-order mark is an encoding artefact, not part of your first backend's name |
| winget identifiers | `ARP\Machine\X64\{GUID}` and `MSIX\…` are names. The validator had to be taught what the grammar already knew — half a fix wedged the model |
| `nix` | **installed at last** (its installer refuses to run as root and `\|\| echo SKIP` ate it), and its first lifecycle found a real bug |
| nix `list` | blind to everything nix installs: schema v3 made `elements` a name-keyed object, Shall asked for an array |
| exemptions | five host-respect ones open on disposable hosts; `web:`/`appimage` given pinned canaries; `stack`/`flatpak` rewritten as PRICES |
| canaries | one binary name per backend — `cowsay` had served four, `pycowsay` two, `hello` two |
| gates | the heal check no longer asserts a premise the sweep falsifies; the union gate no longer counts an empty canary row as coverage |
| release | `0.7.0` cut; both installers install the newest tag rather than `HEAD` |

## The measurements

`tools` 25 -> **26** real lifecycles (`pass=359 fail=0 soft=11`) · Windows native 5 -> **6** ·
container gap ceiling 11 -> **10** · full Windows suite **exit 0, 62 targets** · register at
132 decisions, 130 answered, 0 open.

## Closed at the start of round 10: choco, and its twin

**`choco`'s first real lifecycle** (CI 30684191791) was written up as three failures admitting
two readings, and left red pending "twenty minutes on an elevated Windows shell". It got them
on 2026-08-01. **The three lines were one defect and it was Shall's.** Full account in
`spec/history.md`; the short version:

- `bat` pulls **eleven** packages, not one. The harness comment calling it dependency-free was
  never measured.
- Chocolatey only raises its exit code to 1 for a failed package **when nothing has set one
  already**, so a dependency asking for a reboot leaves `3010` over an install of nothing —
  and `3010` was on Shall's `benign_exits`. Shall reported success. The rest followed.
- The "refused over dependencies" reading was choco's *generic* troubleshooting footer, printed
  after any uninstall failure. `detail_for_user` falls back to the tail when **nothing matches**,
  and choco's one marker was an *install*-time phrasing a failed uninstall never prints — so
  every choco failure ever reported was that footer, with the real reason inside the
  `(13 more line(s))`.
- **The family is two.** `choco` and `winget` were the only policies that forgive a non-zero
  exit and the only two with no way to contradict it. `winget install <typo>` reported success.
  That pair is no longer read off the table by hand: `tests/benign_exit_contradiction_tests.rs`
  derives it from the registry, printed the two names when run against the old policy, and goes
  red on the next policy that forgives a code it cannot argue with.

**The method is the transferable part.** "Cannot reproduce, this shell is not elevated" was a
statement about a shell. The account was an Administrator with a filtered token; one
`Start-Process -Verb RunAs` and a consent click raises it. Before spending a CI round on a
question, check whether the box in front of you can answer it — and check what the *comment*
claims against what the tool does, because `bat` "has no dependencies" was the load-bearing lie.

That was the third backend in round 9 to break the moment it was covered, after `nix` (a `list`
blind to everything it installs) and winget's own identifiers. **The pattern is the finding**:
every exemption removed so far has paid for itself in defects within minutes, which is the
argument for removing the rest.

**The probe found a worse bug than the one it was for.** Measuring what choco says to an
unreachable feed returned `The package was not found with the source(s) listed` — choco's
*absent* marker, word for word — and an absent marker **deletes the declaration from the user's
config files**. A dropped VPN silently removed lines for packages that exist. apt is the same
bug and more common (`Unable to locate package` when it could not fetch the lists); `luarocks`
is the one backend where it was known and written down, and nobody asked whether the others had
it. `transient_markers` now outranks `absent_markers`. The lesson is the one this round keeps
teaching: **the measurement you run for one question answers a question you did not ask** — and
the reason it was reachable at all is that a marker was believed without asking whether the
manager was in a position to say it.

**Still open:**
- Why the eleven-package install failed *on the runner specifically* — this box installs all
  eleven clean. Shall will now print choco's own reason instead of swallowing it, so the next
  `Integration (Windows native)` run answers it rather than needing another round.
- Whether `winget` has the absent-on-network-failure shape too. Unmeasured, not assumed. **The
  first probe design was wrong and the run said so:** `winget source add` validates the source
  at add time and refused a REST source on a dead port (`Failed to open the added source`, exit
  -1978335163), so it measured a missing source *name*, not a network failure. The design that
  reaches the right state is `--proxy http://127.0.0.1:9` behind `winget settings --enable
  ProxyCommandLineOptions`, against an id that really exists. Still unrun.

## The lifecycle run that was meant to close that found two bigger things (2026-08-02)

**Shall bounded no child process at all, and it cost 76 minutes.** `shall -y uninstall
choco:bat` wedged on the pre-sync `Checkpoint-Computer`; Windows event 8194 records the restore
point **created 18 seconds in**, and the process then emitted nothing on either pipe for 76
minutes without exiting. The only timeout in the tree wraps the transaction DAG, and
`auto_snapshot` is called from five places, every one before the DAG starts. The spawn already
sets `kill_on_drop(true)` with a comment naming *"the global timeout"* — the cancel machinery
was built, and nothing outside the DAG ever fired it. **Third occurrence:** the 2026-07-22
section records two more, both killed by hand, both written up as undiagnosed, and what got
fixed then was the *harness*, not the product.

Fixed: a bound on **silence, not duration** (`command_idle_timeout_secs`, default 900, `0`
disables), plus ten spawns outside the executor that captured both streams while leaving stdin
inherited — `git` among them. Registered **Q24, BUILT NEVER RULED**: the bound is settled by
II.12, the number is a judgement nobody has measured.

**Measured and cleared, so nobody re-runs it:** the tidy theory was that `Checkpoint-Computer`'s
poll loop is the defect and `Invoke-CimMethod CreateRestorePoint` would return where it does
not. Elevated, each probe bounded by a job timeout: **50.8s vs 53.3s**, both returning, throttle
off so both did real work. There is no API swap to make; the wedge is intermittent and has no
known cause. A Windows restore point costs **~51s**, taken before every mutating sync.

**And the thing to attack next: `plan` takes 439.6s and it is an algorithm, not a missing
`buffer_unordered`.** `GenericQueryable::info` answers "is this one package installed?" by
running a full `list_installed()` and doing a linear find; the planner calls it once per
declared package, so 302 declarations run ~302 full machine listings. `installed_sets()` twenty
lines above it already does the right thing for the removal side — one listing per backend, then
set membership. Two mechanisms for one question. Separately and **unexplained**: sampled
concurrency during a live plan is **1**, against `buffer_unordered(20)`. See `spec/history.md`
for the hypothesis and the one-number experiment that settles it. **Not fixed** — it touches the
change-detection core, which D13's ruling already flagged as risky ground, and the obvious
fix (cache `list_installed` per run) has a real staleness trap after an install during `sync`.

## What a future round should attack, in this order

1. **The 15 backends unreachable anywhere.** `zfs` and the BSDs need VMs, not containers; `mas`
   needs a signed-in Mac; `yay`/`paru` need a non-root leg, which is an afternoon.
2. **`purge-unmanaged` has still never run against a machine that matters.** It is the most
   destructive command in the product and the largest untested area.
3. **The suite takes an hour on a developer's box**, nearly all of it one test whose cost is a
   function of how many managers the developer has installed. An hour-long gate is a gate that
   gets skipped, and `E3` came back this week for exactly that reason.
4. **`stack` and `flatpak`** — read their reasons against `Q17` again. They are written as
   prices now; the flatpak one names the experiment that would settle half of it.

---

# Round 11 — the untested half, 2026-08-04 (owner-directed)

**Not a grader's round.** Source: `docs/archive/GRADE-2026-08-04.md` (**B+**), whose §5 is not a list of
bugs but a list of things **nothing has ever run** — and the owner's instruction on top of it:
*"it is crucial that we test — real live tests — as much as we can… build the tests that really
test everything like a human."*

The narrative, including the two instruments that reported defects that were not there, is
`spec/history.md`, session 2026-08-04.

## What now runs that never ran

Four sections in `docker/integration/run-in-container.sh`, so every container leg in CI gets them
without a new job:

| | what it drives |
|---|---|
| **16d** | `SIGKILL` mid-transaction at three points — log-open, manager-part-way, and a `setsid` **group** kill that takes the package manager down too — then `heal`, convergence, and idempotency |
| **16e** | two writers against one `DataLock`, and a killed holder whose stamp file outlives it |
| **16f** | `sudo` with a real password on a real pty: the prompt, the password, a wrong password, an unanswered prompt, the keepalive, and a privileged install **and removal** by a non-root user |
| **16g** | snapshot → mutate → restore on a real LVM device, through a user-declared provider row (U27's exemplar) |

`sudo`, `util-linux` and `passwd` joined `Dockerfile.ubuntu`, `.storage` and `.tools` — they are
the **fixture** for 16f, not conveniences. Without them that section reports "not driven here" and
the escalation path stays unrun on every platform.

## Two defects, both found by trying to drive something

1. **`snapshot restore` was unreachable for every provider but btrfs and timeshift** — a
   by-name `match` in `show_diff_and_confirm`, three days' work after `U27` ruled providers are
   rows. Narrower than it first reads: `sync`'s health revert and `bisect` reach
   `SnapshotProvider::restore` without it, so the automatic rollback was never blocked; what was
   blocked is the command a person runs to recover by hand.
   `tests/snapshot_restore_reaches_every_provider_tests.rs`, red before and green after.
2. **`heal` had two branches that did nothing and said nothing** — `if let Some(cap) =
   registry.get(..) { if let Some(handler) = ..as_installable() { … } }`, no `else` on either, so
   an entry naming an absent manager was neither recovered, nor failed, nor mentioned, and `heal`
   returned `Ok`. B36's finding one branch over.

## What this round did NOT do

- **The Windows harness has none of it.** `scripts/integration-windows.sh` is the twin and the
  crash and two-writer checks belong there (`sudo` does not — Shall escalates only off Windows).
  A check on one harness is a check on one platform, which is round 7's own finding.
- **The grade's §3 reporting findings are open**: `--dry-run sync` names nothing and skips the
  guard (3.1), `manual_source` is wrong for 19 backends (3.3), an error names the wrong file
  (3.5), the remedy block offers a flag that cannot work (3.6). 3.2 became **`Q31`** and is the
  owner's.
- **macOS is still only compiled**, and `zfs` still needs a kernel with the module.
