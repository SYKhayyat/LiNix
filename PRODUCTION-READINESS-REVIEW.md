# LiNix — Production Readiness Review & Work Order

## STATUS — worked 2026-07-27. An audit nobody retires becomes the next thing nobody believes.

| item | outcome |
|---|---|
| **C1** pty test in CI | **DONE** — `tests/pty_tests.rs`, named step in CI's fast half. Watched failing against the old behaviour in WSL first. |
| **B1** child stdout/stderr | **DONE** — S42, ruled U40, rule II.12c, reason V.84. Mutations still show progress: output is mirrored to stderr as it arrives. |
| **B2** pagers | **DONE** — S43. Suppressed on the env map every spawn inherits; `--no-pager` on every systemd row and scheduler call. |
| **B3** rollback + guard | **DONE** — S45, ruled U41, rule II.10, reason V.85. All three parts, plus the `Remove` arm's lost pin. |
| **B4** orphaned children | **DONE** — `kill_on_drop` on the spawn, in the S42 commit. |
| **H1** exec lock | **DONE** — moved to the data directory with `datalock.rs`'s treatment. |
| **H2** repo removal | **DONE** — S44, **and a second fault it uncovered**: apt and apk named a program in the argument position, so `repo add/remove/list` had never worked on either. |
| **H3** sudo keepalive | **DONE** — `tokio::process`, `-n`, stdin null, behind a guard that aborts on drop. |
| **H4** version | **HALF WRONG, HALF OPEN.** No hardcoded `v6.0.0` exists in `src/` — `lib.rs` derives `VERSION` from `CARGO_PKG_VERSION`; the banner this review saw came from the stale July binary on disk. `Cargo.toml`'s `version = "0.1.0"` is real and is **the owner's number to pick**. |
| §7 instability | **DONE** — S46 (two `.expect()`ed spawns on the dry-run path), S47 (`attempt` renamed to `retries`), and **uniform retry semantics, which this table said was not attempted**: `Retryability` on `Error`, `ExitPolicy` per backend, and a retry loop that stops on a permanent failure. A held dpkg lock retries; a name no repository carries is reported at once. |
| §6 SOLID | **DONE, three of four at the mechanism and one verified already closed.** *Stringly-typed errors:* `CommandFailed` carries a classification; `Error::retryability()` covers all 23 variants; `ratelimiter.rs`'s `contains("429")` — the one live substring branch — reads the variant now. *Open/Closed:* `is_benign_exit`/`output_signals_failure` are gone from the core executor; nine backends declare an `ExitPolicy` at registration. *Liskov (`RepoManager`):* already closed by **S44** — `remove_repo` resolves the URL from the manager's own listing and `reject_unsubstituted` makes silent degradation a hard error; re-verified across all three implementors, not re-fixed. *SRP:* `context.rs` 1,921 → 665 lines across nine facet structs, each holding only what it uses; `App::restore_session_suspensions` deleted as a dead twin of `EphemeralShell`'s. |
| **U1**, **U2** | **OPEN — owner rulings, now with the measurements the review lacked.** U1's description is substantially wrong: the surface is **62 top-level entries, not 45**, and **6 of the 13 commands it names do not exist** (`remove`, `prune`, `orphans`, `clean`, `unmanaged`, `status`, `doctor`, `migrate`, `clone`, `generation`). Every command in the "removal" cluster does something the others do not. The one real overlap is `undo` / `history` / `rollback`. U2 is confirmed and has a third defect the review did not see: **`--verbose` is dead** — its help promises debug logging and it produces none, because the subscriber is built at `main.rs:41` before clap parses at `:81`. |
| U3 | **OPEN** — `status` is slow. Worth re-measuring now that B1 is fixed; not investigated this session. |

Verified after the work: `cargo build --all-targets` → `cargo test` → `cargo clippy
--all-targets --all-features -- -D warnings` clean on **Windows (1,340 tests) and Linux (1,339,
in WSL)**. The Linux run caught two faults the Windows run could not see.

---

> Review date: 2026-07-27 · Reviewer: Claude (Opus 5)
> Commit reviewed: `89bed26` (main, clean tree) · binary reports `linix 0.1.0`
> Build verified: `cargo build --release` on Linux (WSL/Ubuntu 26.04) — **exit 0, 0 warnings**
> Scope: 202 Rust files, 72,824 lines in `src/`, 1,324 tests.

---

## 0. How to use this document

This is written to be executed by an agent. Read this section before touching code.

**Binding repo rules that apply to every item below** (from `CLAUDE.md`):

- **Test-first.** "Bug fix = write the failing test first, watch it fail, then fix." Each work
  item below names the test to write. Do not fix first and test after.
- **Fix the whole family.** Each item has a **Siblings** subsection listing the parallel sites.
  A fix that patches only the reported line has not fixed the bug. When reporting, say which
  sibling sites you checked — *including the ones you decided were unaffected, and why*.
- **No legacy.** No compatibility shims, no dual code paths. When a thing is replaced the old
  thing is deleted in the same change, including its config keys, docs and tests.
- **Read `docs/SPEC.md` first**, and for any target-state rule you touch, read its `why.md` entry
  before changing it.
- **Verify:** `cargo build --all-targets` → `cargo test` → `cargo clippy --all-targets`.
  Unverified is not done.

**Register status, checked 2026-07-27:** `docs/spec/decisions.md` reports **104 ruled, 0 OPEN**.
I found **no registered decision covering child-process stdio, terminal detection, or pagers.**
The behaviour in B1/B2 is therefore *undocumented*, not *ruled* — fixing it contradicts no ruling,
but see the stop-and-ask table below.

### STOP AND ASK — do not implement these unilaterally

`CLAUDE.md` says stop for: (1) anything with a register ID, (2) anything that changes behaviour a
user would notice, (3) anything that removes a feature, (4) anything where Part II looks wrong.

| Item | Trigger | Why |
|---|---|---|
| **B1, B2** | (2) | Changes what appears on screen for every interactive user. My reading is that this *restores* intended behaviour rather than changing it — but the user-visible delta is large and there is no ruling to point at. **Get a ruling, then write the decisions.md entry in the same commit.** |
| **B3** (guard on rollback) | (2) | A rollback may now *refuse* to complete. That is a new refusal a user will see. Needs a ruling on what happens when a compensating removal is blocked. |
| **H4** (version) | (2) | Version numbering is owner territory. Do not pick a number. |
| **U1** (command surface) | (2)(3) | Consolidating commands removes commands. Do not do this without a ruling. |
| **U2** (log noise) | (2) | Changing default log level is user-visible. |
| B4, H1, H2, H3 | none | Internal correctness. **Build these without asking.** |

---

## 1. Verdict

**Not production ready.** One defect makes the tool substantially non-functional for every
interactive user while remaining completely invisible to CI, and it chains into a path that can
uninstall packages the user already had.

The code is well above average — disciplined error handling, comments that state constraints
rather than narrate, an exemplary download module, and a removal guard designed with real care.
The problem is not sloppiness. It is that **the entire verification strategy shares one blind
spot**: nothing in CI, in the container harnesses, or in the 1,324 tests ever runs LiNix attached
to a terminal. Every blocker below lives in that blind spot.

CI passing is not evidence against these findings. CI *cannot* observe them.

---

## 2. Reproduction harness

Everything below was verified by building current source on Linux and running the real binary
against real `apt`, twice: stdin piped (what CI does) and under `script(1)`, which allocates a pty
(what a human does).

`script -qec "<cmd>" /dev/null` is the whole trick — it is the cheapest way to give a child a real
TTY. Save as `/tmp/repro.sh` and run with `bash /tmp/repro.sh`:

```bash
#!/bin/bash
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HOME/lxbuild"
cd /path/to/linix && cargo build --release || exit 1
B="$HOME/lxbuild/release/linix"

export LINIX_CONFIG_DIR="$HOME/lxcfg" LINIX_DATA_DIR="$HOME/lxdata"
rm -rf "$HOME/lxcfg" "$HOME/lxdata"; mkdir -p "$HOME/lxcfg/groups" "$HOME/lxdata"

echo -n "piped, rows LiNix formatted: "
"$B" list -b apt </dev/null 2>/dev/null | grep -c '^apt '
echo -n "tty,   rows LiNix formatted: "
script -qec "$B list -b apt" /dev/null 2>/dev/null | grep -c '^apt '
```

**Expected before fix:** `piped 609`, `tty 1`. **Required after fix:** both non-zero and equal.

> Note for whoever runs this on Windows: WSL shuts down between separate `wsl --` invocations and
> **wipes `/tmp`**. Build to `$HOME`, and keep build+verify inside a single script. Also invoke via
> PowerShell, not Git Bash — Git Bash rewrites `/mnt/...` paths and the call fails.

---

## 3. Blockers

### B1 — Under a real terminal, LiNix discards the output of every command it runs

**Location:** `src/core/executor.rs:163-176`, in `RawExecutor::execute`.

**Current code:**

```rust
let mut command = Command::new(cmd);
command.args(args).envs(env);

if std::io::stdin().is_terminal() {
    command
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
} else {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
}
```

**The defect.** When stdin is a TTY the child's stdout is *inherited* — it goes to the user's
screen and `output.stdout` comes back **empty**. All **79 `run_output()` call sites** then parse an
empty string. `run_output` is how LiNix learns what is installed, at what version, what a search
returned, what a repo list contains.

**Evidence** (freshly built current source, same machine, same command):

| `linix list -b apt` | rows LiNix parsed |
|---|---|
| stdin piped (CI) | **609** |
| stdin a real TTY (human) | **1** |

It is deceptive rather than obviously broken: what the user sees is `dpkg-query`'s raw output
bleeding through the inherited handle, which looks superficially like a package list.

```
piped:  apt          adduser                          3.153ubuntu1     <- LiNix's format
tty:    adduser 3.153ubuntu1                                           <- dpkg's raw output
```

#### Why the obvious fix is wrong — read this before coding

"Just always pipe" **will break `sudo` password entry.** `src/core/executor.rs:443-446` prepends
`sudo` with no `-n`, and the non-TTY branch sets `stdin(null)`. Piping everything means sudo gets
`/dev/null` on stdin and fails with "no tty present" on every privileged operation. There is a
keepalive (`:730-740`) that tries to pre-warm the sudo timestamp, but it is best-effort and is
itself broken (see H3), so it cannot be relied on.

**The seam already exists — use it.** `CommandExecutor` already separates two layers:

- `reader` (`:363`) — reads and existence probes, reached via `read_raw` (`:429`) →
  `run_output`, `search_output`, `command_exists`. **Never needs stdin. Must always pipe.**
- `inner` (`:357`) — mutations, reached via `run_raw` (`:424`) → `run`, `run_exclusive`.

Both currently route into the same `RawExecutor::execute`, which is why one TTY check poisons both.

**Also relevant:** every mutating backend already runs non-interactively — `install_args` carry
`-y` / `--noconfirm` throughout `src/backends/registry.rs` (`:200-201`, `:298-299`, `:436-437`,
`:616-617`, `:941`, …). So package managers do **not** need an inherited TTY to avoid a
confirmation prompt. `sudo` is the only genuine consumer of inherited stdin.

**Fix design.** Make capture-vs-inherit an explicit property of the call, not an ambient property
of the process's stdin. Concretely: stdout and stderr are **always piped**; only **stdin** may be
inherited, and only on the mutating path. That keeps every parser fed while leaving sudo able to
read a password from the terminal. Note sudo writes its prompt to `/dev/tty` directly when one
exists, so the prompt still reaches the user with stderr piped — **verify this by hand** on a host
where the sudo timestamp has expired; it is the one behaviour a test cannot easily cover.

Do not add a config key or env var to switch this. Per `CLAUDE.md`, one path, no dual code paths.

**Failing test to write first.** There is no pty test in the suite today; this is the gap that hid
B1, B2 and the non-determinism. Add an integration test that runs the built binary under a pty and
asserts LiNix-formatted rows are present. Rust has no pty in std — either shell out to `script -qec`
(present on Linux CI images) or add a dev-dependency. Then **add the same check to
`.github/workflows/ci.yml`**, which is item **C1** below and is the highest-leverage line in this
document.

**Siblings to check in the same change:**
- `src/core/executor.rs:166` — the branch itself.
- Anything that reasons *about* the branch: `output_signals_failure` (`:570`) and its doc comment
  both say scoop's marker is "only consulted when output is piped (non-interactive)". Once output
  is always piped that caveat is stale and the comment must be corrected, not left to mislead.
- `DryRunExecutor::execute` (`:228`) and `MockExecutor::execute` (`:322`) do not spawn and are
  unaffected — state that explicitly in the report rather than silently skipping them.
- The other `is_terminal()` call sites (`src/app/context.rs:383,1027`, `src/verbs/*`,
  `src/core/journal.rs:232`, `src/utils/style.rs:14`) are about *LiNix's own* prompting and
  colour, not child stdio. They are **correct as-is** — do not "fix" them.

### B2 — A child process can capture the terminal and hang the tool

Same root cause as B1, separate consequence, separate fix.

**The defect.** Because stdout is inherited, children detect a TTY and turn on interactive
behaviour. `linix status` invokes `systemctl`, which pipes itself into a pager. Captured verbatim
from a real run:

```
^[[?1h^=                                   <- alternate-screen / pager init
  chrony.service   loaded active running ...
^[[7mlines 1-16/16 (END)^[[27m             <- pager waiting for a keypress
Session terminated, killing shell... ...killed.
```

`linix status` — a **read-only** command — hung waiting for a keypress and had to be killed.

**Consequence: output is non-deterministic.** Three identical `linix status` runs under a TTY
produced **80, 640 and 83 lines**. Piped: 631, 630, 630.

**Fix.** B1 removes the trigger (a piped child does not start a pager), but do not rely on that
alone — a pager can still be forced by `$PAGER`/`$SYSTEMD_PAGER` in the user's environment. Suppress
it explicitly and belt-and-braces:

- Pass `--no-pager` to `systemctl` invocations.
- Set `SYSTEMD_PAGER=`, `PAGER=cat`, `GIT_PAGER=cat` in the env map built in
  `CommandExecutor::run_on` (`src/core/executor.rs:452-457`) — that map is already the one place
  every spawn inherits from, which makes it the correct single site.

**There is not one `--no-pager` in the codebase** (verified by grep).

**Siblings — every pager-capable child:**
- `systemctl`: `src/app/scheduler/mod.rs:146, 150, 166, 170, 186, 193, 220`;
  `src/backends/service.rs:442`.
- `git` (pages `log`/`diff`/`show` by default): `src/core/git.rs:100, 135`;
  `src/app/insight.rs:619`.
- Check also `journalctl` and `dnf` if either is reachable — I did not find call sites, but confirm
  and say so.

**Failing test to write first.** Assert the env map produced by `run_on` contains the pager
suppressors. That is a cheap unit test and it pins the whole family at once.

### B3 — Rollback removes packages the user already had, and bypasses the removal guard

**Location:** `src/core/transaction.rs:518-582` (`Transaction::rollback`).

**The defect, part 1 — an upgrade is compensated by an uninstall.** Rollback compensates a
`GraphAction::Install` by calling `h.remove(...)`. That is correct only if the package was absent
before the transaction. It is not always absent: `needs_change()`
(`src/app/sync/planner.rs:536-541`) returns `true` for a **version or channel change** on an
*already-installed* package, which schedules an `Install` node. If a later node fails, rollback
runs `remove()` on it — so **a failed upgrade uninstalls the package entirely** instead of
reverting it to the prior version.

**Part 2 — it fails open, which chains with B1.** `needs_change()` at
`src/app/sync/planner.rs:526-528`:

```rust
let installed = match q.info(&spec.name).await {
    Ok(Some(p)) => p,
    _ => return Ok(true),          // <- "if I can't tell, assume it needs installing"
};
```

Under B1, `info()` returns nothing for everything, so **every managed package gets an Install
node**. Each `apt install <already-present>` succeeds trivially and lands in `self.history`. One
failure anywhere then triggers rollback, which issues `remove()` across the whole history. That is
a **mass-uninstall path reachable from an ordinary interactive `sync`**.

**Part 3 — the guard does not cover it.** `src/core/transaction.rs` contains **zero** references to
the guard (verified by grep). `guard::enforce` runs at plan time over the planner's `Remove` nodes
(`src/app/sync/mod.rs:141`). Rollback's compensating removals are issued at execution time and never
pass through it, so `protected_packages` and OS-essential protection do not apply to them.

This is a direct violation of the project's own binding rule:

> Every path that removes calls the guard (`app/sync/guard.rs`) … A guard on one command is a
> guard on nothing.

**Fix design.**
1. Record pre-transaction presence (and prior version) per node when the graph is built, so
   rollback can distinguish *install* from *upgrade*. Compensate an upgrade by restoring the prior
   version, not by removing.
2. Route rollback's removals through `guard::protection_of` / `guard::enforce`. **This needs a
   ruling first** (see stop-and-ask table): a guard that refuses a compensating removal leaves the
   transaction partially applied, and what LiNix should then tell the user is a product decision,
   not an implementation detail.
3. Stop failing open in `needs_change()`. An `info()` that *errored* is not the same as one that
   returned "absent" — the distinction already exists in `search_output`'s design
   (`src/core/executor.rs:486-501`), which deliberately separates "no result" from "could not
   answer". Apply the same reasoning here; that function is the model to copy.

**Failing test to write first.** Build a two-node graph where node 1 is an upgrade of an
already-installed package and node 2 fails; assert the package is still installed at its prior
version afterwards. Extend it to: a protected package in the history, and an `info()` that errors
(must not be read as "absent").

**Siblings:** the two `GraphAction` arms in `rollback` are twins — fix both, not just `Install`.
The `Remove` arm reinstalls **without a version** (`:553-559`, `options: HashMap::new()`), so a
rolled-back removal silently loses a pin. Same family, same change.

**Verification caveat — read before reporting this as confirmed.** Parts 1 and 3 are confirmed by
code inspection and grep. **Part 2's mass-uninstall chain I traced but did not trigger** — I was
not willing to run a failing `sync` against a live system. Confirm it in a throwaway container
before treating the chain as proven.

### B4 — Aborted workers leave package managers running

**Location:** `src/core/transaction.rs:176` and `:266`.

Both call `worker_pool.abort_all()` before rolling back. Aborting a tokio task drops the future; it
does **not** kill the spawned child. `kill_on_drop` is set **nowhere in the codebase** (verified by
grep).

So on failure or global timeout, in-flight `apt install` / `dnf install` processes keep running,
orphaned, while `rollback()` concurrently issues `apt remove` against the same dpkg lock. Those
orphans are also absent from `self.history`, so whatever they complete is never compensated.

**Fix.** Set `.kill_on_drop(true)` on the spawn in `RawExecutor::execute`, **or** track children
and reap them explicitly on abort. Prefer whichever leaves one path, not two.

Be aware killing a package manager mid-write can leave dpkg/rpm needing `--configure -a`. If that
tradeoff is judged worse than the orphan, the alternative is to await in-flight nodes before
rolling back rather than aborting them. **Either is defensible; pick one, and put the reasoning in
the commit message** — this is explicitly a "make the call" case under `CLAUDE.md`, not a
stop-and-ask.

**Failing test to write first.** Assert a spawned child does not outlive an aborted task.

**Siblings:** both `abort_all()` sites, and the global-timeout path at `:138-159` which cancels via
the token and has the same orphan exposure.

---

## 4. High severity

### H1 — Predictable lock file in shared `/tmp` (truncation / DoS)

**Location:** `src/core/executor.rs:521-522`.

```rust
let lock_path = std::env::temp_dir().join(format!("linix_{}.lock", lock_key));
let lock_file = File::create(lock_path).map_err(Error::from)?;
```

Fixed, guessable name in a world-writable directory, opened with `File::create` — which
**truncates** and **follows symlinks**. On a shared host an attacker pre-creates
`/tmp/linix_apt.lock` as a symlink; the next `run_exclusive` truncates the target. Because LiNix is
designed to run under `sudo`, that truncation can occur as root. The non-malicious variant is plain
DoS: another user's `0600` file at that path makes every exclusive operation fail.

**This is a sibling bug the project already solved.** `src/core/datalock.rs` does it correctly —
lock in the data directory, `OpenOptions` with `truncate(false)`, a separate owner-stamp file, and
a documented rationale for why the lock file is never deleted. **Copy that approach**; do not
invent a third locking style.

**Failing test to write first.** Create a symlink at the lock path pointing to a canary file; assert
the canary still has its contents after `run_exclusive`.

**Siblings — every fixed-name `temp_dir()` path:** `src/app/bundle.rs:368, 403, 459` and
`src/model/vars_embedded.rs:303, 378` all interpolate `std::process::id()`, so they are
per-process and **not** vulnerable to the same pre-planting — confirm and say so rather than
changing them. `src/core/executor.rs:521` is the only one with a fixed shared name, and that is
precisely because it needs to be shared to function as a cross-process lock — which is why it needs
`datalock.rs`'s treatment rather than a PID.

### H2 — `apk` and `gem` repo removal silently do nothing

**Location:** `src/backends/generic.rs:658-661`.

```rust
let final_args: Vec<String> = base_args
    .iter()
    .map(|a| a.replace("{name}", name))     // <- {url} is never substituted
    .collect();
```

`remove_repo(&self, name, sudo)` has no `url` parameter, but two templates in
`src/backends/registry.rs` require one:

- `:386` (apk) — `sed -i '\|{url}|d' /etc/apk/repositories`
- `:800` (gem) — `["sources", "-r", "{url}"]`

The apk case is the worse one: `sed` searches for the **literal string `{url}`**, matches nothing,
and **exits 0**. `run()` enforces exit status, sees success, and reports the repository removed. It
was not. The gem case passes the literal `{url}` as an argument.

Root cause is an interface defect: the `RepoManager::remove_repo` signature cannot express what
these backends need, and the template system degrades silently instead of failing.

**Fix design.** Two parts, both required:
1. Give `remove_repo` the information it needs (pass the URL, or resolve it from `list_repos`
   before removal).
2. **Make an unsubstituted placeholder a hard error.** After substitution, if any arg still matches
   `\{[a-z_]+\}`, refuse. This is the part that turns the whole class into a loud failure instead of
   a silent one — without it, the next template with a new placeholder repeats the bug.

**Failing test to write first.** Assert apk repo removal actually rewrites `/etc/apk/repositories`
(mock executor: assert the argv contains the real URL and no `{`). Then assert that a template with
an unsubstituted placeholder is refused.

**Siblings:** `add_repo` (`:631-649`) substitutes both `{name}` and `{url}`, so gem/apk *add*
works — check it and say so. Sweep `registry.rs` for every `{...}` placeholder in every `*_args`
vector and confirm each has a substituting caller: `{name}` appears at `:245, 455, 524, 580, 646,
457`, `{url}` at `:386, 799, 800`. The placeholder-guard in part 2 covers any I missed, which is
why it is not optional.

### H3 — `sudo` keepalive blocks the async runtime and cannot be stopped

**Location:** `src/core/executor.rs:730-740`.

```rust
Some(tokio::spawn(async move {
    loop {
        let _ = StdCommand::new("sudo").arg("-v").status();   // blocking, in async
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}))
```

Three defects: `StdCommand::status()` is a **blocking** call on a runtime worker thread (use
`tokio::process` or `spawn_blocking`); the loop has **no cancellation** despite returning a
`JoinHandle` typed as if it were controllable; and `.status()` **inherits stdin**, so an expired
sudo timestamp makes this background task compete with the foreground process for the terminal's
password prompt.

Note this interacts with B1 — do not fix them in ignorance of each other. Fix B1 first.

**Failing test to write first.** Assert the returned handle actually stops the loop when aborted.

### H4 — Version regressed from 6.0.0 to 0.1.0

`Cargo.toml:3` is `version = "0.1.0"`; `src/cli/args.rs:9` reports `env!("CARGO_PKG_VERSION")`.
`CHANGELOG.md` records a released `[6.0.0] — 2026-07-02` and current work as `[Unreleased] — v7`.
The release job in `.github/workflows/ci.yml` publishes on `v*` tags.

A v7 artifact self-identifying as `0.1.0` breaks upgrade checks, semver consumers, and bug triage —
every report will cite a version that never existed. The stale July binary on disk still reports
`6.0.0`, confirming the regression is recent.

**Do not pick a number** (stop-and-ask). Also note the runtime banner in `src/app/context.rs` still
hardcodes `v6.0.0` independently of `CARGO_PKG_VERSION` — that is a second source of truth and
should be derived from the first, which *is* an implementation detail you may fix without asking.

---

## 5. CI — the gap that hid all of the above

### C1 — Add one pty test. Highest-leverage item in this document.

`.github/workflows/ci.yml` runs builds, tests, `clippy -D warnings`, and four container lifecycles
across three OSes. All are non-interactive. **Nothing exercises a pty.** A single job doing what
§2's harness does — run `linix list` under `script -qec` and assert LiNix-formatted rows — would
have caught B1, B2 and the non-determinism.

Add it to the fast half that runs on every push, not the nightly half.

---

## 6. SOLID violations

**Single Responsibility — `App` is a god object.** `src/app/context.rs` is 1,921 lines exposing 45+
public methods spanning firewall rules, dotfile trees, lease expiry, git autocommit, snapshot
pruning, schedules, bootstrap offers, exec plans, search and package listing. Nothing can be tested
in isolation because everything hangs off one struct. `src/backends/registry.rs` (2,091 lines) and
`src/config/grammar/statement.rs` (2,445) are the same shape.

**Open/Closed — backend quirks hardcoded in the core executor.** `is_benign_exit` and
`output_signals_failure` (`src/core/executor.rs:570-617`) match on `"scoop"`, `"choco"`, `"winget"`
by name. Adding a backend with an exit-code convention means editing the core executor. That
knowledge belongs on the backend, which is where the architecture otherwise puts it.

**Liskov / Interface Segregation — `RepoManager`.** See H2: two implementations cannot honour
`remove_repo`'s contract and the system degrades silently rather than refusing.

**Stringly-typed errors.** All 23 `Error` variants wrap a bare `String` (`src/core/error.rs`), and
`Error::Other` is **131 of 392** constructions — a third of all errors carry no machine-readable
structure. Callers cannot branch on cause, only format a message. This is why several recovery
paths resort to substring matching.

> These are real but **none is a shipping blocker**. Do not let refactoring jump the queue ahead of
> §3 and C1. Each would be a large diff touching many callers, and `CLAUDE.md`'s "no change breaks
> existing code" rule makes them expensive to do safely.

---

## 7. Instability

- **Non-deterministic output under a TTY** (B1/B2): 80 / 640 / 83 lines across three identical runs.
- **Uniform retry semantics** (`transaction.rs:393`): install and remove both get `max_retries: 3`
  with backoff regardless of whether the failure is retryable. A held dpkg lock and a nonexistent
  package are retried alike.
- **Telemetry off-by-one**: `attempt: attempt - 1` (`transaction.rs:485, 510`) reports 0 attempts
  for a first-try success.
- **Panic surface is small but real**: 82 `unwrap`/`expect`/`panic!` outside `#[cfg(test)]` (897
  total, so the discipline is genuine). The 9 in `src/core/executor.rs` are the ones worth
  attention, including `.expect("failed to create dummy status")` on paths that spawn `/bin/false`
  and `cmd /C exit 1` to fabricate an `ExitStatus`.

---

## 8. Unintuitiveness

**U1 — 45 top-level commands**, with clusters that overlap without a clear rule for choosing:

- *Removal*: `remove`, `prune`, `orphans`, `clean`, `unmanaged`, `purge-unmanaged`
- *Time travel*: `undo`, `rollback`, `generation`, `snapshot`, `bisect`
- *Preview*: `status`, `plan` (via `--dry-run`), `doctor`, `check`
- *Adoption*: `migrate`, `adopt`, `clone`, `teleport`

`status` and `prune` are two views of the same drift computation under different names; `undo` vs
`rollback` vs `generation` is three vocabularies for restoring prior state. **Stop-and-ask —
consolidation removes features.**

> **Corrected 2026-07-27, measured against `linix --help` rather than read off the source.** The
> count is **62**, not 45. Ten of the commands named above **do not exist**: `remove` (it is
> `uninstall`), `prune` (it is `snapshot prune`), `orphans` (`remove-orphans`), `clean`
> (`clean-cache`), `unmanaged` (`unmanage`, which is a different thing again), `status` (it is
> `service status` and `git status`), `doctor` (`check`), `migrate`, `clone`, `generation`. So
> **"`status` and `prune` are two views of the same drift computation" is a claim about two
> commands that are not there**, and "three vocabularies" names one verb that never existed.
>
> The removal cluster that *does* exist is not redundant — each does something none of the others
> does: `uninstall` (a package), `remove-orphans` (what the manager itself calls an orphan),
> `purge-unmanaged` (everything LiNix does not manage), `unmanage` (forget it, keep it installed),
> `reset` (forget everything, keep everything installed), `clean-cache` (archives, no packages).
>
> **The one real overlap is `undo` / `history` / `rollback`**, and it is a naming problem rather
> than a redundancy: `undo` is the *filesystem snapshot* gallery, while `history` (TUI) and
> `rollback <ref>` (CLI) are two interfaces onto the *git manifest* history. A user who wants to
> undo their last sync reaches for `undo` and gets the wrong mechanism.

**U2 — log noise on by default.** Ordinary runs emit `INFO` tracing with ANSI escapes above the
actual answer (`StateRegistry: No state file found…`, `LiNix Kernel: v6.0.0 kernel initialized
successfully.`). Stop-and-ask.

> **Measured 2026-07-27.** Confirmed, with corrections and one defect the review did not see.
> The default filter is `EnvFilter::new("info")` at `main.rs:44`; there are **256 `info!`/`warn!`
> call sites**. The `StateRegistry` line is real and fires on **every** run, not just the first,
> because a read-only command never writes the registry it just reported missing. The **kernel
> banner does not exist** — same stale-binary artefact as H4.
>
> **The defect: `--verbose` does nothing.** Its help says "Enable debug-level logging". Measured:
> `linix --verbose list -b cargo` emits **0** DEBUG lines, `RUST_LOG=debug` emits **5**. The
> subscriber is built at `main.rs:41`, before clap parses at `main.rs:81`, so `cli.verbose` reaches
> `CommandExecutor` and never reaches the filter. `--quiet` does not suppress the INFO stream
> either — it only sets `config.quiet`, which hides the planned-changes list.
>
> **Constraint on any ruling that lowers the default:** some INFO lines *are* the answer.
> `linix sync` on an up-to-date machine prints `already up to date` at `info!` and nothing on
> stdout. Default to `warn` without first moving those to stdout and a no-op sync goes silent.

**U3 — `status` is slow.** Read-only `linix status` against a single apt backend took **~50-100
seconds** per run. Worth profiling after B1 is fixed — some of it may be `info()` being called
per-spec (`planner.rs:497` notes each check is a separate process spawn).

---

## 9. What is good — preserve this through the fixes

- **`src/core/download.rs`** is exemplary. HTTPS and checksums are separate opt-outs that
  deliberately do not imply each other; the scheme is enforced on **every redirect hop** rather than
  the typed URL; the rationale for per-line flags over a global config key is written down. Do not
  disturb it.
- **`src/core/datalock.rs`** is a correct file lock with a genuinely useful contention message. It
  is the model for H1.
- **`src/app/sync/guard.rs`** — `--yes` deliberately not overriding mass-removal is the right call
  and rarely made. It just needs to cover B3's path.
- **Comment discipline** per the project's P6 rule: comments state constraints and name the bug they
  prevent. Match this style in every fix.
- **No live command injection.** The `sh -c` paths I traced (`pacman.rs:29`, `generic.rs:614`)
  single-quote values and reject metacharacters. The denylist is fragile and should become an
  allowlist eventually, but there is no live vulnerability — do not report one.
- **1,324 tests and a clean `-D warnings` clippy gate.**

---

## 10. Order of work

Dependencies matter — B1 changes the conditions under which B3 and H3 are diagnosed.

| # | Item | Blocked by | Ask first? |
|---|---|---|---|
| 1 | **C1** — pty test in CI | — | no |
| 2 | **B1** — stop inheriting child stdout/stderr | C1 (so the fix is provable) | **yes** |
| 3 | **B2** — pager suppression | B1 | **yes** (same ruling as B1) |
| 4 | **B3** — rollback correctness + guard | B1 | **yes** (guard-refusal behaviour) |
| 5 | **B4** — orphaned children | — | no |
| 6 | **H1** — exec lock | — | no |
| 7 | **H2** — repo removal + placeholder guard | — | no |
| 8 | **H3** — sudo keepalive | B1 | no |
| 9 | **H4** — version | — | **yes** |
| 10 | §6 SOLID, U1-U3 | — | **yes** for U1/U2 |

Start at 1. Items 5-7 are independent of everything else and can proceed in parallel with the
rulings for 2-4.

---

## 11. Confidence

| Finding | Status |
|---|---|
| B1 | **Verified empirically** — built current source, measured 609 vs 1 rows |
| B2 | **Verified empirically** — pager captured mid-run, had to be killed; 80/640/83 lines |
| B3 parts 1 & 3 | **Verified by inspection + grep** (no guard reference in `transaction.rs`) |
| B3 part 2 (mass-uninstall chain) | **Traced, not triggered** — confirm in a container first |
| B4, H1, H3 | **Verified by inspection + grep** (`kill_on_drop` absent; `File::create` semantics) |
| H2 | **Verified by inspection** — template/substitution mismatch is unambiguous |
| H4 | **Verified** — `--version` output vs `CHANGELOG.md` |
| U3 | **Measured** on one machine, one backend — not profiled; treat as a symptom, not a diagnosis |
