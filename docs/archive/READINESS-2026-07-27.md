# LiNix — production readiness assessment

> Assessed 2026-07-27 · commit `6936475` (main, clean tree) · binary reports `linix 0.1.0`
> Method: every gate and harness run for real on this machine — Windows native and Linux
> containers via WSL — plus a code read and a hand-driven walkthrough as a first-time user.
> Nothing below is inferred from a previous document; where an earlier claim was checked and
> held, it is marked **re-verified**.

---

## 0. Index of every defect found

Every row was reproduced on this machine. `§` links to the detail. Severity is user impact, not
how interesting the bug is.

| # | severity | defect | where |
|---|---|---|---|
| E1 | **blocker** | A failed install writes the package to `modules/imperative.txt` anyway; the config is then wedged for every later command | `src/verbs/packages.rs:110` · §3.1 |
| E2 | **blocker** | Both harnesses assert "the unresolvable name is out of the manifest" *after deleting it themselves* — the check cannot fail | `run-in-container.sh:261`, `integration-windows.sh:269` · §3.2 |
| E3 | **blocker** | `cargo fmt --check` fails on 26 diffs across 10 files; CI gates on it, so the tree is red | §3.3 |
| E4 | **blocker** | The local ship gate rates `fmt` *informational* while CI rates it *fatal* — a gate weaker than CI | `release-check.ps1`, `release-check.sh` · §3.3 |
| E5 | **blocker** | Any backend whose install fails is soft-passed as "ecosystem/network variance" and its whole remaining lifecycle is skipped | `integration-windows.sh` §12 · §3.4 |
| E6 | **high** | `go`: the package installs and its binary *is* on `PATH`, but `linix list -b go` cannot see it — a `list` parser defect, confirmed against disk | §3.4 |
| E6b | **high** | `nimble`: `linix list` does not show the package right after a reported-successful install, and no binary reaches `PATH` (`~/.nimble/bin` is on `PATH` and was never created). Two hard failures; root cause not isolated | §3.4 |
| E6c | **medium** | `pub`: install and `list` are correct, but the binary is unusable — `~/.pub-cache/bin` is not on `PATH`. LiNix reports success and never mentions it | §3.4 |
| E7 | **high** | scoop's exit code is always 0: `which` resolves `scoop.cmd` (no `.PS1` in `PATHEXT`) and `cmd /C` swallows it | `src/core/executor.rs:231` · §4.1 |
| E8 | **high** | Consequently scoop failure detection rests on **one** marker string; `scoop uninstall <not-installed>` reads as success | `src/core/exit_policy.rs:194` · §4.1 |
| E9 | **high** | The careful `.ps1` shim branch is dead code on a default Windows box | `src/core/executor.rs:188` · §4.1 |
| E10 | **high** | `psresource::is_available` probes PowerShell, not PSResourceGet — reports `[READY]`, then fails every command | `src/backends/psresource.rs:120` · §4.2 |
| E11 | **high** | `helm` install fails: `plugin source does not support verification` — real argv defect, laundered as network variance | §3.4 |
| E12 | **high** | `luarocks` install fails: `No results matching query were found for Lua 5.5` — no version pinned; laundered | §3.4 |
| E13 | **medium** | `pixi global upgrade-all` was removed upstream; the pixi upgrade path is dead | `src/backends/registry.rs:1381` · §4.3 |
| E14 | **medium** | `linix info cargo:ripgrep` takes **98s** and answers "not found" — while `linix search` finds it in the same tree | §4.4 |
| E15 | **medium** | `linix search` takes 2m41s | §4.4 |
| E16 | **medium** | pixi's search parser emits 19 junk rows (`-`, `...`, bare version numbers) as package names | `src/parsers/ecosystem.rs:60` · §4.5 |
| E17 | **medium** | choco's search parser turns `5 packages found.` into a package named `5`, and its banner into `Chocolatey v2.7.3` | §4.5 |
| E18 | **medium** | Two message families for one condition; a user sees both in one screen | `src/core/manager.rs:40` vs `src/backends/generic.rs:219` · §4.6 |
| E19 | **medium** | That message names the *backend*, not the binary probed: `lvm`→`lvs`, `xbps`→`xbps-install`, `krew`→two, `appimage`→none | `src/core/manager.rs:40` · §4.6 |
| E20 | **medium** | `check health` calls a healthy machine "23 critical" — those are managers the user simply has not installed | §4.7 |
| E21 | **medium** | The `check` rollup ("ok health 25 ready") and `check health` ("23 critical") disagree about the same machine | §4.7 |
| E22 | **medium** | `purge-unmanaged` — the most destructive command — lacks the `is_terminal` refusal its sibling `reset` has 80 lines below | `src/verbs/cleanup.rs:262` · §4.8 |
| E23 | **medium** | The auto-remediation confirm, which installs packages, lacks the same guard | `src/app/diagnostics.rs:147` · §4.8 |
| E24 | **medium** | `linix nosuchcommand` exits **2**, which the readme's published contract defines as "found work to do" | §4.9 |
| E25 | **medium** | A `purge-unmanaged` ratio refusal exits **1**, not the documented **3** | `src/verbs/cleanup.rs:205` · §4.9 |
| E26 | **medium** | 10 commits (5,082 inserted lines, incl. a 711-line executor rewrite) have never been compiled or tested on Linux or macOS | §2 |
| E27 | **low** | First-run `linix sync` explains the `priority` file format by hand and never mentions `linix init` | §4.10 |
| E28 | **low** | `linix init --help` promises "a starter module"; `modules/` is created empty | §4.10 |
| E29 | **low** | Both harnesses exempt `undo`, a subcommand that no longer exists; exemption lists are unvalidated | §4.10 |
| E30 | **low** | One failure prints three times, two of them leaking `WAL`, `Node`, module paths and a 32-hex operation id | §4.10 |
| E31 | **low** | `SPEC.md` says 107 decisions / 105 answered; `decisions.md` says 104 / 102 | §4.10 |
| E32 | **low** | `PRODUCTION-READINESS-REVIEW.md` reuses `U1`/`U2`/`U3`, which `CLAUDE.md` reserves as register IDs requiring an owner ruling | §4.10 |
| E33 | **low** | `psresource`'s error is truncated mid-word in `search` output | §4.2 |
| E34 | **low** | `linix uninstall <undeclared>` runs a full convergence and can **install** unrelated packages — observed installing `cargo:ripgrep` (116s) | §3.1 |

**Verified fixed since the previous review** (re-checked, not assumed): `--verbose` works and its
help text is accurate (`-v` info, `-vv` debug); the guard is enforced on every removal path
enumerable from the code, including `heal` and rollback; the pty/stdio defect is genuinely fixed
and both pipes are drained concurrently.

---

## 1. Verdict

**Not production ready. Roughly 70% of the way there, and the remaining 30% is almost entirely
validation rather than construction.**

**The project's own ship gate agrees.** Run to completion for this assessment — which no
automated job does — `scripts/release-check.ps1` ends:

```
 RESULT  pass=275  fail=4  soft=22
=====> NO-GO: at least one hard gate failed (see above).
```

That is not an outside opinion. It is the repo's designated "am I ready to ship?" command,
answering no, on a gate nobody runs.

The core is genuinely good and the dangerous parts are the well-built parts. What is not ready
is the *breadth*. Measured on this machine rather than quoted: **`check health` registers 48
backends on Windows and the Ubuntu harness registers 56**; `linix --help` lists **61 top-level
commands**. Real coverage exists for a fraction of that — the Ubuntu lifecycle reports **7 real
lifecycles against 49 plan-smokes**. Every defect found in this assessment — without exception —
is on a path that has never been executed against the real tool it drives.

Four things block a release outright:

1. **A failed install permanently wedges the config** (§3.1). One typo and every later LiNix
   command fails until the user finds and hand-edits a file nothing told them about.
2. **Both integration harnesses contain an assertion that cannot fail**, covering exactly that
   bug, with the requirement stated verbatim in the comment above it (§3.2).
3. **CI is red on the current tree** — `cargo fmt --check` fails on 26 diffs, and the local
   ship gate rates that check *informational* while CI rates it *fatal* (§3.3).
4. **The native sweep launders real defects into soft passes** (§3.4), and running it to
   completion — which no automated gate does — produced **four hard failures and two further real
   backend defects on the first pass**. The clearest is `go`: the package installs, its binary is
   on `PATH`, and `linix list` cannot see it. For a declarative tool that is the failure shape
   that matters most — LiNix's model disagrees with the machine, so `sync` sees drift that is not
   there.

None of the four is deep, and all are hours of work each. The reason to call the product not
ready is not their difficulty — it is that item 4 is a *sampling result*. One pass through the
untested remainder yielded six new findings, and the remainder is most of the product. What is
unfinished is the verification, and §5 takes that up.

---

## 2. What was run, and what it said

Everything in this table was executed for this assessment. No result is quoted from a prior run.

| gate | where | result |
|---|---|---|
| `cargo build --all-targets` | Windows | **pass**, 0 warnings |
| `cargo test` | Windows | **pass — 1,359 passed, 0 failed** |
| `cargo clippy --all-targets --all-features -- -D warnings` | Windows | **pass**, clean |
| `cargo fmt -- --check` | Windows | **FAIL — 26 diffs across 10 files** |
| `scripts/harness-logic-test.sh` | Windows | **pass — 32/32 predicates** |
| `scripts/release-check.ps1` — hermetic gates | Windows | **pass**, with `fmt` downgraded to informational |
| `scripts/release-check.ps1` — native sweep | Windows | **`pass=275 fail=4 soft=22` → `NO-GO: at least one hard gate failed`.** The 4 hard failures are `go` ×1, `nimble` ×2, `pub` ×1; among the 22 softs are 4 real defects laundered as "network variance" |
| container lifecycle — ubuntu/apt | Docker in WSL | **pass — 283 checks, 0 fail, 5 soft** |
| container lifecycle — alpine/apk | Docker in WSL | **pass — 278 checks, 0 fail, 3 soft** |
| container lifecycle — arch/pacman | Docker in WSL | **pass — 283 checks, 0 fail, 5 soft** |
| container lifecycle — fedora/dnf | Docker in WSL | **pass — 291 checks, 0 fail, 5 soft** |
| GitHub Actions CI | remote | last run **green**, but see below |

**On CI being green.** The most recent nightly (run `30243243410`, 06:35 today) passed on all
three platforms. It tested `origin/main` at `89bed26`. **`HEAD` is 10 commits ahead of
`origin/main` and CI has never seen any of them** — 5,082 inserted lines including a 711-line
rewrite of `src/core/executor.rs`, the whole of the new `src/core/exit_policy.rs`, and a nine-way
split of `context.rs`. Those commits have been compiled and tested on Windows only. The last
time this repo pushed a large unpushed backlog, the first CI run that saw it failed on all three
platforms on two distinct bugs (S33, S34). The `cargo fmt` failure in §3.3 guarantees a red run
today; whether anything else is waiting is unknown.

**Not run, and why.** The `tools` and `gentoo` images (nightly-only, tens of minutes to build)
and the `macos-native` job (no Darwin hardware here; it has never gone green anywhere).

---

## 3. Blockers

### 3.1 A failed install writes the package into the manifest and wedges every later command

Reproduced by hand on Windows, from a clean config:

```
$ linix install scoop:definitely-not-real-xyz123 -y
Error: `scoop` failed (exit 0): Couldn't find manifest for 'definitely-not-real-xyz123'.

$ cat $LINIX_CONFIG_DIR/modules/imperative.txt
scoop:definitely-not-real-xyz123        # <-- written despite the failure
```

From that moment the config is unusable:

```
$ linix sync -y        -> Error: ... Couldn't find manifest ...     (exit 1)
$ linix upgrade        -> fails
$ linix uninstall scoop:notinstalledpkg123
    WARN  scoop:notinstalledpkg123 is not declared in any active file.
    Planned changes:  install 1  remove 0        # <-- an *uninstall* plans an install
    Error: ... definitely-not-real-xyz123 ...    # <-- and dies on an unrelated package
```

Three separate problems, in order of severity:

- **The config is wedged.** Every command parses the model, so one unresolvable line breaks all
  of them. The only escape is knowing that `modules/imperative.txt` exists and editing it by
  hand. No error message mentions the file, the line, or `linix unmanage`.
- **`uninstall` runs a full convergence** and reports `install 1` before failing on a package the
  user never named. To a human this is incomprehensible.
- **The error is self-contradictory**: `` `scoop` failed (exit 0) ``. See §4.1.

**The mechanism, and why this is a near-miss rather than an oversight.** `handle_install`
(`src/verbs/packages.rs:110`) *does* have a withdrawal path, and the comment above it states this
exact failure mode in the author's own words:

```rust
// A name no backend claims can never be satisfied by retrying, so leaving it in the file
// wedges every later command that parses the model — one typo, and `status` is broken
// until someone hand-edits a file. Withdraw it. Only this cause: a sync that failed for
// any other reason (the network, a lock, a hook) leaves the line alone, because you did
// mean it and retrying is the right move.
if let Err(e) = &synced {
    if let Some(linix::core::Error::Unresolvable { name, .. }) = e.downcast_ref() {
```

The distinction is sound: *unresolvable* is permanent and withdrawn; *install failed* might be a
lock or a mirror, so the line stays and retrying is right. The gap is that the two categories do
not cover the case. `scoop:definitely-not-real-xyz123` **resolves fine** — `scoop` is a real
backend — so the error is `CommandFailed`, not `Unresolvable`, and the line stays forever even
though it can never succeed. A bare `linix install typo` *is* withdrawn correctly; it is the
qualified `backend:typo` form, and any name a backend claims but cannot install, that wedges.

**The fix is already in the codebase and simply unwired.** The `ExitPolicy` work in the unpushed
commits computes exactly the missing fact, and scoop's policy already carries it:

```rust
// src/core/exit_policy.rs:194
pub fn scoop() -> ExitPolicy {
    ExitPolicy { permanent_markers: vec!["find manifest for"], .. }
}
```

LiNix classified this failure as **permanent** and then left the line in anyway. The condition
wants to be `Unresolvable` **or** `Retryability::Permanent`.

**Sibling paths checked:** `linix add`, `adopt` and `activate` also write manifest lines. `adopt`
writes only packages it observed installed, so it cannot mint an unsatisfiable name;
`activate`/`add` copy existing declarations rather than creating them. The
failure-writes-anyway ordering is specific to `handle_install`. **`handle_uninstall` was checked
and is correct** — it removes the line and converges through the guard, per II.8.

**One thing still needs an owner ruling.** The two harnesses disagree *in writing* about the
intended behaviour: `docker/integration/run-in-container.sh:252` says *"The failure must not be
left in the manifest"*, while `scripts/integration-windows.sh:259` says *"a pinned name that a
manager could not install is a failed sync, not a wrong name, and only a name nothing can
resolve is withdrawn."* That is a rule, not a detail, and it is not in `decisions.md`.

### 3.2 Both harnesses assert the §3.1 property with a check that cannot fail

`docker/integration/run-in-container.sh:261-267` and `scripts/integration-windows.sh:269-274`,
identically:

```sh
IMPERATIVE="$LINIX_CONFIG_DIR/modules/imperative.txt"
if [ -f "$IMPERATIVE" ]; then
    grep -v -F "linix-no-such-pkg-zzz" "$IMPERATIVE" > "$IMPERATIVE.tmp" 2>/dev/null
    mv "$IMPERATIVE.tmp" "$IMPERATIVE"          # <-- deletes the line
    nok "the unresolvable name is out of the manifest" \
        grep -q "linix-no-such-pkg-zzz" "$IMPERATIVE"   # <-- then asserts it is gone
fi
```

The harness removes the line itself and then tests that it is absent. **The assertion is testing
its own `grep -v`.** It cannot fail, and it printed `PASS the unresolvable name is out of the
manifest` on every one of the five lifecycle runs in §2 while the product does the opposite.

This is the S36/S37 class — "assertions that could not fail" — which a previous session fixed at
the predicate level. `scripts/harness-logic-test.sh` exists specifically to catch it and passed
32/32, because it lifts only `never_ran`, `assert_binary_gone` and `on_path`. This assertion is
inline, so nothing looks at it.

Worse, the scrub is load-bearing: the comment states that leaving the line in makes `rollback`,
`activate` and `restore --force` fail later in the same sweep. **The harness is working around a
known product bug in order to stay green, and reporting a PASS whose text asserts the product
does not have it.**

### 3.3 CI is red on the current tree, and the local ship gate cannot see it

```
$ cargo fmt -- --check ; echo $?
1
26 diffs across: src/app/apply/firewall.rs, src/app/context.rs, src/app/leases.rs,
src/app/mod.rs, src/core/executor.rs, src/core/exit_policy.rs, src/main.rs,
tests/exec_lifecycle_tests.rs, tests/feature_logic_tests.rs,
tests/security_and_resiliency_tests.rs
```

`.github/workflows/ci.yml:72` runs `cargo fmt -- --check` with no `continue-on-error`, so the
ubuntu `build` job fails, and `release` (which `needs: build`) never runs.

`scripts/release-check.ps1` — the file whose header calls itself *"the single 'am I ready to
ship?' gate for Windows"* — classifies the same command as `(informational)`. It printed
`[INFO] cargo fmt --check reports diffs (non-blocking)` during this assessment and then declared
the run a pass.

**A ship gate that is weaker than CI is not a gate.** This is the whole mechanism by which the
violations reached the tree. Fix: make fmt hard in `release-check.ps1` and `release-check.sh`,
matching CI.

### 3.4 The native harness converts real defects — and LiNix's own correct refusals — into soft passes

Discovered by running `release-check.ps1` to completion. Section 12 drives a real lifecycle for
every user-scoped manager on the host, and it has a catch-all: if the install step fails, it
records

```
soft  <backend>: install of <pkg> failed (ecosystem/network variance) — the checks after it did not run
```

and **skips every remaining check for that backend**. In this run that rule fired four times, and
*not one of them was network variance*:

| backend | actual cause, from the log |
|---|---|
| `github` | LiNix **correctly refused**: `refusing to deploy 'fd.exe': …already exists and LiNix did not create it` |
| `helm` | real argv defect: `Error: plugin source does not support verification. Use --verify=false to skip verification` |
| `luarocks` | real defect: `No results matching query were found for Lua 5.5` — LiNix never pins the Lua version |
| *(and the hard failure below)* | |

The consequences compound:

1. **Any backend whose install is broken has its entire lifecycle silently skipped** — list,
   PATH, remove and gone-from-list all go unrun — and the run still reports pass. Coverage
   evaporates exactly where the product is broken, which is the worst possible place for it to.
2. **The harness cannot distinguish three different outcomes** — the network flaked, LiNix has a
   bug, and LiNix refused on purpose and was right to. It calls all three "variance".
3. Two genuine backend defects (`helm`, `luarocks`) were sitting in the log of a gate that
   reported success.

Section 12 also produced **hard failures that are real bugs, and they form a family**:

```
PASS  go installed golang.org/x/example/hello for real
FAIL  go: list shows hello (output missing /hello/)

PASS  nimble installed nimjson for real
FAIL  nimble: list shows nimjson (output missing /nimjson/)
FAIL  nimble: nimjson is on PATH (rc=1)
```

Two different backends, same shape: **the install genuinely succeeds and `linix list` cannot see
the result.** That is the worst possible failure for a declarative tool — LiNix's model of the
machine disagrees with the machine, so the next `sync` sees drift and reinstalls forever, and
`check` reports a problem that is not there. `nimble` additionally installs a binary that never
reaches `PATH`.

I checked each against the machine, and the results are more mixed than the raw FAIL lines
suggest. **A caution that applies to anyone re-running this:** the harness uninstalls each
package immediately after listing it, so inspecting the machine *after* the sweep proves nothing
about what was there during it. I initially misread two of these on exactly that mistake.

| backend | what the sweep showed | verified ground truth |
|---|---|---|
| `go` | install PASS · **`list` FAIL** · on-PATH PASS | `~/go/bin/hello.exe` was on disk and on `PATH`. The install genuinely worked. **`linix list -b go` is blind to it — a real `list` parser defect.** |
| `nimble` | install PASS · **`list` FAIL** · **on-PATH FAIL** | `~/.nimble/bin` is on `PATH` and was never created, so no binary was produced. Whether the install partly failed or `list` is blind, **I could not isolate** — the subsequent `uninstall` and "gone from list" both passed, which is consistent with either. Two hard failures, root cause open. |
| `pub` | install PASS · `list` PASS · **on-PATH FAIL** | Correct on both counts. The binary is simply unreachable: **`~/.pub-cache/bin` is not on this machine's `PATH`.** |

So the honest reading, smaller than the FAIL count implies but still real:

- **`go` is a confirmed `list` defect.** LiNix's model of the machine disagrees with the machine.
  For a declarative tool that is the failure shape that matters most: `sync` sees phantom drift
  and reinstalls something already present, forever.
- **`nimble` has two unexplained hard failures** and deserves an investigation, not a verdict.
- **`pub` is a usability gap, not a correctness bug.** LiNix reports a successful install, `list`
  agrees, and the user still has no working command — because an ecosystem directory is not on
  `PATH` and nothing says so. Reporting that would cost one line.

Treat it as a class: every backend that installs into a per-user bin directory (`go`, `nimble`,
`cargo`, `pub`, `luarocks`, `mix`, `stack`, `krew`, …) needs a real install → `list` → binary
round-trip **asserted while the package is still installed**. `cargo` passes it; `go` and
`nimble` do not.

These are the only hard failures in any gate run for this assessment, and **they would have
failed CI had CI run this harness. It does not** — `release-check.ps1` is a developer-invoked
script, and CI never runs the native sweep on any platform except the nightly macOS job that has
never gone green. The bugs were sitting in a gate nobody automated.

**The fix:** classify the failure instead of assuming. LiNix already computes
`Retryability::{Transient, Permanent, Unknown}` — the harness should soften only on `Transient`,
fail hard on `Permanent`, and treat a `Refused` exit as its own outcome rather than as a failure
at all.

---

## 4. Defects below the blocker line

Ordered by user impact. Each was reproduced on this machine.

### 4.1 `scoop` failures are invisible to the exit code, and only one string catches them

`PATHEXT` on Windows does not contain `.PS1`. So `which::which("scoop")` resolves
`scoop.cmd`, and `windows_effective_command` (`src/core/executor.rs:231`) takes the
`"cmd" | "bat"` branch, running `cmd /C scoop.cmd …`. **That shim does not propagate the exit
code:**

```
$ cmd /C ...\scoop.cmd install definitely-not-real-xyz123
Couldn't find manifest for 'definitely-not-real-xyz123'.
CMD_EXIT=0
```

Consequences:

- The elaborate `.ps1` branch of `windows_shim_wrap` (`executor.rs:188-217`), with its careful
  commentary about `Out-String` and `$LASTEXITCODE`, is **dead code on a default Windows box**.
  I verified it works correctly when invoked directly — it is simply never reached.
- Every scoop outcome is therefore decided by `ExitPolicy::signals_failure`, and
  `src/core/exit_policy.rs:194` gives scoop exactly **one** failure marker: `"find manifest for"`.
- Any scoop failure that prints something else is **reported to the user as success**. Confirmed:
  `scoop uninstall <not-installed>` prints `ERROR 'x' isn't installed.` and exits 0. That string
  matches no marker. Download failures, hash mismatches, and install-script errors are in the
  same position.
- The user-visible message is incoherent: `` `scoop` failed (exit 0) ``.

This matters more than its position here suggests: **scoop is the one Windows backend that has
been live-validated**, and it is the default in `release-check.ps1`.

### 4.2 `psresource` reports healthy and cannot run a single command

`src/backends/psresource.rs:120`:

```rust
fn is_available(&self) -> bool {
    self.executor.command_exists_sync(&self.shell)   // "powershell" — always present
}
```

It probes **PowerShell**, not **PSResourceGet**, the module that actually provides
`Install-PSResource` / `Get-InstalledPSResource`. PSResourceGet does not ship with Windows
PowerShell 5.1. So on a default Windows box `check health` prints `[READY] psresource`, and then:

```
$ linix upgrade
Get-InstalledPSResource : The term 'Get-InstalledPSResource' is not recognized ...
$ linix search ripgrep
psresource: `powershell` could not answer: Find-PSResource : The term 'Find-PSResource' is not r
```

Note the truncated error — the message is cut mid-word.

**The codebase already knows the right pattern.** `src/backends/krew.rs:43` probes *both*
`kubectl` and `kubectl-krew`. `psresource` is the sibling that was missed.

**Why it survived.** The native sweep exempts psresource from a real lifecycle —
`soft psresource: no real lifecycle here — writes to the PowerShell module path for the whole
user profile — plan-smoked instead`. That exemption is defensible for *mutation*. But nothing
forced the exemption to extend to the **read-only** question, and it did: no gate ever asks
whether `check health`'s verdict for a backend matches what that backend can actually do.

That check is cheap and general, and it would have caught this: for every backend reporting
`[READY]`, run its cheapest real read (`list`) and assert it succeeds. A backend that claims
health and cannot answer `list` is lying, whatever the reason.

### 4.3 `pixi global upgrade-all` was removed upstream

`src/backends/registry.rs:1381` sets `upgrade_args = ["global", "upgrade-all"]`. Against the
installed pixi 0.73.0:

```
Error: × `pixi global upgrade-all` has been removed
  ╰─> You can call `pixi global update` for most use cases
```

The pixi upgrade path is dead. Install/uninstall/list argv were checked against `pixi global
--help` and are correct.

### 4.4 `linix info` takes 98 seconds to return a wrong answer

```
$ time linix info cargo:ripgrep
Package 'cargo:ripgrep' not found in any available backend.
real  1m37.669s
```

`ripgrep` is on crates.io; `cargo search ripgrep` returns it instantly; and **`linix search
ripgrep` in the same tree returns `cargo  ripgrep  15.2.0`**. So two commands in one program
contradict each other, and the wrong one takes a minute and a half. The explicit `cargo:`
qualifier does not appear to narrow the probe.

`linix search ripgrep` takes **2m41s**. That is defensible for a deliberate cross-backend
search; 98 seconds for a single qualified `info` lookup is not.

### 4.5 Two search parsers emit junk rows

From one `linix search ripgrep`:

```
pixi   -                pixi   ...              pixi   14.0.3     (19 junk rows)
choco  5    packages    choco  Chocolatey  v2.7.3
```

- **pixi** is assigned `names_only` (`src/parsers/ecosystem.rs:60`), documented as *"search prints
  bare identifiers (opam `--short`, spack `list`, pixi `search`, emerge …)"*. Real `pixi search`
  output is a **detail record**, not a name list — `Name`/`Version`/`Build`/`Size` rows plus a
  build table. `names_only` takes the first token of each line, producing separators, ellipses and
  bare version numbers as package names.
- **choco** parses its own summary line `5 packages found.` into a package named `5` at version
  `packages`, and its banner into `Chocolatey v2.7.3`.

`names_only` **is tested** — `names_only_skips_headers_and_noise` — with a *spack* fixture. The
test passes. It says nothing about the other four managers routed through the same function.

### 4.6 One condition, two message families, and both name the wrong program

`src/core/manager.rs:40` (default trait impl) and `src/backends/generic.rs:219-236` both render
"this backend's program is missing", differently, and a user sees both in one screen:

```
[FAIL] cabal — `cabal` is not on PATH, so the `cabal` backend cannot run     <- generic.rs
[FAIL] snap  — Binary for snap not found in PATH                             <- manager.rs
```

`generic.rs` has the better version (it distinguishes an absolute path from a PATH lookup).
The `manager.rs` default, used by the ~13 backends that implement `BackendCore` directly, also
**names the backend rather than the binary it actually probed**:

| backend | message says | actually probes |
|---|---|---|
| `lvm` | `Binary for lvm not found in PATH` | `lvs` |
| `xbps` | `Binary for xbps not found in PATH` | `xbps-install` |
| `krew` | `Binary for krew not found in PATH` | `kubectl` **and** `kubectl-krew` |
| `appimage` | `Binary for appimage not found in PATH` | nothing — it is a `cfg!(target_os)` gate |

Deleting the `manager.rs` default in favour of `generic.rs`'s, parameterised by the probed
program, fixes the whole family.

### 4.7 `check health` calls a normal machine "23 critical"

On an ordinary Windows box, `linix check health` opens with:

```
Backends: 25 OK, 0 degraded, 23 critical (of 48 total).
```

Nothing is wrong. Those 23 are package managers the user does not have installed. Calling
brew-is-absent-on-Windows *critical* is a fail-loud principle applied where there is no failure,
and it is the first thing a new user sees.

The aggregate `linix check` gets this right — it prints `ok health 25 backend(s) ready` — so the
rollup and the detail view of the same section disagree about whether the machine is healthy.

### 4.8 Two prompts lack the non-interactive refusal their siblings have

The established pattern (`src/verbs/cleanup.rs:338` for `reset`, `src/verbs/sync.rs:305`,
`src/app/apply/dotfiles.rs:76`, `src/app/apply/bootstrap.rs:92`) is: check `is_terminal`, and
`bail!` with an actionable message. Two prompts skip it:

- `src/verbs/cleanup.rs:262` — **`purge-unmanaged`**, the most destructive command in the
  program, 80 lines above the sibling that does it correctly.
- `src/app/diagnostics.rs:147` — the auto-remediation confirm, which **installs packages**.

I measured what dialoguer 0.11 actually does on a closed stdin rather than assuming: it returns
`Err("IO error: not a terminal")`. **So both fail safe — nothing is removed and nothing hangs.**
The defect is that a scripted user gets a bare `IO error: not a terminal` instead of *"Refusing
to … in a non-interactive shell. Re-run with --yes, or --dry-run to preview."*

### 4.9 A typo exits `2`, which the readme defines as "found work to do"

`readme.md:708` publishes an exit-code contract and says *"The same four everywhere, so a script
can branch on them"*: `0` converged, `1` failed, `2` differences (a read-only command found work),
`3` refused. Measured:

| command | exit | documented meaning of that code |
|---|---|---|
| `linix check` (drift found) | 2 | differences ✅ |
| `linix check config` (clean) | 0 | converged ✅ |
| `linix nosuchcommand` | **2** | *"a read-only command looked and found work to do"* ❌ |
| `linix --nosuchflag` | **2** | ❌ |
| `linix sync --badflag` | **2** | ❌ |

Clap's convention for a usage error is exit 2, and clap exits before LiNix's `finish()` ever maps
a code — so the two contracts collide on the same number. **A CI job branching on the documented
table reads a typo in the command name as "the machine has drifted."** Since the whole stated
point of code `2` is unattended scripting, this defeats it.

Related, unresolved: `linix purge-unmanaged -y` refused by the unadopted-machine ratio check
exits **1**, not the documented **3**. The refusal is raised with `anyhow::bail!` rather than
`Error::Refused`, so it never reaches the `Exit::Refused` mapping. The native harness asserts
this case with `nok`, which accepts any non-zero code and therefore cannot tell `1` from `3`.
**I did not manage to produce a confirmed `3` cheaply**, so whether the protected-package path
returns it correctly is untested here — the mapping exists in `main.rs`, but that is a code read,
not a measurement.

### 4.10 Smaller items

- **First-run dead end.** `linix sync` with no config explains the `priority` file format and
  asks the user to write it by hand. It never mentions `linix init`, which exists to do exactly
  that. This is the first command a new user runs.
- **`init` promises a starter module it does not create.** `linix init --help`: *"…and a starter
  module."* `modules/` is created empty.
- **Stale exemption in the coverage audit.** Both harnesses list `undo` as an exempt subcommand
  with a reason. `linix undo` does not exist — it was renamed (`src/app/undo.rs` →
  `snapshot_restore.rs`). The audit iterates over `--help`, so this is a reporting error rather
  than a coverage hole, but it means the printed `5 exempt` is wrong and the exempt list is
  unvalidated. `harness-logic-test.sh` verifies that every *invoked* subcommand exists; it should
  do the same for every *exempted* one.
- **Failure reported three times.** One failed install prints the same sentence as a `WARN` from
  `linix::core::journal`, an `ERROR` from `linix::core::transaction`, and an `Error:` — two of
  them exposing module paths, the word "WAL", "Node", and a 32-hex operation id.
- **Register count drift.** `docs/SPEC.md:16` says *"All 107 decisions. 105 answered, 2 parked."*
  `docs/spec/decisions.md:64` says *"All 104 are ruled: 102 ANSWERED, 2 PARKED, 0 OPEN."* The
  register is the authority; the map is stale.
- **ID namespace collision.** `PRODUCTION-READINESS-REVIEW.md` uses `U1`/`U2`/`U3` for its own
  findings. `CLAUDE.md` makes `U*` a register ID requiring an owner ruling, and the register's
  real `U1` is *"where does a custom backend definition live"* (ruled 2026-07-23). Renaming the
  review's labels costs nothing and removes a live trap.

---

## 5. The lesson: what these defects have in common

It would be easy to read §3 and §4 as a long list and conclude the codebase is weak. That is not
what the evidence says, and getting this diagnosis right matters more than any individual fix.

### 5.1 The defects are not distributed randomly

Every single one is on a path that has **never been executed against the real tool it drives.**

| defect | path | ever run for real? |
|---|---|---|
| scoop exit code lost (4.1) | Windows `cmd` shim | never — the `.ps1` branch was the tested one |
| psresource health lies (4.2) | psresource | never |
| `pixi global upgrade-all` (4.3) | pixi upgrade | never |
| `info` slow and wrong (4.4) | cross-backend probe | never |
| pixi / choco search junk (4.5) | those parsers | never |
| `go` / `nimble` `list` blind to what was installed (3.4) | those `list` paths | **never — until this run** |
| `helm` / `luarocks` install defects (3.4) | those install paths | **never — until this run** |
| `psresource` health lie (4.2) | psresource | never — and the sweep *exempts* it |

The last three rows are the strongest evidence, because they are not a code read: they are what
happened the first time anyone drove those backends end to end on this machine. **Running the
native sweep to completion — something no automated gate does — turned up four hard failures and
two more real defects in a single pass.** That is the yield of the untested remainder, measured
rather than argued.

**One correction worth recording, because it is the same mistake this document warns about.** My
first reading of the `pub` and `nimble` failures was that `list` had invented a package and that
an install had silently done nothing. Both were wrong. I had inspected the machine *after* the
sweep, and the harness uninstalls each package immediately after listing it — so the empty
directories I found were the harness's cleanup, not evidence. The correct findings are narrower
(`go`'s `list` is genuinely blind; `pub` is a `PATH` usability gap; `nimble` is unexplained).
**Checking a claim against a system in the wrong state is exactly how an audit manufactures a
finding, in the same way a harness manufactures a pass.**

The spec states the number and is honest about it: **52 backends registered, 22 have ever run
against a real package manager, 45 "plan-smoked."** The Ubuntu run in §2 corroborates the ratio
independently — it reported **7 real lifecycles against 49 plan-smokes**. A plan-smoke proves the
argv is *constructed*. It cannot prove the argv is *correct*. `pixi global upgrade-all` passes a
plan-smoke perfectly; it just does not exist anymore.

The same gap exists one level down, in the parsers. `names_only` serves five managers and is
tested against one fixture, from spack. The test passes and says nothing about pixi — which is
exactly where it is wrong.

**So the bug rate in covered code is low, the bug rate in uncovered code is high, and uncovered
code is most of the surface.** "More bugs keep appearing" is the arithmetic of that, not evidence
of decay. It also means the defects in §4 should be read as a *sample*, not an inventory. I
stopped looking because I had characterised the class, not because I ran out.

### 5.2 The parts that are covered are genuinely good

This is not faint praise, and it is the reason the project is worth finishing:

- **The removal guard** (`src/app/sync/guard.rs`) is the most careful code in the repo. I
  enumerated the removal paths from the code rather than from any list, and every one routes
  through it — including the recovery paths where it was historically missed. `heal` guards its
  interrupted removals and resolves the journal entry so it cannot get stuck
  (`src/app/sync/mod.rs:684`). Transaction rollback refuses to remove a package whose prior state
  is `Unknown`, on the stated grounds that not knowing is not permission
  (`src/core/transaction.rs:726`).
- **The confirmation discipline** is right in six of eight places, with a stated rule and
  actionable refusals.
- **The executor rewrite** that fixed the pty defect is correct: both pipes are drained
  concurrently with the wait, so a child that outruns the buffer cannot deadlock, and the
  read/mutate layer split means a read can never take the terminal.
- **1,359 tests pass. Clippy is clean. Four container lifecycles pass 1,127 checks with zero
  failures.**
- `--verbose` — reported dead in the prior review — is **fixed and its help text is accurate**
  (`-v` → info, `-vv` → debug), verified by running it. **Re-verified.**

### 5.3 The secondary pattern: several checkers check the wrong thing

Cheap to fix, and it is what keeps everything in §5.1 invisible:

| checker | checks | should check |
|---|---|---|
| `psresource::is_available` | PowerShell exists | PSResourceGet is importable |
| default `check_health` message | the backend's name | the program actually probed |
| harness "name out of manifest" | its own `grep -v` | the product's behaviour |
| harness coverage audit | invoked commands exist | exempted commands exist too |
| `release-check.ps1` | fmt, informationally | fmt, the way CI does |

Each of these reports success about something it never examined. Together they are why a tree
with 26 formatting violations, a wedging bug, and a dead pixi upgrade path presents as green.

### 5.4 What this means for "is the code worthless"

No. Concretely:

- **The architecture is sound and the risk is correctly placed.** The code that could destroy a
  machine is the code that was written most carefully, and it holds up under a from-the-code
  audit rather than a from-the-list one.
- **Almost every defect is shallow** — a wrong argv string, a probe of the wrong program, a
  parser eating a header line, a missing `is_terminal`. Hours each. None requires changing the
  model, and none is in the declarative engine.
- **The one structurally interesting bug** (§3.1) is a localized ordering fix plus an owner
  ruling that was never recorded.

The problem is a **breadth-versus-depth mismatch**, not a quality problem. LiNix shipped roughly
52 × 61 backend-command pairs and built real verification for about a tenth of it. That is a
scoping decision, and it is reversible.

---

## 6. Recommended order of work

**Correctness of the core promise — these come first, because they break `sync` itself:**

1. **`go`'s `list` cannot see a package that is genuinely installed and on `PATH`** (E6). `sync`
   compares desired against `list`, so this produces permanent phantom drift.
2. **Investigate `nimble`'s two hard failures** (E6b) — no binary produced and `list` blind. Root
   cause is open; do not guess it.
3. Add an install → `list` → binary round-trip for the whole per-user-bin family, **asserted while
   the package is still installed.** `cargo` already passes it.
4. Make LiNix say so when a backend's bin directory is not on `PATH` (E6c) — one line, and it
   turns a silently useless install into an actionable message.

**Then the config-wedging and gate failures:**

5. Fix §3.1 — widen the withdrawal condition in `src/verbs/packages.rs:111` from
   `Error::Unresolvable` alone to `Unresolvable || Retryability::Permanent`. The classification
   already exists and is already computed; it is one condition, not a redesign. Then make the
   error name `modules/imperative.txt` and suggest `linix unmanage <name>` for whatever is
   deliberately left behind. Record the harnesses' disagreement as a ruling in `decisions.md`.
6. Delete the `grep -v` scrub in both harnesses (§3.2) and let the assertion fail. Replace the
   `install failed → soft` catch-all (§3.4) with a classification on `Retryability`. Extend
   `harness-logic-test.sh` to cover inline assertions of this shape.
7. **Run the native sweep in CI.** It found every hard failure in this assessment, and no
   automated gate executes it on any platform.
8. Make `cargo fmt --check` hard in `release-check.ps1` and `release-check.sh`; run `cargo fmt`;
   **push the 10 unpushed commits and get a CI verdict on them** (§3.3).
9. Fix §4.1 — prefer the `.ps1` shim over `.cmd` on Windows so scoop's exit code survives, and
   stop deciding an entire backend's success on one marker string.
10. Fix §4.2 by probing the real dependency, and audit every other `is_available` against the
    `krew` pattern. Add the cheap general check: every `[READY]` backend must answer `list`.

**Then, and this is the one that changes the trajectory:**

11. **Change what "supported" means.** A backend is *supported* when it has passed a real
    install → list → remove round-trip verified against the manager's own view; everything else
    is *experimental* and says so in `check health`, in `priority`, and in the readme. This turns
    an unbounded invisible-bug surface into an honest, bounded claim, and it is the only item here
    that stops §5.1 from regenerating.
12. Stop adding backends until (11) is done. `linix init` currently writes 23 managers into
    `priority` on a fresh Windows box; most have never been run.

**Quality of life, in rough value order:** §4.4 (`info` — 98s and wrong), §4.7 ("23 critical"),
§4.10 first-run dead end, §4.6 (message family), §4.5 (parsers), §4.8 (prompts), §4.9 (exit codes), §4.3 (pixi).

---

## 7. Answers to the questions asked

**Is it production ready?** No.

**How far?** About 70%. The engine, the safety model and the grammar are close to done. The
backend surface is roughly a tenth validated, and the harnesses currently overstate it.

**Does it work for a human, not just a bot?** Partly, and the gap is real. The documented
quickstart works exactly as written — I ran it verbatim and `check` reported `0 present` then
`1 present` at precisely the points the readme says it will. But the paths a *human* takes and a
CI job does not are where the damage is: a first `sync` that does not mention `init`; a typo that
wedges the config with no way out named; `info` taking 98 seconds to give a wrong answer;
`23 critical` on a healthy machine; raw PowerShell stack traces in `upgrade` output; one failure
printed three times with a WAL id in it. CI exercises none of these, and passes.

---

## 8. Grade

**Overall: C+.** Not the B+ the architecture argues for. One number hides the story, so:

| area | grade | why |
|---|---|---|
| Core engine & safety model | **A−** | The removal guard holds up under a from-the-code audit, including recovery paths. Rollback refuses to delete when prior state is `Unknown`. The executor's pty fix is correct and drains both pipes concurrently. |
| Declarative model & grammar | **B+** | The documented quickstart works verbatim. `eval`, `check`, `plan` are coherent and the refusals are legible. |
| Backend layer | **D** | The mass of the product, and mostly unverified. Every defect in §3–§4 lives here. |
| Test & gate infrastructure | **D−** | The lowest grade, and deliberately — see below. |
| Documentation vs. reality | **B−** | Unusually honest about its own gaps, with drift at the edges (E29, E31, E32). |

**Why the backend layer is a D and not lower.** The failures are shallow and localized — a wrong
argv string, a probe of the wrong program, a parser eating a header line. But there are many of
them, and `go`'s `list` being blind to a package that is installed and on `PATH` is not cosmetic.
For a tool whose whole job is "make the machine match the file", a `list` that disagrees with the
machine breaks the one thing it promises.

**Why the test infrastructure is a D−, which is the substantive judgement here.** Not for being
thin. For **reporting success about things it never examined** — §3.2, §3.4, §3.3 respectively:
an assertion that deletes its own evidence, a catch-all that skips a backend's whole lifecycle
when that backend is broken, and a ship gate weaker than CI. Each of those is worse than having
no check at all, because a missing check is visible and a lying check is not.

**Architecture did not earn a pass.** The design is good and the design is not the problem. The
distance from C+ to B+ is roughly two weeks of unglamorous work, none of it in the engine.

### 8.1 The rubric — what each grade requires

This exists so the next assessment grades the same way, and so the work has a bar to clear rather
than a vibe to satisfy.

| grade | bar |
|---|---|
| **C** | Builds, tests green, core safety paths correct. Backend breadth largely unverified. **(Where it was before this assessment thought it was a B+.)** |
| **B** | Every blocker in §3 fixed. No check in any harness can pass without examining the thing it names. Local gates match CI exactly. The native sweep runs in CI on Windows and macOS. |
| **A** | Every backend advertised as *supported* has passed a real install → `list` → binary → remove round-trip, asserted while installed, in an automated gate. Everything else is labelled *experimental* in `check health`, `priority` and the readme. Every parser is tested against a fixture captured from its own tool, including the empty, single, not-found and error cases. Every `[READY]` backend can answer `list`. |
| **A+** | All of A, plus: an argv-drift gate that fails when an upstream manager removes a subcommand LiNix invokes (E13's class); pty coverage on every read command, not just `list`; the destructive effectors (btrfs/zfs/lvm restore, `dpkg -i`/`rpm -U`, U30 storage removal) exercised in disposable VMs; a mid-transaction `SIGKILL` → `heal` test at every step boundary; latency budgets enforced per command class (E14 would have failed one); and property/model-based tests over the declarative core. **No user-visible failure without a file or command to act on.** |

The honest reading of that table: **A is reachable with the work in `docs/BUILDER.md`. A+
additionally requires test kinds this repo does not have yet**, all of them specified in
`docs/GRADER.md` §5–§6.

### 8.2 How the three documents fit together

They are a loop, and they are meant to be run in order:

1. **`READINESS-2026-07-27.md`** (this file) — *what is wrong.* 36 indexed defects with
   reproductions, plus the diagnosis in §5 and the rubric above.
2. **`docs/BUILDER.md`** — *what to build.* A work order per defect: root cause, the
   prescribed fix, the failing test to write first, the sibling sites, and the acceptance
   criterion. This is the document to hand to the AI that writes code.
3. **`docs/GRADER.md`** — *how to check it, adversarially.* Hand this to a **different** agent,
   with no sight of the remediation work, and have it grade against §8.1. If it cannot reproduce
   the fix's acceptance criterion independently, the fix is not done.

Do not let the same agent do (2) and (3). The failure this whole assessment is about is a system
grading its own homework.
