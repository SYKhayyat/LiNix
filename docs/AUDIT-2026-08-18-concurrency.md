# Audit: inefficiencies, blocking, and races

**Baseline:** `3cb5232`, working tree clean, 2026-08-18.
**Scope:** three questions, asked across all 367 source files / ~158k lines.

1. What costs more than it needs to?
2. What blocks when it should not?
3. What races?

**27 findings: 8 blocking, 8 races, 11 inefficiencies.** Ordered within each section by severity.

---

## Read this first

The architecture-level performance work in this repo is done, and done well. `list_installed`
is memoised once per run behind `core::installed`; PATH lookups are memoised in `core::launch`;
regexes are cached with a bounded, deliberately-not-LRU eviction; the planner asks each manager
once instead of once per spec; recovery runs on the transaction DAG rather than serially;
`fleet` fans out over SSH; archive extraction and SHA-256 hashing are correctly on the blocking
pool; child processes are owned, bounded and killed on drop; HTTP clients carry deliberate,
differentiated timeouts. Every one of those carries a comment explaining the measurement that
motivated it. **The easy wins are gone. Do not go looking for them.**

What is left has one shape, and it is worth naming before the list starts:

> **This repo writes its rules down, correctly, and then applies them at some call sites and
> not at their siblings.** Nineteen of the twenty-seven findings below are the *un-fixed sibling* of
> a fix that already shipped elsewhere in the tree — and in seven cases the file that states the
> principle is one directory away from the file that breaks it.

That is the `CLAUDE.md` "fix the whole family" shape, one layer down. It also means most of
these are cheap: the correct pattern is already in the tree, with a comment explaining why, and
the work is to apply it where it was missed. Where that is true, the finding names the file to
copy from.

**A second, narrower theme worth holding in mind while reading the blocking section:** this
codebase deliberately multiplexes its hottest fan-outs onto a *single task* (`planner.rs:659`,
`planner.rs:961` — *"the futures borrow `&self` so this stays on one task (no spawn)"*). That is
a sound choice, but it changes the cost of every synchronous call reached from inside a wave.
**A blocking call there does not cost one task's latency; it costs the whole wave's.** B2, B3,
B5 and I6 are all instances of that, and fixing them piecemeal without understanding the
mechanism will leave the next one in place.

---

## Verification status — read before acting

Everything below was verified **by reading the source at `3cb5232`**, following each claim to
the definition that backs it — the `sync_all` in `utils::file`, the `escalates()` in
`core::executor`, the `needs_root: true` on the apt backend, the `config_root.join("locks")` in
`model::layout`, the absence of a ceiling in `wait_watched`'s `pump`. Line numbers and control
flow are checked.

**Nothing below was verified by running Shall.** No benchmark was taken, no container harness
was run, no deadlock was reproduced. Where a finding states a number (B1's 120 seconds, B3's
flush count, B6's 64 KiB), that number is *derived* from constants in the source or from
documented OS behaviour, not measured. Per this repo's own standard, **reproduce each one before
fixing it, and put the measurement in the commit message.** Each finding says what the cheapest
reproduction is.

**One correction I made mid-audit, recorded because it is instructive.** I initially believed
`recording_locks()` had no callers, which would have meant the bare-name lock was never written
and II.6's stability guarantee was dead. That was wrong — my grep pattern was malformed;
`verbs/sync.rs:81` and `verbs/plan.rs:846` both call it. **The re-check is what found R3**, now
one of the three most serious items here. Treat a "this whole feature is dead" conclusion as a
signal to re-grep, not to write up.

**Confidence is stated per finding.** Two items (R5, R6) are **latent** — a correct outcome
resting on a fragile mechanism — and are labelled as such rather than dressed up as live bugs.
Where I judged something *not* to be a defect, it is in "What I checked and found sound" at the
end, with the reason. That section is part of the deliverable: it is what stops the next pass
re-treading the same ground.

**Second pass.** Re-run at the end of the audit: `git status` clean apart from this file, HEAD
still `3cb5232`. Nothing changed underneath it — no concurrent writer was active. If you are
reading this later, diff against that SHA before trusting a line number: the findings are anchored
to content and quoted code, but the numbers are not.

## What was read, and what was run

The first pass was **pattern-driven**: grep the whole tree for each defect shape, chase every hit
to its definition. That finds instances of classes you can name. It does not find what you had no
name for. A second pass closed that gap:

**Read end-to-end** (non-test portions): `backends/generic.rs` (2,246), `config/grammar/statement.rs`
(2,196), `app/sync/guard.rs` (1,239), `model/resolve.rs` (1,222), plus structural passes over
`app/scheduler/mod.rs` and `backends/onboarder.rs`. **Three findings came only from reading** —
I10, I11 and R8 — and none of them has a greppable shape. I10 in particular is a `.to_lowercase()`
sitting inside a `find` closure; no pattern search would ever have flagged it.

**The suite was run, not assumed.** `cargo test --no-fail-fast`: **616 passed, 0 failed, 0 ignored,
493 s.** This report's premise — that 27 findings sit under a green suite — is now verified rather
than asserted. It also matters for what follows: nothing here is a symptom of a broken build.

**Ran** (all inside the 5-minute budget on a warm 28 GB `target/`, except the suite at 8 min):

- `cargo clippy --all-targets` — **38.7 s, zero warnings.** The tree passes its own gate cleanly.
- `cargo clippy` with `await_holding_lock`, `await_holding_refcell_ref`, `redundant_clone`,
  `needless_collect`, `mutex_atomic`, `large_stack_frames` — **66 s, 49 hits.** See I12.

**The most valuable result was a negative.** Zero `await_holding_lock`, zero
`await_holding_refcell_ref`, zero `mutex_atomic`. That is worth stating plainly because it is
easy to assume the lint subsumes the hunt: **it does not.** `await_holding_lock` fires on a
`std::sync::MutexGuard` held across an `.await`. B2 and B3 are a *tokio* mutex held across a
*synchronous* `fsync` — no `.await` inside the critical section, and no lint models it. The
automated check and the manual one cover disjoint ground, and the clean result on one says
nothing about the other.

**Also checked:** `Cargo.toml` and the crate roots carry **no clippy configuration** — no
`#![deny(clippy::…)]`, no `[lints]` table — and the tree holds only 18 `#[allow(…)]`, all of them
benign (`unused_imports` under `cfg`, `too_many_arguments`, `module_inception`). **Nothing is
suppressed.** The clean default run is a real result, not a configured silence.

**Never read:** `verbs/plan.rs`, `verbs/packages.rs`, `main.rs`, `cli/args.rs`, `config/config.rs`
end-to-end (all sampled heavily); `parsers/`; most of `utils/`; `model/edit.rs`; `backends/artifact/*`
beyond the teardown and format paths. Estimated read coverage after both passes: **~45% of
`src/`**, with 100% pattern coverage for the named classes and 100% of the concurrency-bearing
modules.

---

# Blocking

## B1 — The hook re-entrancy stand-down does not survive `sudo`

**Severity: highest. Confidence: high.** A 120-second stall per escalated manager transaction, on
Linux machines that have run `shall hooks`.

### The mechanism, in order

1. `shall sync` is a `LockScope::Writer` (`src/cli/args.rs:1082`), holding the exclusive
   data-directory lock for its whole run.
2. It installs through apt. apt is `needs_root: true` (`src/backends/generic.rs:2304`), so
   `CommandExecutor::escalates()` is true on any non-root Linux
   (`src/core/executor.rs:1048-1050`) and the argv becomes `sudo -n apt install …`
   (`src/core/executor.rs:1117-1120`).
3. Immediately after, the executor sets the re-entrancy marker
   (`src/core/executor.rs:1126-1130`):

   ```rust
   let mut env = HashMap::new();
   env.insert(crate::core::executor::INSIDE_SHALL.to_string(), std::process::id().to_string());
   ```

   under the comment: *"The env var travels to every descendant, and `hook-reconcile` stands down
   when it sees it."*
4. **It does not travel to every descendant, because `sudo` is in the way.** `sudo` runs
   `env_reset` by default: it rebuilds the child environment and keeps only `env_keep` (`TERM`,
   `LANG`, `LC_*`, a `PATH` from `secure_path`, …). `SHALL_INSIDE` is not in that set, and
   nothing here adds it — no `-E`, no `--preserve-env`, and `shall hooks` writes no `env_keep`
   line. Verified by grep: `INSIDE_SHALL` appears in exactly four places (`executor.rs:25`,
   `executor.rs:1128`, `executor.rs:2058` (a test), `main.rs:849`); `sudo -E` and `preserve-env`
   appear nowhere in the tree.
5. So apt runs **without** the marker. Its `DPkg::Post-Invoke` — which Shall itself wrote to
   `/etc/apt/apt.conf.d/99shall` (`src/app/pm_hooks.rs:106-114`) — fires
   `shall hook-reconcile --manager apt`.
6. `stands_down_inside_shall` (`src/main.rs:848-851`) requires **both** the marker and a `hook-*`
   subcommand. The marker is gone, so it does not fire.
7. `HookReconcile` is a `Writer` (`src/cli/args.rs:1115`), so it calls `DataLock::for_one_step`,
   which waits `WAIT_SECS = 120` (`src/core/datalock.rs:23`) for a lock **its own grandparent is
   holding and will not release until the sync exits**.
8. Two minutes later it fails with *"the Shall data directory is locked by …"*, the reconcile is
   lost, and the sync moves to the next package — where it happens again.

### Why this one goes first

The stand-down exists *precisely* to prevent this. `src/cli/args.rs:1133-1148` describes the
identical failure on pacman and calls it "two minutes of silence per transaction." The fix
generalised correctly from one subcommand to all `hook-*` subcommands, and
`every_hook_shall_installs_stands_down` asserts that against `pm_hooks.rs` rather than against
prose — genuinely good work. **Nobody checked whether the carrier survives the trip.** And it
fails on exactly the managers `pm_hooks` targets: apt, dnf and zypper all need root, so all three
go through `sudo`.

### Why it was not caught

- The Rust suite is hermetic and mocks the executor, so nothing crosses a real `sudo`.
- The container harness runs as **root** — and `escalates()` returns `false` for root, so `sudo`
  is never invoked there and the marker propagates fine.

**The one configuration that breaks it is the ordinary one: a normal user on a normal Linux
desktop.** That is a gap in the harness as much as a bug in the code — see the harness note at
the end.

### The fix I would argue for

Not `sudo -E`, and not a `sudoers` edit — both need privilege Shall may not have, and both fail
*open* (silently, back into this stall) when denied. Stop depending on the environment:

> **A `hook-*` subcommand should `try_lock` and stand down on contention, instead of waiting.**

The lock is already an OS lock on an open handle and the owner file already records the holding
pid and command (`datalock.rs:92-93`). A hook that finds the directory locked has nothing useful
to do with a 120-second wait — by the time it won, the sync has finished and reconciled anyway.

That also closes the variant this finding does not depend on: an *unrelated* `apt install` typed
by a person while a `shall sync` runs. There the marker is legitimately absent and the stand-down
legitimately should not fire — and today that hook also eats 120 seconds.

**Reproduce.** Cheap half first: `sudo -n sh -c 'printenv SHALL_INSIDE'` from a process that set
it — two seconds, and it settles the premise. Then the full case: Linux container, non-root user
with passwordless sudo, `shall hooks` installed, time a `shall sync` that installs one apt package.

**Ruling needed?** No — a bug against documented intent. The fix's only observable change is that
a two-minute hang disappears; reasoning goes in the commit message.

---

## B6 — The `on_drift` hook payload can exceed the pipe buffer and deadlock forever

**Severity: high. Confidence: high.** A documented constraint that a real caller violates,
producing an unbounded hang with no timeout armed.

### The documented constraint

`src/core/supervise.rs`, on `supervised_output_fed`:

> *"The payload is written before the output is drained, so it must be small enough for the pipe
> buffer — **every caller here sends a JSON fact sheet of a few hundred bytes**. A large one would
> deadlock against a child that will not read until it has written."*

The mechanism is right there in `supervise` — the feed is written **before** `wait_watched` is
called, and `wait_watched` is what spawns the two `pump` tasks that drain the child:

```rust
if let (Some(feed), Some(mut pipe)) = (feed, child.stdin.take()) {
    let _ = pipe.write_all(feed.as_bytes()).await;   // <-- nothing is draining the child yet
    let _ = pipe.shutdown().await;
}
RawExecutor::wait_watched(child, what, …, command_idle_timeout()).await   // <-- drain starts here
```

### The claim is false

`supervised_output_fed` has exactly one caller, `src/app/events.rs:244`, reached from
`EventHooks::fire` (`events.rs:83`). Two events feed it:

- `Event::AfterSync` (`src/app/sync/mod.rs:332-343`) — `{"installed": n, "removed": m}`. Tiny,
  as documented.
- **`Event::OnDrift` (`src/app/sync/mod.rs:184-188`) —
  `serde_json::to_value(changes.generate_report())`.** That is `SyncReport`
  (`src/app/sync/planner.rs:517-542`): **one entry per install and one per removal**, each
  carrying `backend`, `name`, `version` and `source`.

At roughly 80–150 bytes of JSON per entry, a plan of ~500 changes is ~50 KiB and ~1000 changes is
~100 KiB. **Linux's default pipe buffer is 64 KiB**; Windows anonymous pipes are commonly smaller.
So the payload crosses the bound on an ordinary large sync — and trivially on the case this repo
already knows about, where a fresh config makes every installed package a removal.

### Why the hang is unbounded

Once `write_all` blocks on a full pipe:

- nothing is draining the child's stdout/stderr, because `wait_watched` has not been reached — so
  a chatty hook fills *its* output pipe and stops reading stdin, and both sides are stuck;
- **no timeout is armed.** `command_idle_timeout()` is passed *into* `wait_watched`, and the idle
  clock starts inside it. The deadlock happens strictly before that point, so the bound that
  exists for exactly this purpose never engages.

The result is a `shall sync` that hangs forever with no message — precisely the failure the
module's own doc says supervision exists to prevent (*"one that waited on something waited
forever, and one abandoned by whatever fired it kept running"*).

### Fix

Write the feed **concurrently with the drain**, not before it. Either move the `write_all` into a
spawned task whose handle is joined after `wait_watched` returns, or restructure `wait_watched`
to take the feed and start the pumps first. Standard shape; no semantics change.

Then delete the "few hundred bytes" sentence, or make it true — if the constraint is to survive,
something has to enforce it. A `debug_assert` on payload length is not enough; the fix should
remove the constraint entirely.

**Reproduce.** An approved `on_drift` hook that reads stdin slowly (or not at all), and a manifest
producing >64 KiB of drift report. Hermetic: the hook can be `sleep 5` and the report can be
synthesised. **Write this test first — it fails today and it is the whole finding.**

---

## B2 — Eleven call sites `fsync` while holding the global state mutex, on a runtime worker

**Severity: high. Confidence: high.** The largest clean "family not fixed" here.

### The rule, which this repo already wrote down

`StateRegistry::snapshot()` exists so the serialisation happens under the lock and the *writing*
happens after it, off the runtime (`src/core/state.rs:177-179`):

> *"The bytes to write, taken while the lock is held, so the writing can happen after it is
> released and off the runtime."*

And `src/core/executor.rs:1591` states the general rule: *"`spawn_blocking` because `sync_all`
parks a thread on the disk, and parking a runtime worker…"*.

**That the write is a real `fsync` is not an inference.** `save()` → `snapshot()?.write()` →
`persist()` (`utils/file.rs:35`) → `atomic_write()` → `temp_file.as_file().sync_all()`
(`utils/file.rs:128`). A physical disk flush.

### Who follows the rule

`src/app/sync/mod.rs:297`, `:1011`, `:1477` — snapshot under the lock, then
`spawn_blocking(move || to_write.write())`. Correct. (`src/app/adopt.rs:550` uses `spawn_blocking`
but see **I1**.)

### Who does not

Eleven sites call `state.lock().await.save()`, holding the global state mutex across a synchronous
`fsync` on a tokio worker:

| file:line | command |
|---|---|
| `src/app/leases.rs:118` | lease expiry |
| `src/app/leases.rs:214` | suspension processing |
| `src/app/shell/mod.rs:114` | `shall shell` provisioning |
| `src/verbs/cleanup.rs:631` | `cleanup` |
| `src/verbs/declare.rs:512` | `declare` |
| `src/verbs/declare.rs:566` | `declare` (non-locked variant) |
| `src/verbs/declare.rs:610` | `declare` |
| `src/verbs/packages.rs:1001` | install / remove |
| `src/verbs/packages.rs:1056` | install / remove |
| `src/verbs/packages.rs:1083` | install / remove |
| `src/verbs/upgrade.rs:243`, `:381` | `upgrade` |

### What it costs — two harms, not equal

- *Parking a runtime worker.* Real but bounded; several of these sit at the end of a command with
  nothing else in flight. On its own, a nit.
- *Holding the global state mutex across the flush.* **This is the one that matters.** Every other
  task wanting `state` — 60 `state.lock()` sites — waits out a disk flush. In `leases.rs` and
  `upgrade.rs`, which run inside or alongside a sync, that is a stall injected into the middle of
  a concurrent wave.

### Fix

Mechanical; the pattern is three lines away in `sync/mod.rs`. Replace `state.lock().await.save()?`
with `snapshot()` under the lock plus `spawn_blocking(…write())` after it. **Do not weaken
durability** — `sync_all` is load-bearing.

### Siblings I checked and am *not* flagging, with reasons

`src/app/locate.rs:40` (settings, not the registry, not concurrent); `src/config/config.rs:1067`
and `src/config/settings.rs:123` (preferences, once per command); `src/app/profile.rs:141,204,
284,319,383` (profile files, sequential); `src/core/ledger.rs:70` (one write per command);
`src/verbs/plan.rs:581` and `src/verbs/setup.rs:610` (once per invocation).

All are also synchronous `fsync` inside `async fn`, and all are fine: none holds a contended lock
and none sits in a parallel wave. **The defect is the combination, not the `fsync`.** Stated so
nobody "fixes" seventeen call sites when eleven are the family.

---

## B3 — The journal `fsync`s once per entry, un-batched, under a global mutex, on the runtime

**Severity: high. Confidence: high.** Same shape as B2, on the WAL, scaling with package count.

**Where.** `src/core/journal.rs:271-280`. Every `record_start` / `record_success` /
`record_failure` calls `append()` → `utils::file::append_line` → `file.sync_data()`
(`utils/file.rs:73`). One physical flush per call, synchronous, inside `async fn`, under
`journal.lock().await`.

**The batch that is not a batch.** `src/core/transaction.rs:953-990` opens WAL entries for a whole
wave, commenting:

> *"The WAL, per package and before the manager is invoked. Recovery depends on the entry reaching
> disk first, and a batch does not change that — it changes how many bytes each entry costs."*

The first half is right and non-negotiable. But the loop takes the journal mutex once and calls
`record_start` **once per member** — *k* writes and *k* flushes for a wave of *k*, holding the
mutex throughout. It batches the lock acquisition and not the thing that costs.

**What it costs.** For *N* packages: *N* flushes at start plus *N* at completion, serialised. On
the 298-package config this repo's own planner comment cites, ~596 flushes on the critical path —
and each one stalls the whole wave, per the single-task note in the preamble.

**Fix, in two parts.**

1. **Free, no semantic change:** batch the writes. One `write_all` of *k* serialised lines then
   **one** `sync_data` gives every entry the same guarantee — all on disk before the function
   returns, therefore before any manager is invoked — at one flush instead of *k*. `append_line`
   needs an `append_lines` sibling. **Do this one.**
2. **Needs an owner ruling:** whether `record_success` needs a flush *per package* at all.
   *For:* the WAL's promise is that its record is not behind reality. *Against:* recovery re-runs
   an interrupted install, and `src/app/sync/mod.rs:1207` states that re-running an install over a
   half-installed package is something every manager Shall drives can do — so a crash between
   "install succeeded" and "success recorded" costs one idempotent re-run, not corruption.
   **A durability trade-off is behaviour, which is the owner's call.**

**Reproduce.** `--timings` instruments every manager invocation; the flushes are the gap it does
not account for. A 200-line manifest against a mock backend, timed with and without batching,
isolates it without touching a real manager.

---

## B5 — `block_in_place` protects other workers, not other futures on the same task

**Severity: medium. Confidence: high on the mechanism, medium on the impact.** Structural.

`core::blocking::on_the_terminal` uses `tokio::task::block_in_place`, which "moves the runtime's
other tasks off this worker." Correct, and the right primitive for a prompt. But **it does nothing
for futures multiplexed onto the *same* task** — and this codebase deliberately multiplexes its
two hottest fan-outs onto one task (`planner.rs:659`, `planner.rs:961`).

So any blocking call reached from **inside** a wave stalls the entire wave. Two such paths:

1. **`tool_help::help_text`** (`src/core/tool_help.rs`) spawns `program --help` through
   `blocking::command_output` — a synchronous `std::process::Command`. Reached from
   `accepts_flag` at `src/backends/generic.rs:1085`, `:1105`, `:1215`, `:2988` — the **install
   argv construction path**, i.e. per package, inside the wave. See also **I3**.
2. **`#rhai` hooks.** `LuaHooks::run_rhai` (`src/app/hooks.rs:229`) is a **synchronous** `fn`
   called from the async `run_hook` — with no `block_in_place` at all. Rhai's `http_get`
   (`src/core/rhai_stdlib.rs:170-180`) then does a **blocking `rx.recv()`** on a
   `std::sync::mpsc::sync_channel` while a spawned task performs the request. Hooks run per
   package around each batch (`transaction.rs:999`, `:1227` — see **I6**), so a `#rhai` hook
   making one HTTP call stalls every other package in that wave for the round trip.

**Fix.** For (1), make the probe async, or hoist it out of the wave — it is per-*binary*, not
per-package (see I3). For (2), route `run_rhai` through `off_the_runtime`, and replace the
`sync_channel` recv in `http_get` with a proper await where a runtime is present.

**A related asymmetry worth fixing on its own:** `off_the_runtime` — the primitive whose doc lists
"unpacking an archive, hashing a file, waiting out a file lock" — has exactly **one** caller in the
tree (`datalock.rs:42`). The archive and hash paths reach `spawn_blocking` directly, which is
correct but means the named primitive is not the one anybody uses. Either route those through it
or shrink its doc to what it actually does.

---

## B4 — `github:` and `web:` `fsync` under their internal lock, inside the install wave

**Severity: medium. Confidence: high.** As B2, in the two backends that install concurrently by
nature. `save_state_internal` (`github.rs:282-286`) and `save_state` (`web.rs:87-91`) call
`utils::file::persist` — a synchronous `fsync`-ing atomic write — inside an `async fn` while
holding `internal_lock`.

Fixing **R1** properly (one in-memory map, written once at the end of the run) removes this and
**I2** as a side effect. Listed separately so it is not lost if R1 is patched narrowly.

---

## B7 — Child process output is captured with no ceiling

**Severity: medium. Confidence: high on the mechanism, low-medium on the impact.**

`RawExecutor::wait_watched`'s `pump` (`src/core/executor.rs:376-396`) reads a child's stdout and
stderr into `collected: Vec<u8>` in 8 KiB chunks, **with no cap of any kind**:

```rust
let mut collected = Vec::new();
loop {
    let n = src.read(&mut buf).await?;
    …
    collected.extend_from_slice(&buf[..n]);
    …
}
```

Every manager invocation's complete output is held in memory until the command exits. A manager
that streams heavily — a `nix` build, a `cargo install` compiling, a progress bar redrawn with
carriage returns for an hour — grows this without bound, and a concurrent wave holds *N* of them
at once.

**This repo already treats unbounded input as a hazard everywhere else.** `core::download` has an
explicit 2 GiB ceiling with a pre-flight refusal (*"refused before it filled the disk"*), and
`executor.rs:1462` caps what a *terminal* is shown. The one path that accumulates unboundedly is
the one that keeps the bytes.

**Fix.** A ceiling on `collected`, with the same shape download uses: stop accumulating past the
cap, keep the head and the tail (the two parts any error message wants), and note the truncation
in what is returned. The `mirror` path already streams to stderr as bytes arrive, so nothing a
user is watching is lost.

**Reproduce.** `yes | head -c 2G`-style child through the executor, watch RSS. Or read it off the
code and treat it as a robustness fix — I would accept this one without a benchmark, since the
argument is structural and the fix is cheap.

---

## B8 — Cache cleanup runs a synchronous filesystem crawl on the runtime

**Severity: medium (opt-in). Confidence: high.**

`model::cache::clean_cached` is `async`, but the work it does is not:

```rust
// cache.rs:34 — a plain sync fn
fn matches_in(root: &Path, basename: &str) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root).max_depth(MAX_DEPTH) … .collect()
}
// cache.rs:52 — also sync, calls matches_in for every root
pub fn find_cached(basename: &str, extra: &[PathBuf]) -> Vec<PathBuf> { … }

// cache.rs:68 — async, but the crawl inside it is not, and is not moved off the runtime
pub async fn clean_cached(basename: &str, extra: &[PathBuf]) -> Vec<PathBuf> {
    for path in find_cached(basename, extra) { … tokio::fs::remove_file(&path).await … }
}
```

The roots are `$XDG_CACHE_HOME`, `~/.cache` and — on unix — **`/var/cache`**
(`standard_cache_dirs`, `cache.rs:20-32`), plus anything in `cache_dirs`. `MAX_DEPTH = 4` bounds
it, and the comment says why: *"shallow enough that `/var/cache` is not a full-disk walk."* That
is a real bound on the *extent*; it is not a bound on the *cost*. Four levels of `~/.cache` and
`/var/cache` on a working machine is thousands to tens of thousands of `stat` calls, run
synchronously on a tokio worker — and reached from the removal path, so per **B5** it stalls the
whole wave rather than one task.

**Fix.** Wrap the crawl in `spawn_blocking` — this is the textbook case for `off_the_runtime`,
whose doc names exactly this kind of work and which currently has one caller. Combine with **I9**,
which is the same function.

**Severity note, stated honestly:** `clean_cache_on_remove` defaults to `false`
(`src/config/config.rs:984`), so this is off unless a user opts in. That caps how much it matters
today; it does not make it correct, and a default can change.

**Sibling checked and worth fixing with it:** `src/backends/go.rs:236` — `std::fs::read_dir`
inside `async fn scan(&self)`, which is on the `go` backend's `list_installed` path and therefore
inside the planner's fan-out. Same class, much smaller extent (`$GOPATH/bin` is a flat directory
of a few dozen entries), so it is a tidy-up rather than a defect. **Siblings checked and NOT
affected:** `model/modules.rs:127`, `model/profiles.rs:91`, `model/dotfiles.rs:94`,
`app/apply/execs.rs:355`, `core/installed.rs:170`, `backends/registry/mod.rs:2092` — all are
synchronous `read_dir` in synchronous functions, called from the config-reading phase, not from an
async wave. The defect is sync-crawl-inside-async-inside-a-wave, not sync crawling.

---

# Races

## R3 — The regex lock is written by *reader* commands, under no lock at all

**Severity: high. Confidence: high.** The cleanest missed sibling in the audit.

**Where.** `src/app/sync/resolver.rs:847-850`:

```rust
lock_changed |= lock.retain_declared(&declared);
if lock_changed {
    lock.save(&lock_path)?;      // RegexLock — no gate
}
```

Compare its sibling 330 lines later, `src/app/sync/resolver.rs:1177-1182` (BareLock):

```rust
// Written only when it changed: … And only by a run that acts: a preview that froze the
// backend it guessed at made the real install afterwards use that guess.
if lock_changed && self.may_record_locks && !self.config.dry_run {
    lock.save(&lock_path)?;
}
```

**Three guards on one, one guard on the other.** `may_record_locks` is opt-in, defaulting to
`false` (`resolver.rs:178`), set only by `recording_locks()` — whose doc says *"this resolution
belongs to a run that will act on it"* — called from exactly two places, `verbs/sync.rs:81` and
`verbs/plan.rs:846`. The regex lock consults none of it.

**What follows.**

1. **A `Reader` command writes state.** `Plan`, `Check`, `List`, `Diff`, `Export`, `Why`, `Info`
   are all `Reader` (`src/cli/args.rs:1051-1069`) under the comment *"Reads the machine, the
   config or a remote, and **writes neither**."* Every one resolves the model, and regex expansion
   happens during resolution. A `shall check` that expands a `^fonts-` pattern writes
   `locks/regex.toml` for real. The comment is therefore false, and the test policing that table
   (`the_readers_are_exactly_the_commands_that_read`) checks the *classification*, not whether
   readers actually refrain from writing.
2. **That write is under no lock whatsoever.** `Reader` never takes the data lock
   (`src/main.rs:1524-1525`). So `shall check` racing `shall sync` is two processes rewriting one
   TOML file whole — last-one-wins, an expansion silently lost. Exactly the failure
   `core::datalock` was built for, in a command the lock deliberately exempts. (See **R6**: the
   file is not even in the directory the lock covers.)
3. **The preview hazard BareLock's comment describes.** *"A preview that froze the backend it
   guessed at made the real install afterwards use that guess."* The regex lock records what a
   pattern matched; freezing it from a read-only command has precisely that shape.
   (`--dry-run` itself *is* covered, by luck rather than by this code: `persist` checks
   `dry_run::active()` at `utils/file.rs:36`. The `Reader` hole is separate and uncovered.)

**Fix.** Gate the regex lock as the bare lock is gated:
`lock_changed && self.may_record_locks && !self.config.dry_run`. Then check the other four ledgers
for the same asymmetry (**R6**), and add a test that a `Reader` subcommand leaves the config root
byte-identical.

**Reproduce.** Fresh repo, a manifest with a regex line, `stat locks/regex.toml`, run
`shall check`, `stat` again. Hermetic; no concurrency needed for the reader-writes half.

### Why the test that looks like it covers this cannot

`tests/dry_run_every_verb_tests.rs::a_preview_leaves_the_config_byte_identical` passes, and its
name is almost exactly the assertion R3 wants. **It cannot cover R3, structurally** — and the
reason is worth reading, because it is a good test defeated by its own rigour.

It runs `shall --dry-run <verb>`, snapshots the config root before and after, and asserts nothing
changed. Then — correctly — it guards against vacuity by running **the same verb without
`--dry-run`** on a fresh fixture and *requiring that control to change something*:

```rust
if ctl_changed.is_empty() {
    failures.push("… THE CONTROL DID NOTHING. Without a run that changes the config, the
                   dry-run assertion below cannot fail and proves nothing. Fix the fixture.");
```

That control admits only commands that mutate. Its 15 cases are `activate`, `deactivate`,
`profile create`/`save`, `module create`, `lock`, `config init`, `git init`, `hold`, `unhold`,
`heal`, `adopt`, `bundle` and `export` — **every one a `Writer`, and not one `Reader` among
them.** `check`, `list`, `plan`, `diff`, `why` and `info` cannot be added without failing the
control, because by definition they change nothing.

So the tree has a well-built gate for *"a preview must not write"* and none at all for *"a reader
must not write"* — and R3 is exactly a reader writing, with `--dry-run` nowhere in the picture.
The assertion needed is the **sibling** of this test, not an extension of it: run each `Reader`
subcommand with no flags and require the config root byte-identical, with its own control proving
the harness can see a write at all.

---

## R1 — `github:` and `web:` lose state records when two packages install concurrently

**Severity: high. Confidence: high.** A lost-update race on a per-backend state file:
read-modify-write with no lock held across the modify, inside a wave that is concurrent by design.

**Where.** The twin pair, which is the same code twice —

```rust
// github.rs:268
async fn load_state_internal(&self) -> HashMap<String, GithubState> {
    let _guard = self.internal_lock.lock().await;   // lock taken
    …read the whole file…
}                                                   // lock RELEASED here

// github.rs:282
async fn save_state_internal(&self, state: &HashMap<String, GithubState>) -> Result<()> {
    let _guard = self.internal_lock.lock().await;   // lock taken AGAIN
    …write the whole file…
}
```

**The window is enormous.** In `github.rs` the load is at line 485 and the matching save at 972,
with the release download and install in between. `web.rs` has the same shape at 124 → 345 and
355 → 384.

**The race.** Two `github:` packages in one wave, concurrent under the transaction semaphore
(`src/core/transaction.rs:452`):

| step | task A | task B |
|---|---|---|
| 1 | loads state `{}` | |
| 2 | | loads state `{}` |
| 3 | downloads + installs `A` (seconds) | downloads + installs `B` |
| 4 | saves `{A}` | |
| 5 | | saves `{B}` — **A's record is gone** |

The lock makes each individual file access atomic and does nothing about the thing that needs
protecting. This is the hazard `core::datalock`'s module doc states for the data directory —
*"two whole writes are last-one-wins"* — reproduced one layer down, without the lock that lesson
produced.

**Cost.** The lost record tracks a deployed artifact's paths and version. A package whose record
vanished reads as unmanaged: the next sync cannot see what it deployed and teardown has nothing to
remove. `web.rs:403` and `github.rs:1040` *read* this state, so the damage is not confined to the
write path.

**The same shape applies to the artifact ledger** in the same function (`github.rs:973`, `:1021` —
`ledger.save(&self.core.locks_file)`), which is per-backend and therefore shared by exactly the
concurrent installs above. Fix both together.

**Fix.** Hold `internal_lock` across the whole read-modify-write, or — better, and closer to the
rest of the tree — keep the state as one in-memory map behind one mutex, written once at the end
of the run, the way `StateRegistry` already is. The second option also kills **B4** and **I2**.

**Reproduce.** Two `github:` lines, `max_parallel >= 2`, sync from empty, read the state file: one
record where there should be two. Hermetic if the download layer is mocked.

**Family check.** I looked for every other backend with a private state file and this load/save
shape. `github.rs` and `web.rs` are the only two. `btrfs.rs:337,359` persists `/etc/fstab` but not
from a concurrent wave; the artifact backend keeps its records in the main registry. **The family
is two, and both are listed.**

---

## R4 — Concurrent `sudo` priming can put several password prompts on one terminal

**Severity: medium-high. Confidence: medium-high.** A check-then-act on process-global state with
no mutual exclusion, on a path that ends at a terminal.

**Where.** `CommandExecutor::ensure_sudo_credentials` (`src/core/executor.rs`, reached from
`run_on` at `:1116`):

```rust
if SUDO_PRIMED.load(Relaxed) { return Ok(()); }               // check
if let Some(why) = SUDO_REFUSED.lock()… { return Err(…); }    // check
… sudo -n -v …                                                // probe
… sudo -v  with  .stdin(Stdio::inherit())  …                  // ASK A HUMAN
SUDO_PRIMED.store(true, Relaxed);                             // act
```

No lock, no `OnceCell`, nothing serialising check and store.

**The race.** *N* escalated commands start concurrently in the first wave — ordinary with
`max_parallel > 1` and more than one root-needing backend (apt + snap + a system-scope flatpak).
All *N* read `SUDO_PRIMED == false`, all *N* find no recorded refusal, all *N* run `sudo -n -v`
and fail, and all *N* reach the interactive branch — **`sudo -v` with inherited stdin, several
processes reading a password from the same tty at once.** Keystrokes split between them; prompts
interleave; some fail and record a refusal that then short-circuits the rest through
`SUDO_REFUSED`.

**Why the surrounding code makes this more likely, not less.** `S88` and `S89` did good work here:
`-n` on every command so no manager invocation sits on a prompt, and a *remembered* refusal so a
120-second bound stops costing 900. Both make the priming call the single funnel every escalated
command passes through — which is what turns an unsynchronised check-then-act into a thundering
herd.

**Cost.** Benign case: one wasted `sudo -n -v` per task. Interactive case: a garbled prompt and a
spuriously recorded *permanent* refusal — which, by design, then fails the whole run.

**Fix.** Serialise the priming: a `tokio::sync::Mutex` around the body (re-checking `SUDO_PRIMED`
after acquiring), or an async `OnceCell` holding the result. One task probes and prompts; the rest
await its answer. Keep `SUDO_REFUSED` as it is — it is correct and well-reasoned.

**While you are there (minor):** `start_sudo_keepalive` (`executor.rs:1765`) spawns its 60-second
`sudo -n -v` loop whenever the platform allows, without consulting `SUDO_REFUSED`. On a machine
that has already recorded a permanent refusal, that is one pointless process per minute for the
rest of the run. Harmless, and free to fix in the same change.

**Reproduce.** Non-root Linux, `max_parallel >= 4`, no warm sudo timestamp, a manifest touching
two root-needing backends. Watch for more than one `sudo -v` in the process table.

---

## R2 — A listing memo can hand back a pre-mutation answer *after* the invalidation

**Severity: medium. Confidence: high.** A narrow but real staleness race in `core::installed`.

**Where.** `src/core/installed.rs`, `once()` versus `forget_all()`:

```rust
pub async fn once<F>(&self, backend: &str, fetch: F) -> Result<Listing> {
    let slot = self.by_backend.entry(backend.to_string()).or_default().clone();  // (1) Arc cloned
    let mut slot = slot.lock().await;                                            // (2) may wait
    if let Some(cached) = slot.as_ref() { return Ok(cached.clone()); }           // (3) stale?
    …
}

pub fn forget_all(&self) {
    self.by_backend.clear();      // clears the MAP, not the Arcs already handed out
    …
}
```

**The race.** The `Arc<Slot>` is cloned out of the `DashMap` at (1); `forget_all` drops the map
entry but not that Arc. A task that cloned before the clear reads it at (3), after the mutation
that was supposed to invalidate it:

| step | task A (a query) | task B (an install) |
|---|---|---|
| 1 | clones Arc `S`, which already holds listing `L` | |
| 2 | blocks at `slot.lock().await` | |
| 3 | | mutates → `run()` → `forget_run_scoped_answers()` → `forget_all()` clears the map |
| 4 | wins the lock, sees `Some(L)`, returns `L` | |

`L` predates the install *and* the invalidation.

**Why the window is not as small as it looks.** Step 2 is the giveaway. `once()` holds the slot
mutex across the whole fetch — deliberately and correctly, so two askers produce one `winget list`
rather than two. That fetch is a subprocess measured elsewhere in this tree at over a second on
Windows. A task can sit at (2) for seconds holding a stale Arc, which is ample time for a
concurrent install to complete. **Exposure scales with `max_parallel`** — worst on the
configurations that matter most.

**Cost.** A stale "is it installed?" answer taken after an install completed: a needless
reinstall, or a drift report naming a package that was just fixed. Self-correcting next run, which
is why this is not ranked with R1/R3.

**Fix — and the correct pattern is already in this repo, one file over.** `VARS_MEMO`
(`src/app/sync/resolver.rs:22-41`) solves the identical problem by putting a generation counter
(`RESOLUTION`, an `AtomicU64`) **in the memo key**, so an invalidated entry cannot be reached by an
already-cloned handle. Do the same: `forget_all` bumps an `AtomicU64`; `once()` samples it at (1)
and discards the cached value under the slot lock at (3) if the generation moved. ~6 lines, no
change to the dedup property.

**Do not fix it by shortening the lock hold** — the long hold is what makes the memo work, and the
comment says so.

**Note.** `forget_all` already carries the sentence *"the invalidation that covers one and not the
other is the invalidation that covers neither."* This is a third instance of that sentence, about
that same function.

---

## R5 — `remote_gate` is a cap scoped to an object that is built 34 times

**Severity: latent. Confidence: high on the mechanism, low on current impact.** Reported as
fragility, not a live bug — verified as such rather than assumed.

**Where.** `src/app/sync/resolver.rs:143` and `:179`:

```rust
/// The one cap on remote lookups this resolver has in flight.
/// … One gate held by the leaf that actually talks to a registry is the number a user set.
remote_gate: Arc<tokio::sync::Semaphore>,
…
remote_gate: Arc::new(tokio::sync::Semaphore::new(config.network_parallel.max(1))),
```

The intent is that `network_parallel` is *the* number of concurrent remote lookups. But the
semaphore is constructed inside `StateResolver::new`, and `App::resolver()`
(`src/app/context.rs:217-219`) is **not memoised** — it mints a fresh `StateResolver`, and thus a
fresh semaphore, on every one of its 34 call sites. The neighbouring `App::backends()`
(`context.rs:202`) *is* a `OnceCell`.

**The principle is stated in this repo, one file away.** `src/core/ratelimiter.rs:19-21`:

> *"`Arc<OnceLock<_>>` rather than `OnceLock` inside a clone: the cell is what the clones share,
> so two backends holding copies of one quota still hold ONE quota. **A per-clone cell would
> silently double every limit here.**"*

That is precisely what `remote_gate` does, one directory over, in the same crate.

**Why it is latent and not live.** I checked all 34 call sites
(`grep -rn '\.resolver()\.await'`). Every one is sequential within its command; none holds two
resolvers concurrently. So today the cap holds. **The first concurrent caller multiplies it
silently, with no test to notice** — and the gate's own comment ("bounding them separately
multiplies") shows the author already knew multiplication was the hazard, one level down.

**Fix.** Hoist the semaphore to the `App` (or a process-wide `Lazy`), so it is the run's cap rather
than the object's — which is what `network_parallel` means to a user. Cheap now; removes a trap
rather than a symptom. Fixing **I4** touches the same constructor, so do them together.

---

## R6 — The `locks/` ledgers live outside the directory the data lock covers

**Severity: structural / latent. Confidence: high on the layout, medium on the exposure.**

`DataLock` guards `safe_data_dir()` — the OS data directory (`src/utils/mod.rs:17-28`). The six
ledgers governed by `core::ledger::LockFile` live under `config_root/locks`
(`src/model/layout.rs:195-197`): the regex expansions, the bare-name resolutions, the exec run
counts, the hook approvals, the artifact selections, the applied extras. **Two different trees. The
lock covers one of them.**

`core::datalock`'s own module doc gives the argument that makes this matter:

> *"The lock covers the directory rather than one file: those files must agree with each other,
> and a lock over one of a set that must agree is the same as no lock."*

That applies to the ledgers exactly as to the registry, and they are outside it. Every
`LockFile::save` is a whole-file TOML rewrite — last-one-wins.

**What actually protects them today** — two unrelated mechanisms, neither of which is the lock:

1. `may_record_locks` (opt-in, `false` by default) keeps most resolutions from recording; and
2. the commands that *do* write ledgers — `lock`, `unlock`, `sync` — happen to be `Writer`s, so
   they mutually exclude on the data lock incidentally.

The outcome is currently correct. It rests on a pairing nothing asserts, and **R3 is what it looks
like when one of the two is missing from one ledger.** Expect more: the four ledgers I did not
trace individually (`exec`, `hook`, `extras`, `artifact`) should each be checked for the R3
asymmetry.

**Suggested resolution.** Either bring `locks/` under a lock of its own, or record explicitly — in
`decisions.md` — that ledger writes are protected by "writer scope plus `may_record_locks`", and
add the test R3 asks for. **The status quo is defensible; the status quo being undocumented is what
produced R3.**

---

## R7 — Reader commands take no lock and read files that must agree

**Severity: low. Confidence: high on the mechanism, low on the impact.** Listed so it becomes a
ruled-on non-issue rather than an unexamined one.

`LockScope::Reader` never takes the data lock. A reader reads `registry.json`, `journal.jsonl` and
the `locks/` ledgers as separate, unsynchronised operations while a writer in another process
updates all three.

Each individual file is safe — `registry.json` is written by atomic rename, and the journal's
torn-tail-drop is documented and deliberate. The exposure is *between* them: a reader can observe
post-write `registry.json` and pre-write `journal.jsonl`, or the reverse. `shall status`,
`shall check drift` and `shall list` can report a combination of facts that never existed
simultaneously.

Almost certainly nothing a user will see — the window is milliseconds and the output is advisory.
I flag it because the argument that a *directory* lock is necessary for writers is the same
argument for readers, and only one of the two conclusions was drawn.

**Resolution: rule it, do not necessarily fix it.** "Readers accept a torn cross-file view; a
shared lock on every `shall list` is not worth it" is a perfectly good ruling — but per `CLAUDE.md`
it belongs in `docs/spec/decisions.md` with a status. **This one needs the owner, not a patch.**

---

## R8 — Some mutating verbs take the manager lock and some do not

**Severity: latent. Confidence: high on the asymmetry, low on current impact.** Found by reading
`generic.rs` and the hand-written backends side by side.

`GenericBackendCore::lock_key()` exists so that several backends driving one manager take **one**
lock, and its doc explains what went wrong without it:

> *"OpenBSD installs with `pkg_add` and removes with `pkg_delete`, and keying on the program gave
> those two verbs two different locks over one package database … `pacman` and `yay` in one config
> … were two locks over one database, so a sync touching both ran them concurrently and let
> pacman's own `db.lck` decide, which it does by failing the loser."*

`generic.rs` applies this correctly and symmetrically: install (`:1124`) and removal (`:1245`,
`:1850`) both go through `run_exclusive(self.core.lock_key(), …)`. The hand-written backends do
not agree with each other:

| backend | install / remove | its own `update` / cache-clean |
|---|---|---|
| `flatpak` | `run_exclusive` | `run_exclusive` (`:428`, `:440`) |
| `snap` | `run_exclusive` | `run_exclusive` (`:415`) |
| `brew` | `run_exclusive` | **plain `.run`** — `brew update` (`:262`), `brew cleanup` (`:289`) |
| `mise` | `run_exclusive` | **plain `.run`** — `plugins update` (`:265`), `prune --force` (`:283`) |
| `nix` | `run_exclusive` | **plain `.run`** — cache clean (`:273`, `:287`) |
| `emacs`, `go` | `run_exclusive` | plain `.run` |

`brew update` rewrites taps, `brew cleanup` deletes old versions and cached bottles, `mise prune`
removes tool versions, `nix store gc` collects garbage. Every one of those mutates the manager
state that `install` locks. Two answers to one question, in sibling files.

**Why it is latent and not live.** `managers.rs`'s `update`/`upgrade` issue one call per backend,
so no two brew commands run concurrently inside one process; across processes the `DataLock`
serialises writers. So the outcome is currently correct — for reasons that have nothing to do with
this lock. And `managers.rs:116` is explicit that the design *relies* on it:

> *"the contending set stays strictly sequential and the rest overlap. **`run_exclusive`'s
> per-manager mutex is still underneath both**, which is the safety this loop was being blunt
> about."*

That safety is absent for the verbs above. The partition in `managers.rs` is the thing that would
break first if a future change let two verbs of one manager overlap.

**Second, smaller half.** Those hand-written backends spell the lock key as a **literal**
(`run_exclusive("brew", "brew", …)`) rather than asking `lock_key()`. I checked whether that is a
live bug: `stale_lock`'s families are `pacman/yay/paru`, `dnf/yum/microdnf`, `zypper` and
`apt/apt-get/dpkg` (`stale_lock.rs:101-136`) — none contains brew, flatpak, go, mise, nix or
emacs, so every literal currently equals what `lock_key()` would return. **Correct today, and the
second copy of a table whose own doc says the second copy is the one that goes stale.**

**Fix.** Route every mutating verb through `run_exclusive(self.lock_key(), …)`, including
`update` and the cache cleaners, and delete the literals. Then the rule is "a verb that changes
the manager takes the manager's lock", with no per-backend exceptions to remember.

---

# Inefficiencies

## I1 — `adopt` deep-clones the whole registry to cross a thread boundary

**Severity: medium. Confidence: high.** The exact clone `snapshot()` was created to eliminate,
still live at the highest-*N* call site in the program.

**The evidence is the doc comment on the fix** (`src/core/state.rs:183-188`):

> *"`sync` used to deep-clone the whole registry to hand it to `spawn_blocking` — every
> `ManagedPackage` including its `properties: HashMap`, so a few hundred map allocations to cross
> a thread boundary with data that was about to be serialised anyway."*

`sync` was fixed. `adopt` was not (`src/app/adopt.rs:549-552`):

```rust
let state_to_persist = state_mut.clone();      // <-- the clone the comment describes
recorded = tokio::task::spawn_blocking(move || state_to_persist.save())
    .await…
```

**Why this is the worst place for it to survive.** `adopt`'s whole job is to take ownership of
**every package already on the machine** — hundreds of `ManagedPackage` values, each with a
`properties: HashMap`. It is the largest registry this program ever holds, and it is the one call
site still deep-cloning it. The fix landed on the caller with the smallest *N* and missed the one
with the largest.

**Fix.** `let to_write = state_mut.snapshot()?;` then `spawn_blocking(move || to_write.write())` —
identical to `sync/mod.rs:297`. `adopt` uses `save()`'s "did the bytes reach disk" boolean for its
preview messaging; `StateSnapshot::write()` returns the same `bool`, so `recorded` keeps working
unchanged.

---

## I7 — `essential()` is asked repeatedly per run while `list_installed` is memoised

**Severity: medium. Confidence: high.** A sibling of `core::installed` that never got the memo.

`Queryable::essential()` (`src/core/manager.rs:313`) is a live subprocess query per backend —
"names the OS itself marks as essential". `essential_names` (`src/app/sync/guard.rs:428-472`) runs
them concurrently, correctly honouring `max_parallel`, and its comment says *"This is on every
removal path."* It is right about that, and that is the problem: **there is no memo.**

Call sites, all of which re-run the subprocess set:

- `guard.rs:483` (`inspect`)
- `guard.rs:785` (`inspect_removals`)
- `guard.rs:908` (`preview_refusals`, per kind)
- `guard.rs:964` (`enforce_kind`)
- `app/leases.rs:40`
- `core/transaction.rs:1382` (rollback)

A single `shall sync` with removals goes through the preview path *and* the enforce path, so the
whole set runs at least twice; a rollback makes three. The answer cannot change during a run —
which is exactly the argument `core::installed` makes for `list_installed`:

> *"The answer does not change while nothing is being installed, so it is fetched once. The one
> thing that can change it is a mutating command, and `CommandExecutor::run` forgets these."*

**Fix.** Route `essential()` through the same seam as `list_installed` — an entry in
`InstalledListings` (or a parallel map on the executor), invalidated by the same
`forget_run_scoped_answers`. That gets the correct invalidation for free instead of inventing a
second policy. Fix **R2** first, or the new memo inherits R2's staleness window.

---

## I6 — Lifecycle hooks run serially around a batch that is otherwise concurrent

**Severity: medium. Confidence: high.** Also a doc/implementation mismatch.

`Transaction`'s field doc (`src/core/transaction.rs:150-152`) says:

> *"When set, `before_install`/`after_install` fire per package at the moment it is installed
> (**interleaved with parallel execution**)."*

They do not interleave. Both are plain sequential loops that bracket the concurrent batch:

- `transaction.rs:999-1015` — `for (i, (idx, _, name)) in members.iter().enumerate() {
  h.run_hook("before_install", name).await … }`, **before** the batch runs;
- `transaction.rs:1225-1231` — `for &i in &keep { h.run_hook("after_install", name).await … }`,
  **after** it.

Each `run_hook` is a process spawn (a shebang hook), an mlua eval, or — per **B5** — a fully
synchronous Rhai eval that can block on HTTP. So a batch of *k* packages with hooks pays *2k*
serial hook invocations wrapped around the one part that was made concurrent.

**Why they can overlap.** Each hook is about a different package. `before_install` must precede
*its own* package's install, not everyone else's, and the code already handles per-member failure
by dropping that member from `keep`. A `buffer_unordered` over members, collecting
`(index, result)`, preserves every existing semantic.

**Fix.** Fan both loops out at `max_parallel`, the way every other per-package loop in this tree
does. Then correct the field doc, or make it true — "interleaved with parallel execution" is
currently a description of something that does not happen.

---

## I4 — `App::resolver()` re-reads and re-parses `locks/versions.json` on every call

**Severity: medium. Confidence: high.**

`App::resolver()` (`src/app/context.rs:217-219`) is not memoised — it constructs a fresh
`StateResolver` every time, while its neighbour `App::backends()` (`:202-213`) is a `OnceCell`.
And `StateResolver::new` (`src/app/sync/resolver.rs:147-181`) is not cheap:

1. `try_exists` on `locks/versions.json` (a syscall),
2. `read_to_string` of the whole file,
3. `serde_json::from_str` over all of it,
4. a `HashMap` built from every lock entry,
5. a fresh `Semaphore` (see **R5**).

**34 call sites**, at least three inside loops:

- `src/verbs/packages.rs:79` — inside a nested loop, **once per manifest line**
- `src/verbs/declare.rs:792` — once per named backend
- `src/verbs/plan.rs:757` — once per named manager

(`src/app/inventory.rs:62` builds another directly.) On a machine with hundreds of pins,
`shall install` over a multi-line input re-reads and re-parses the entire pin file for every line.

**Fix.** Memoise on `App` as `backends()` is. Note the resolver borrows `&'a Config` and carries
mutable builder flags (`upgrading()`, `recording_locks()`, `vars_override`), so a naive
`OnceCell<StateResolver>` will not fit — memoise the *expensive shared part* (the parsed locks map,
behind an `Arc`) rather than the whole struct.

---

## I3 — `tool_help` re-probes `--help` concurrently, defeating its own cache

**Severity: medium. Confidence: high.** Check-then-act on a cache, with a process spawn in the gap.

**Where.** `src/core/tool_help.rs`, `help_text`:

```rust
if let Some(hit) = cache().lock().ok().and_then(|c| c.get(&key).cloned()) { return hit; }
… spawn `program --help`, synchronously, via blocking::command_output …
if let Ok(mut c) = cache().lock() { c.insert(key, answer.clone()); }
```

The lock is taken to read, dropped, and taken again to write. The cache's own doc states the goal:

> *"A manager's help does not change while Shall is running, and **an install of forty plugins must
> not launch forty help processes**."*

Concurrently, it can: *k* tasks that miss together all spawn the probe. Reached from the install
argv path (`generic.rs:1085`, `:1105`, `:1215`, `:2988`) — per package, inside the wave. **And each
duplicate stalls the whole wave**, per **B5**.

**Fix — the correct pattern is in this repo twice.** `InstalledListings::once`
(`core/installed.rs`) and `VARS_MEMO` (`resolver.rs:314-323`) both hold a per-key mutex *across the
fetch*, so concurrent askers produce one call. Do the same: a per-key
`Arc<tokio::sync::Mutex<Option<…>>>` rather than a `HashMap` released between check and insert.
Better still, hoist the probe out of the per-package path — the answer is per *binary*.

**Same shape, benign, do not "fix" it:** `core::http::client` (`http.rs:43-49`) is also
check-then-act on a `DashMap`. Two concurrent misses build two `reqwest::Client`s and one is
dropped. Idempotent, cheap, and pooled thereafter. Noted so a family sweep does not treat it as a
defect.

---

## I2 — `github:` and `web:` rewrite their whole state file, prettily, per package

**Severity: low-medium. Confidence: high.** Two parts, both in the R1 pair.

- **Quadratic writes.** Every package installed through these backends reads the entire state file
  and writes the entire state file. Installing *n* packages writes O(n²) bytes and performs *n*
  flushes, for a file that could be written once at the end of the run.
- **Pretty-printing a machine-read file.** Both use `serde_json::to_string_pretty`. The registry
  ruled the other way and wrote down why (`src/core/state.rs:185-188`):

  > *"Compact, not pretty: this is a machine-read registry of every managed package … Pretty
  > printing roughly doubles the bytes for a file nobody opens."*

  Two files in the same data directory, same purpose, opposite answers. Minor alone; free while
  R1 is open.

---

## I9 — Cache cleanup re-walks every cache root once per artifact

**Severity: medium (opt-in). Confidence: high.** Same function as **B8**; separate fix.

`teardown.rs:110-113` cleans a removed package's cached artifacts:

```rust
for basename in &deployed.cached {
    crate::model::cache::clean_cached(basename, cache_dirs).await;
}
```

and `clean_cached` → `find_cached` → `matches_in` walks **every root, from scratch, for one
basename**. A package with five cached artifacts costs five full crawls of `~/.cache` *and*
`/var/cache`; removing twenty such packages costs a hundred crawls of the same two trees, looking
for a different name each time.

**The tree already knows the right shape**, and states it in `planner.rs:590-594` about the
identical problem one layer up:

> *"Asking per package would be one subprocess each; asking per backend is one, and the answer is
> a set."*

Apply that here: walk each root **once**, collect the file names into a `HashSet`, and intersect
with the set of basenames wanted. One pass over the filesystem instead of *N*, with no change to
the matching rule — and the rule is worth preserving exactly as written, since the module's own
doc explains that exact-name-only, files-only, bounded-depth is what keeps a cache clean from
becoming the removal bug this repo is named after.

Do this together with **B8**: one crawl, on the blocking pool, matching a set.

---

## I10 — The guard scans its protection lists three times per package, allocating a String per pattern

**Severity: medium. Confidence: high.** Found only by reading; it has no greppable shape.

`Config::first_match` (`src/config/config.rs:1196-1207`) is the matcher behind every protection
decision:

```rust
fn first_match<'a>(patterns: &'a [String], name_lower: &str) -> Option<&'a str> {
    patterns
        .iter()
        .find(|p| {
            let p = p.to_lowercase();          // <-- allocates, per pattern, per call
            match p.strip_suffix('*') {
                Some(prefix) => name_lower.starts_with(prefix),
                None => name_lower == p,
            }
        })
        .map(|s| s.as_str())
}
```

**Two separate wastes, and the second one is provable.**

1. **`p.to_lowercase()` inside the closure.** The patterns come from `preferences.toml` and do
   not change during a run, yet each is re-lowercased — a fresh `String` allocation — on every
   comparison against every package. This is the shape `utils::regex_cache` exists to fix one
   directory over (*"`Regex::new` builds and optimises an automaton. Eleven places in this tree
   called it inside…"*). Lowercase the lists once, at config load.

2. **`unprotected_packages` is scanned twice per package, and the second scan cannot ever
   return anything.** `guard::protection_of` (`src/app/sync/guard.rs:270-277`) does:

   ```rust
   if config.unprotect_rule(name).is_some() { return None; }        // scan #1: unprotected
   if let Some(rule) = config.protection_rule(name) { … }           // scan #2 + #3
   ```

   and `protection_rule` (`config.rs:1175-1183`) opens by scanning `unprotected_packages` again
   before it looks at `protected_packages`. But reaching that line means `unprotect_rule` already
   returned `None` on the identical list with the identical input — so scan #2 is **dead work
   whose result is known**. Three scans where two would do, and one of the three is provably
   unreachable-to-a-different-answer.

**What it costs.** `inspect_removals` calls `protection_of` once per removal, on every removal
path. For *R* removals against *P* patterns that is `3 × R × P` pattern lowercasings plus `2 × R`
name lowercasings. A 500-package purge against 30 protection rules is ~45,000 throwaway `String`
allocations to answer a question about data that never changed.

**Fix.** Store the lists pre-lowercased (or a `Vec<(String, bool /*is_prefix*/)>` computed once),
and drop `protection_rule`'s redundant unprotected scan — or better, have `protection_of` call a
single function that answers "protected, unprotected, or neither" in one pass. Cheap, contained,
and it removes a duplicated precedence rule that currently lives in two places.

---

## I11 — Set math recomputes an allocating key inside a nested loop

**Severity: medium. Confidence: high.** Also reading-only.

`src/model/resolve.rs:726-728`, the tail of module/profile set resolution:

```rust
for keep in &intersects {
    base.retain(|(s, ..)| keep.iter().any(|k| same_package(k, &set_key(s))));
}
base.retain(|(s, ..)| !subtract.iter().any(|k| same_package(k, &set_key(s))));
```

`set_key` is `stmt.key()` (`resolve.rs:1098-1100`), and `Statement::key()` builds its answer with
`format!` — **it allocates a `String` every call**. Both lines call it *inside* the inner
`.any()` closure, so it is recomputed once per `(base element × candidate)` pair rather than once
per base element.

For a 300-line module intersected against a 300-line one, that is up to 90,000 `String`
allocations where 300 would do. The `subtract` line is worse in the common case: a package that is
*not* being subtracted scans the whole list, so it always hits the worst case.

**Fix.** Hoist it:

```rust
base.retain(|(s, ..)| { let key = set_key(s); keep.iter().any(|k| same_package(k, &key)) });
```

One line, exact same semantics.

**Why not a `HashSet` as well:** `same_package` (`resolve.rs:1107-1122`) is deliberately fuzzy — a
bare `vim` matches `apt:vim`, while `apt:vim` does not match `cargo:vim` — so the linear scan is
not naively replaceable. The hoist is unambiguous and is the whole win; the scan itself is bounded
by the set sizes and can stay.

---

## I12 — 49 nursery-lint hits, one of them in the batch path

**Severity: low. Confidence: high (machine-checked).** Reported for completeness, with proportion.

Default `cargo clippy --all-targets` is **clean**. Enabling the nursery/pedantic lints relevant to
this audit surfaces 49 hits:

- **47 × `redundant_clone`.** ~35 are one identical pattern: the final `.with_metadata_provider(core.clone())`
  in each backend's registration (`backends/registry/language.rs`, `os_native.rs`, `system.rs`,
  `mod.rs:310`, and one per hand-written backend). It is the last use of `core`, so the clone
  could be a move. **Each is one `Arc` refcount bump, ~50 per process** — genuinely negligible as
  performance. Worth fixing as one mechanical sweep because it is a single pattern replicated at
  every registration site, not because it costs anything.
  The remaining dozen are scattered (`model/resolve.rs:855,868,882,896`, `app/adopt.rs:818`,
  `app/sync/resolver.rs:1726`, `backends/onboarder.rs:1155,1307`, `config/config.rs:1485`,
  `core/hook_lock.rs:378`) and are worth a look individually.
- **2 × `needless_collect`**, one of which is in the hot path — `src/core/transaction.rs:939`:

  ```rust
  futures::stream::iter(members.iter().map(|m| m.2.clone()).collect::<Vec<_>>())
  ```

  `stream::iter` takes any `IntoIterator`, so the `collect` materialises a throwaway `Vec` per
  batch, on the path that opens every wave. The `.clone()` per name is what forces it; restructure
  so the stream borrows, or accept the clone and drop the `collect`. The other is
  `verbs/plan.rs:866`.

**Recommendation, stated as a judgement rather than a finding:** consider adding a `[lints.clippy]`
table to `Cargo.toml` pinning the handful that map onto this repo's stated rules — `redundant_clone`
and `needless_collect` at minimum. The tree is already clean under the default set; the cost of
raising the bar now is 49 mechanical edits, and it never rises again. That is a decision about the
project's gates, so it belongs to the owner, not to a commit.

### The full pedantic + nursery run, and the one lint that matters

Running the whole of `clippy::pedantic` + `clippy::nursery` (minus five pure-documentation lints)
produces ~2,700 warnings. **Almost all of it is noise for this audit's purposes** and I am not
reporting it as findings: 1,211 `uninlined_format_args`, 566 `use Self`, 169
`missing_const_for_fn`, 115 "first doc paragraph too long". Enabling those wholesale would be a
bad trade for this codebase. Three are worth naming:

- **`significant_drop_tightening` — 47 hits, and it independently corroborates B2 and B3.** This
  is the closest automated proxy that exists for "a lock guard is held longer than it needs to
  be", and it fires on exactly the sites this audit reached by hand:
  `app/sync/mod.rs:1137`, `:1195`, `:1357` and `core/transaction.rs:1291` (the journal guard —
  **B3**); `verbs/plan.rs:51,234,595`, `verbs/sync.rs:135`, `verbs/setup.rs:857`,
  `verbs/upgrade.rs:607`, `app/adopt.rs:331,525`, `app/insight.rs:694`, `app/inventory.rs:390`,
  `app/profile.rs:399`, `app/shell/mod.rs:270` (the state guard — **B2**'s family);
  `core/installed.rs:199` (**R2**'s slot lock) and `app/sync/resolver.rs:319` (`VARS_MEMO`'s).

  **Two different methods, one conclusion.** That materially raises my confidence in B2 and B3.
  But note what the lint *cannot* see, which is why it is corroboration and not a substitute: it
  flags that the guard could be dropped earlier, structurally. It has no idea that what is held
  across is an `fsync`, and it fires on plenty of benign sites too. It finds the shape; the
  severity still has to be argued. **If you enable exactly one nursery lint, make it this one.**
- **`or_fun_call` — 13 hits, 12 of them one pattern.** `unwrap_or(self.core.binary())` throughout
  `generic.rs` evaluates `binary()` eagerly even when the `Option` is `Some`. `binary()` is
  `self.config.binary.as_deref().unwrap_or(&self.name)` — no allocation, genuinely negligible —
  so this is a tidy-up, not a performance finding. Listed because it is one mechanical sweep in
  the hottest file in the tree.
- **`unused_async` — 14 hits**, including `guard.rs:1067` (`enforce_installs`) and `:1112`
  (`enforce_additions`): `async fn`s with no `.await` in them. Harmless. Mildly informative,
  though — it says the guard's *install*-side gates do no I/O at all, while the removal-side ones
  query `essential()` per backend, which is the asymmetry **I7** is about.

---

## I5 — Small serial loops that the rest of the tree would fan out

**Severity: low. Confidence: high.** Grouped because each is the same one-line fix and none alone
justifies a section.

1. **Snapshot provider selection.** `SnapshotProvider::choose` (`src/core/snapshot.rs:680-686`)
   probes providers one at a time:

   ```rust
   for p in providers { if p.is_available().await { available.push(p); } }
   ```

   Each `is_available` is a real probe — `command_exists` plus a path check
   (`snapshot.rs:381-392`). Small *N*, but it sits before a sync takes a snapshot. A `buffered`
   fan-out preserves the registration order the function already depends on;
   `app::inventory::query_backends_concurrently` is the in-repo pattern.

2. **Per-package property probes.** `GenericQueryable::info` (`src/backends/generic.rs:1448-1451`)
   runs every configured `property_probe` sequentially, each a subprocess, once per declared
   package. The outer fan-out overlaps packages, so this is *P* extra serial spawns inside each
   one rather than a serial run overall. Fan out the probes too.

3. **`check`'s hardcoded 60-second bound.** `src/verbs/check.rs:242` wraps `list_installed` in
   `tokio::time::timeout(Duration::from_secs(60), …)` — a literal, ignoring
   `query_idle_timeout_secs`. The comment justifies the *number* with real measurements, which is
   more than most magic numbers get, and I nearly left it out. But `planner.rs:658` states the rule
   it breaks — *"a cap that ignores the setting is a cap the user cannot move"* — and a user who
   raised the bound for a slow machine still gets `check` failing at 60s with a message blaming the
   manager. Derive it from the configured bound (clamped, if 60 is the right default) rather than
   pinning it.

---

# What I checked and found sound

Part of the deliverable: this is what stops the next pass re-treading the ground, and what answers
"did you look at X".

- **`core::installed`** — the once-per-run listing memo. Correct dedup (fetch inside the slot
  lock), correct refusal to cache failures, correct disk-layer policy (a cached listing may inform
  a report, never source a decision that outlives the run). Only R2.
- **`core::launch`** — PATH memo, the `which` *crate* not the `which` binary, Windows shim
  handling, `forget_path_lookups` after a mutation, the zero-byte app-execution-alias handling.
- **`app::sync::planner`** — `installed_sets` asks each manager once; both fan-outs read
  `max_parallel`; `is_installed` treats unknown as "present" so a sick backend cannot silently
  cancel removals. Best-reasoned file in the audit.
- **`core::prompt`** — the three-outcome confirm (yes / no / nobody is there), `--yes` answered
  first, `on_the_terminal` so a waiting human does not park a worker.
- **`core::http` / `core::download`** — deliberate timeout policy, including the correct
  distinction that a download carries no whole-request timeout while an API call always does;
  chunked writing with a byte ceiling and a pre-flight refusal on a declared length. (The client
  pool's benign check-then-act is noted under I3.)
- **`core::ratelimiter`** — lazily built (a limiter costs nothing on a run with no requests) and
  `Arc<OnceLock>` so clones share one quota. The file that states the principle R5 breaks.
- **`core::supervise` + `RawExecutor::wait_watched`** — children owned and killed on drop,
  SIGTERM-then-SIGKILL with a grace window, the `unsafe` `kill(2)` correctly justified by tokio
  still owning the unreaped pid, **and both pipes pumped by separate spawned tasks** with an idle
  clock driven off last-activity. The drain itself is right; B6 is the ordering *before* it and B7
  is the missing ceiling *inside* it.
- **`core::executor` timeouts** — separate idle bounds for queries and mutations, `sudo -n`
  everywhere, the credential prime under its own bound so no manager invocation sits on a prompt.
  R4 is about the *concurrency* of that prime, not its bounds.
- **`SudoKeepalive`** — a guard type whose `Drop` aborts the task, replacing a detached
  `JoinHandle` that outlived its caller. Correct. (One minor note folded into R4.)
- **`core::datalock`** — the wait is correctly moved off the runtime via `acquire_async`, the 120 s
  constant is justified, and the error refuses to advise deleting the lock file. B1 is not a defect
  in this module; it is a defect in what reaches it.
- **`app::stale_lock`** — the apt/dpkg-versus-pacman distinction is careful, and keeping "may it be
  removed" as a column in the same table rather than a second list is right.
- **`app::pm_hooks` + the `hook-*` stand-down** — generalising from one subcommand to a name test,
  asserted against `pm_hooks.rs` rather than prose, is correct. B1 is the carrier, not this logic.
- **`VARS_MEMO` / `RESOLUTION`** (`resolver.rs:18-41`) — generation counter *in the key*, memo
  cleared on bump, lock held across resolution, provider run via `spawn_blocking`. **No leak in
  `watch`**: I checked specifically, and `new_resolution()` calls `VARS_MEMO.clear()`. **This is the
  model answer for R2.**
- **Bare-name resolution** (`resolver.rs:1055-1182`) — the fan-out returns verdicts and the lock is
  mutated *serially afterwards* in declaration order, then saved once. No intra-call race, and
  determinism is preserved deliberately.
- **`app::sync::guard`** — `essential_names` fans out at `max_parallel` (and its comment names the
  rule); `protection_of` is in-memory; both ceilings are reported together so a raised limit does
  not just meet the next one. Only I7, which is about repetition across calls, not within one.
- **`core::transaction` rollback** — sequential *by necessity* (reverse dependency order), every
  compensating failure reported by name rather than swallowed, and the guard applied to
  execution-time removals that never passed the plan-time gate. Correct as written.
- **Cancellation** — `total_timeout` wraps `execute_internal` in `tokio::time::timeout`, and
  dropping that future reaches every child through `Stopping::drop`. The token is checked between
  batches; mid-command interruption goes through drop, which is the right mechanism.
- **`app::fleet`** — SSH fan-out at `network_parallel`, ordered output, hosts beginning with `-`
  refused so a value cannot become an ssh option, a host that could not answer counted as drifted
  rather than in-sync.
- **`utils::regex_cache`** — bounded at 1024 with a deliberate clear-don't-evict policy, and a
  comment explaining that `watch` is what made "never evicted" stop meaning "bounded".
- **Archive extraction and SHA-256 hashing** — `utils::archive::extract_archive` and
  `core::security` are called through `spawn_blocking` at every site (`github.rs:795`,
  `web.rs:265`, `bundle.rs:285`); hashing is streamed, not read into a `Vec`.
- **Atomicity of individual writes** — `utils::file::atomic_write` does temp + `sync_all` + rename.
  No torn-file races found anywhere.
- **TOCTOU** — 129 `exists()` / `try_exists` sites sampled across `setup`, `shim_manager`,
  `locate`, `utils::file`. `shim_manager` deliberately checks ownership before overwriting (S4)
  with a test asserting it; `ensure_dir` correctly dropped its `if !exists` guard. Nothing found.
- **`env::set_var`** — the data-race class. Every call outside tests is `main.rs:778`
  (`settle_data_dir`, pre-clap, before the runtime does any work) and `git.rs:594` (a
  `#[cfg(test)]` helper behind a `std::sync::Once`). Both fine.
- **`Drop` impls** — all lightweight (an unlink, a `try_wait`, an `abort`). Nothing blocking.
- **DashMap re-entrancy** — checked for the classic deadlock (a live `Ref` held across a second
  access to the same map). None found; every `get` result is cloned out before any insert.
- **Semaphore / DAG execution** — `max_concurrent` derives from `max_parallel`, permits are owned,
  none held across an unbounded wait.
- **`app::inventory`** — has its own `query_backends_concurrently` fan-out; not serial despite
  containing no `buffer_unordered`.
- **`app::scheduler`** — writes **one file per schedule** (a systemd unit + timer, a launchd
  plist, a Task Scheduler XML) through `write_atomic`. I went looking specifically for a
  crontab-style shared-file read-modify-write, which would have been an R1 sibling with teeth.
  There is none: the per-schedule file layout makes the lost-update class structurally impossible,
  and the drift readers are careful not to report a field Shall does not write.
- **Duplicate model resolution.** `resolve_model` is the most expensive operation in the program
  (dozens of file reads plus the bare-name network fan-out) and is **not** memoised, so I traced
  all 17 call sites. Every one is a distinct command entry point — `check_summary`, `check_config`,
  `handle_eval`, `handle_absent`, `handle_status`, `handle_canary`, `handle_policy` and so on — and
  none resolves twice in one invocation. `plan.rs:255` records that a double-resolve *did* exist
  here once and was fixed. It has not come back.
- **`core::git`** — entirely synchronous by design, reaching the terminal through
  `blocking::command_output`, and it runs after a sync rather than inside a wave. `git_available()`
  spawns `git --version` unmemoised, but has exactly two call sites, once each per command.
- **`app::diagnostics`** — the rule regexes are pre-compiled into the rule (`diagnostics.rs:25`),
  not built inside the match loop; the comment says that is deliberate.
- **The feed-before-drain family is complete.** `supervise.rs:184-188` is the *only* place in the
  tree that writes to a child's stdin (`grep` for `stdin.take()` / `write_all` on a pipe returns
  one production site). B6 is one bug, not the first of several.
- **`config/grammar/statement.rs`, read end-to-end (2,196 lines).** A pure, synchronous parser and
  validator: no I/O, no shared state, no `async`. It therefore cannot hold a race or a blocking
  defect, and its constant factors (an `Origin::clone` per error, a `KEYWORDS` scan per line) sit
  on error paths or are bounded by a 23-entry list. The dispatch ordering is carefully argued —
  set-expression detection deliberately runs *after* statement prefixes and known-backend
  detection, so a `link:C:\…` path full of backslashes is not eaten by the set-math grammar.
  Nothing to report.
- **`Statement::key` / `phase` / `kind` exhaustiveness.** The grammar makes "which phase does this
  run in" a compile error rather than a list — a statement kind added later cannot ship without
  answering. This is the mechanism that would have prevented several of the historical bugs the
  comments describe, and it is applied consistently.
- **`model/resolve.rs` variable-scope check** builds a `HashSet` for the top-level lookup
  (`resolve.rs:374-384`) rather than scanning — the right shape, and the contrast that makes I11
  two functions away stand out.
- **`Reaped` (guard.rs:46-77)** — the removal-authorisation token with a private field, so an
  effector cannot remove without having asked, and `Reaped::for_reason` is a greppable list of the
  deliberate exemptions. This replaces a source-regex enumeration with a compiler-enforced one, and
  it is the strongest single safety mechanism I found in the tree.
- **`app::scheduler`** — writes one file per schedule through `write_atomic`; I went looking
  specifically for a crontab-style shared-file read-modify-write (an R1 sibling with teeth) and
  there is none. The per-schedule file layout makes that class structurally impossible.
- **Lint suppressions.** 18 `#[allow(…)]` in the whole tree, none of them hiding a concurrency,
  clone or collect lint; no `[lints]` table and no crate-level `deny`/`allow`. The clean default
  clippy run is genuine, not configured silence.

---

# The spec pass: what these findings are, in the repo's own terms

I read `docs/spec/` **after** the code, which was the wrong order and is corrected here. Three
results, and the first one changes how most of the blocking section should be read.

## Five findings are not suggestions — they breach a written target-state rule

`target-state.md` **II.52** (`Q52`, why-entry `V.182`) says, in its own words:

> **Blocking waits do not sit on a runtime worker.** A confirm at a prompt, a TUI event loop, the
> data-directory lock's two-minute poll: each of them parked a tokio worker for its whole
> duration. `core::blocking` is where that is decided — `on_the_terminal` where the call cannot
> move, `off_the_runtime` where the work can.

**B2, B3, B4, B5 and B8 are each a violation of that sentence.** They are not engineering
preferences I am recommending; they are the rule the repo wrote for itself, unapplied. That
should raise their priority relative to how the first draft of this document framed them.

**And II.52 tells you exactly why they survived.** The rule has two halves and closes with:

> **A gate, not a sweep.** `tests/a_spawned_child_has_an_owner_tests.rs` fails on a `Command`
> that reaches `spawn`/`output`/`status` outside the executor unless it goes through a door or
> sits in an exemption table with a sentence. Fixing seventeen sites fixes seventeen sites; the
> gate is what stops the eighteenth.

I read that gate. **It scans for `Command::new` reaching `.spawn()` / `.output()` / `.status()`,
and nothing else** (`a_spawned_child_has_an_owner_tests.rs:88-98`). It gates the *process*
half of II.52 completely and the *blocking-wait* half not at all. Nothing in the tree enumerates
an `fsync` under a lock, a synchronous `walkdir` inside an `async fn`, or a synchronous
interpreter eval on a shared task — and `off_the_runtime`, the door II.52 names for "work that
can move", **has exactly one caller in the whole codebase** (`datalock.rs:42`).

V.182 makes the argument for closing this better than I can:

> *A list of sites fixed is a fact about one afternoon; a predicate that fails the build is a
> fact about every afternoon after it.*

**So the recommendation is not "fix five sites."** It is: write the second predicate. A gate that
fails on a durable write, a filesystem walk, or a `std::process::Command` reached from an `async
fn` without `spawn_blocking` / `off_the_runtime` — with the same exemption-table-plus-sentence
design the first gate already uses. B2, B3, B4, B5 and B8 are its first eight or so hits; the one
after that is the finding this audit did not make.

## S59's next layer: what B2 and B4 add to a bug already fixed once

`bugs.md` **S59** (FIXED 2026-08-09) found that of four atomic writers, two had no `fsync`. The
fix unified durability into `utils::file::durable_write` and — characteristically — replaced the
comment with a scan (`a_writer_that_reaches_the_disk_goes_through_one_tests.rs`).

That fix is about **which function does the writing**. B2 and B4 are about **the state the caller
is in when it calls it**: the global state mutex held, on a runtime worker, inside a concurrent
wave. S59's gate passes them because they do go through the one door. Worth saying explicitly in
whatever commit fixes B2, so it reads as the completion of S59 rather than a contradiction of it.

## Nothing here is already known

I checked the whole register rather than assuming. Searching `docs/spec/` for every mechanism in
this report — `SHALL_INSIDE`, `env_reset`, `preserve-env`, pipe buffers, `load_state`/`save_state`,
`internal_lock`, `first_match`, `set_key`, `remote_gate`, `RegexLock`, lost updates — returns
**zero hits** for all of them. `decisions.md` holds 13 `OPEN`/`DEFERRED` entries and none is
related. The near-misses are the *principles* these findings violate, not records of the findings:
`target-state.md:1338`'s *"two whole writes are last-one-wins"* is the argument for the data lock
that **R1** reproduces one layer below it, and `regex.toml` appears only as a feature description,
never as **R3**.

**All 27 are new.** Two things follow: the register does not need amending before work starts, and
`decisions.md` should gain entries for the three owner items when they are ruled.

## One thing I nearly reported and killed

`core/datalock.rs:1` cites `(II.8)` for "one writer at a time on the data directory", and `II.8`
in `target-state.md` is headed **Commands** — a table of subcommands. That looks exactly like the
stale-citation class `a_citation_in_a_comment_still_points_at_its_claim_tests.rs` exists for. It
is not: `II.8` runs from line 1251 to past 1360 and contains both the table and the *One writer at
a time (V.61)* paragraph at line 1333. **The citation is correct.** Recorded because the next
person will have the same suspicion, and because it is the fourth time in this audit that
verifying a promising finding killed it.

---

## NEW — Part II describes a locking model the code deliberately no longer has

**This is an owner item, not a patch.** `CLAUDE.md`: *"Anything where Part II looks wrong. **Do
not fix Part II yourself.**"*

`target-state.md` II.8 (line 1333) states:

> **One writer at a time (V.61).** Every command that mutates state takes an exclusive lock on the
> data directory **for its whole run**, and a second one waits or says who holds it.

The code has had three lock scopes since `LockScope` was introduced, and the change was correct
and well argued (`cli/args.rs:1027-1119`). `Writer` holds it for the run; **`Deferred` does not**,
and the reason it does not is a real bug that was fixed:

> *"`watch` is an unbounded `loop` — the GitOps daemon, meant to be left running… Held for the
> run, it disabled `install`, `sync` and the `hook-reconcile` a hand-typed `apt install` fires,
> for as long as the daemon was up. **The user who followed the documented deployment bricked
> their own CLI.**"*

So `watch`, `shell`, `run` and `history` mutate state and do **not** hold the lock for their whole
run — deliberately. And `Reader` commands take no lock at all.

**`LockScope`, `Deferred` and `Reader` appear nowhere in `docs/spec/`.** Part II still describes
the pre-`LockScope` model.

**Why it matters beyond bookkeeping.** Read Part II alone and you would conclude that anything a
mutating command touches is covered by the lock for the duration — which is exactly the belief
that makes **R6** (the `locks/` ledgers live outside the locked directory) and **R7** (readers take
no lock and read files that must agree) look like non-issues. The spec's model is what a reviewer
checks a design against; here it is more protective than the code, which is the direction that
hides findings rather than raising false ones.

**What I think the owner should decide:** whether II.8 gains the three-scope model as a rule (with
`Deferred`'s justification, which is strong and already written), and whether **R6** and **R7** are
then answered by that rule or need their own. Those three questions are one question, and
answering them separately is how they drifted apart in the first place.

---

# Why the suite is green: what it gates, and the shape it cannot see

**This is the most useful section for whoever fixes these findings.** The test suite is 137 files
and ~35,800 lines, and it is genuinely good — better than most. Every top finding above passes it
anyway, and not by accident: they all share one shape the suite is structurally blind to.

## The suite's best mechanism, and its blind spot

There is a whole genre here of **source-scan enumeration gates** — tests that walk `src/`, find
every site of a kind, and require each one to go through a single door or be listed in a ledger
with a written reason. `removal_guard_enumeration_tests.rs`, `wal_enumeration_tests.rs`,
`a_spawned_child_has_an_owner_tests.rs`, `a_writer_that_reaches_the_disk_goes_through_one_tests.rs`
and `fanout_cap_reads_the_setting_tests.rs` are all this shape, and each states its own rationale:

> *"The defect is a path that exists and is not covered; no behavioural test can enumerate the
> paths nobody wrote a test for — that is the shape of the bug."* — `wal_enumeration_tests.rs`

That is a strong idea, well executed. **But every one of these gates asks "does this site go
through the door?" and none asks "is the door used correctly?"** — and every serious finding in
this audit is a *correct call to a correct door, made at the wrong moment or in the wrong scope*:

| gate | what it asserts | why the finding passes |
|---|---|---|
| `a_writer_that_reaches_the_disk_goes_through_one` | every durable write goes through `durable_write` | **B2, B4** do. It says nothing about whether the caller holds a mutex or a runtime worker while it happens. |
| `fanout_cap_reads_the_setting` | no fan-out hard-codes a literal width | **R5**'s `Semaphore::new(config.network_parallel.max(1))` reads the setting perfectly. The gate cannot see that the semaphore is constructed 34 times. |
| `a_spawned_child_has_an_owner` | every child is owned and bounded | **B6**'s child is owned and bounded. The bug is that the feed is written *before* the drain starts, so the bound is not armed yet. |
| `wal_enumeration` | every package mutation is recorded before it runs | **B3** records every mutation. The gate does not ask whether the record costs one flush or *k*. |
| `hook_reentrancy` | every `hook-*` subcommand stands down under `SHALL_INSIDE` | **B1** — see below. |
| `dry_run_every_verb::a_preview_leaves_the_config_byte_identical` | `--dry-run <verb>` leaves the config untouched | **R3** is a `Reader` writing *without* `--dry-run`. The test's own non-vacuity control demands the verb change something, so no `Reader` can ever be one of its cases. See R3. |
| `fanout_cap_reads_the_setting` (again) | *value* of every cap comes from config | Nothing checks a cap's **scope**. See R5. |

**One sentence to hand to whoever writes the next gate: the suite polices the seam, not the use of
the seam.** A sixth enumeration gate of the same design will have the same blind spot.

## `hook_reentrancy_tests.rs` — the sharpest instance

This test is excellent at what it does. It enumerates every `hook-*` subcommand from clap's own
metadata, drives each one against a held `DataLock`, and asserts a stand-down inside 10 seconds;
a second test walks `pm_hooks.rs` in the other direction so a hook body calling something else
fails too. It is the exact shape a reviewer would ask for.

And in `run_bounded` it does this:

```rust
.env("SHALL_CONFIG_DIR", dir.join("config"))
.env("SHALL_DATA_DIR", dir.join("data"))
.env("SHALL_INSIDE", "1")        // <-- the test sets the marker itself
```

**It hands the hook the marker directly.** Production hands it through `sudo`, which strips it
(**B1**). The test verifies the second half of the chain — *given the marker, does the hook stand
down?* — and assumes the first half, which is the half that is broken. Nothing in the suite spawns
anything through `sudo`, and the container harness runs as root where `escalates()` is `false`, so
the carrier is untested on both instruments at once.

## The other reason the races are invisible: nothing simulates duration

`MockExecutor` has a `delays: DashMap<String, Duration>` and a `set_delay` method, described in
its own field comment as *"How long a command takes, for tests about concurrency rather than about
output"* (`core/executor.rs:713`).

**It is used in exactly one test file, on exactly two commands** —
`tests/the_engine_runs_the_graph_in_order_tests.rs:152-153`, on `brew list` and
`cargo install --list`. Both are *read* commands, in a test about graph ordering.

**No mutating command anywhere in the suite is given a non-zero duration.** That is the precise
reason R1, R2 and R4 can be real and still never fire:

- **R1** needs one task to be inside its download while another loads the same state file. Mocked,
  the "download" returns in microseconds, so the window is essentially closed.
- **R2** needs a task parked on the slot mutex for the length of a real `winget list` while a
  mutation invalidates the memo. Mocked, the fetch is instant.
- **R4** needs several escalated commands to reach the sudo prompt together. Mocked, nothing
  escalates at all.

I checked whether concurrency is simply switched off in tests, and **it is not** —
`default_max_parallel()` is `available_parallelism()` (`config.rs:770-774`), so a test on a default
`Config` really does run its wave concurrently. The concurrency is *on*; it is just never *slow*,
and these races need width, not parallelism.

**This is the cheapest actionable recommendation in the whole audit:** the instrument already
exists. Give two `github:` installs a `set_delay` and R1 reproduces deterministically, in-process,
with no network and no container. The same trick reaches R2. Neither needs new infrastructure —
only for `set_delay` to be pointed at a mutating command for the first time.

## Gates worth adding

Each closes a finding *and* the class it belongs to:

1. **A `Reader` subcommand leaves the config root byte-identical.** Closes **R3**. Write it as the
   sibling of `a_preview_leaves_the_config_byte_identical`, whose non-vacuity control structurally
   excludes every `Reader` — so this needs its own control (a planted write the harness must see),
   not a new case in the existing table.
2. **A hook spawned through `sudo` still stands down.** Closes **B1**'s carrier. Needs the non-root
   harness leg below.
3. **A hook fed >64 KiB on stdin completes.** Closes **B6**, and enforces a constraint that is
   currently only a sentence in a doc comment.
4. **A cap is shared, not just correctly sized.** Closes **R5** — the sibling assertion to
   `fanout_cap_reads_the_setting`: a `Semaphore` constructed inside a short-lived struct is a cap
   nobody shares.
6. **The second half of II.52: a blocking wait does not sit on a runtime worker.** The biggest of
   these, and the one the spec already demands — see "The spec pass" above. A gate on durable
   writes, filesystem walks and `std::process::Command` reached from an `async fn` without
   `spawn_blocking`/`off_the_runtime`, built like the first half's gate. Closes **B2, B3, B4, B5,
   B8** as a class rather than as five patches. `clippy::significant_drop_tightening` is a usable
   first approximation and already flags most of them (see I12).
5. **`set_delay` on a mutating command, with two packages in one wave.** Closes **R1**, and makes
   the whole R-class reachable for the first time.

## And the harness

- **The container harness runs as root.** `escalates()` returns `false` there, so the entire
  `sudo` path — argv construction, env propagation, credential priming — is never exercised by
  either instrument. A non-root leg with passwordless sudo covers **B1** and **R4** at once, and
  it is the single highest-value change to the test apparatus in this report.

---

# Adjacent observations

Found during the sweep, outside the three questions asked. Recorded rather than dropped, and
explicitly *not* counted among the 27 — judge them on their own merits.

- **`core::git` bypasses the launcher.** `git.rs` builds `std::process::Command::new("git")`
  directly in three places, never going through `core::launch::effective_command`. `tool_help.rs`
  documents why that matters: *"Through the executor's launcher, or a shimmed manager on Windows
  cannot be run at all — the mistake the argv-drift gate made for four installed managers before it
  was fixed."* Git on Windows is normally a real `git.exe`, so this is latent rather than broken —
  but it is the same family, and it also means these spawns miss the `PATH_LOOKUP` memo.
- **`Transaction`'s hook field doc is wrong**, independent of whether I6 is fixed: it claims
  `before_install`/`after_install` are *"interleaved with parallel execution"* and they are not.
  If the serial loops are kept deliberately, the sentence still has to change.
- **`core::blocking::off_the_runtime`'s doc oversells it.** It lists "unpacking an archive, hashing
  a file, waiting out a file lock" and has one caller; the archive and hash paths reach
  `spawn_blocking` directly. Fixing B8 through `off_the_runtime` would make the doc true again,
  which is the tidiest resolution.

---

# Suggested order of work

1. **B1** — a deadlock on the ordinary Linux configuration; the premise is a two-second `printenv`
   before it is a container run.
2. **B6** — an unbounded hang with no timeout armed, and the failing test is easy to write.
3. **R3** — a reader writing unlocked state; hermetic reproduction; a one-line gate plus a test.
4. **R1 (+ B4, I2)** — data loss; hermetic test; twin fix in two files.
5. **B2** — eleven mechanical call sites, pattern already in the tree, zero design risk.
6. **I1** — one line, deleting a clone whose removal is already documented.
7. **R4** — a mutex around the sudo prime.
8. **B3 part 1** (batch the flushes) — free. **B3 part 2** → a `decisions.md` entry, not a patch.
9. **R2**, then **I7** — the generation counter first, so the new memo does not inherit the bug.
10. **I6, I3, I4 (+ R5)** — fan-outs and memoisation the tree already knows how to do; I4 and R5
    touch the same constructor.
11. **B8 + I9** — one change to `model::cache`: one crawl, on the blocking pool, matching a set.
12. **I10, I11** — two contained fixes found only by reading: pre-lowercase the protection lists
    and drop the dead second scan; hoist `set_key` out of the inner loop. Neither touches a design.
13. **B5, B7, I5, I12** — structural cleanups; do them while touching the surrounding code.
14. **R6, R7, R8** — rulings and a consistency sweep, not urgent patches.

**Before any of it, read "Why the suite is green" below.** Five of these findings have a passing
gate sitting next to them, and the gate will pass again after a narrow fix. The section names the
assertion each one needs instead.

# What needs the owner rather than a commit

Per `CLAUDE.md`'s four-item stop list:

- **B3 part 2** — whether `record_success` flushes per package. A durability trade-off is behaviour
  a user notices after a crash.
- **R7** — whether reader commands accept a torn cross-file view. Either answer is defensible; it
  needs a status in `decisions.md`.
- **R6** — whether `locks/` should come under a lock, or whether "writer scope plus
  `may_record_locks`" is the ruled protection. R3 is what that pairing looks like when half of it
  is missing, so it deserves an explicit answer rather than an inherited one.

Everything else is a bug against documented intent, an implementation detail, or a mechanical
application of a pattern already ruled on — all of which `CLAUDE.md` says to build without
stopping, with the reasoning in the commit message.

# The harness

Folded into **"Why the suite is green"** above, which supersedes this section — it names, per
finding, the gate that passes today and the assertion that would not. The one item worth repeating
here because it is the highest-value change to the test apparatus in this report:

> **The container harness runs as root.** `escalates()` returns `false` there, so the entire `sudo`
> path — argv construction, env propagation, credential priming — is exercised by neither
> instrument. A non-root leg with passwordless sudo covers **B1** and **R4** at once.

# The one-sentence version

The performance architecture here is finished and good, and the tree is clean under its own
clippy gate; what is left is that six correct rules —
`fsync` off the runtime, hold the lock across the read-modify-write, carry the re-entrancy marker
to every descendant, never let a preview write state, drain the child before you feed it, and ask
once for a set rather than once per member — are each applied at some call sites and not at their
siblings, and the sibling that was missed is, in nearly every case, the one with the largest N or
the most common configuration.

**And the one sentence about the tests, which is the other half of the report:** every serious
finding here is a correct call to a correct door made at the wrong moment or in the wrong scope,
and the suite's enumeration gates all ask whether the door was used — never whether it was used
right. That is why 27 findings sit under **616 passing tests and a clean linter**, both of which
I ran rather than assumed.
