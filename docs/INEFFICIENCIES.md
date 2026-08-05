# Inefficiencies

**A from-the-code audit of everything in this tree that is slower than it has to be.**
Read as a work order: each finding names the file, the mechanism, the cost, and the fix.

Audited at `320bd5e`, 2026-08-02. 85,383 lines across `src/`.

---

# Disposition — what was built, 2026-08-02

**The owner ruled the whole document on 2026-08-02**: *as parallel as possible, as efficient as
possible, as fast as possible; restructure if it takes that.* The four findings marked
**[RULING]** whose behaviour a user notices are in the register as `Y1`–`Y4`, the rules in
`spec/target-state.md` **II.19**, the reasons in `spec/why.md` **V.115–V.118**.

**Every finding in this document is now one of three things: FIXED, FIXED BY (something else), or
NOT DONE with the reason.** Nothing is left unaccounted for, because an audit with an unmarked
entry is an audit nobody can finish.

| tier | finding | state |
|---|---|---|
| 0 | **I-1** one node per package, batch machinery dead | **FIXED** — the DAG's ready set is grouped by manager and kind; one command per group, bounded at 100 names / 6000 bytes. `Y1`, V.115. Tests: `core::transaction::batching_tests` (5). |
| 0 | **I-2** `run_exclusive` serialises what the DAG parallelised, and the telemetry lies | **FIXED** — batching is what makes the per-manager mutex free, and the breakdown now says when packages shared a command. The blocking `flock` moved to the blocking pool; `open_exec_lock`'s `create_dir_all` runs once per directory per process. |
| 0 | **I-3** `info(name)` = list the machine, in 18 backends | **FIXED** — `Queryable::list_installed` is a memoised trait default over a per-backend `fetch_installed`, held on the executor so it is scoped to the run. The post-install `info()` (I-3's item 1) is deleted outright: its only consumer was a `download_size` **no backend produces**. Item 3 (a targeted `has()`) is **NOT DONE** — see below. |
| 0 | **I-3b** journal rewritten in full, pretty, under one mutex | **FIXED** — append-only `journal.jsonl`, one line per transition; `compact()` only where a removal cannot be expressed as an append. |
| 0 | **I-4** `PackageCache` built every run, never read | **FIXED by deletion** — `core/cache.rs` is gone. The right thing (`core/installed.rs`) is process-lifetime, not TTL'd, which is the correct semantics for a one-shot CLI. |
| 0 | **I-4b** eight HTTP clients, one per request | **FIXED** — `core/http.rs`, one pooled client per distinct policy (user agent × redirect × timeout). |
| 0 | **I-43** the model resolved up to 3× per command | **FIXED** — variables memoise per invocation (IX.6), keyed by repo and provider. `watch` bumps the resolution at each tick, because a clock-reading provider *must* answer freshly there. |
| 0 | **I-44** `remote_has` has no implementations and its caller searches again | **FIXED by deletion** — `remote_has`/`remote_info` are replaced by one `lookup`. |
| 0 | **I-45** linear scans over state, one allocating in the fan-out | **FIXED** — `is_held` compares in place; `installed_but_unmanaged` asks `managed_index()`. |
| 0 | **I-46** managers asked expensive questions | **PARTLY** — the `winget`/`choco`/`emacs` cases are closed by I-3's memo, which is what this finding said they needed. `zypper`→`rpm -qa` and `vscode`'s Electron listing are **NOT DONE**: both change what a backend reports, and the argv-drift gate has no fixture for either. |
| 0.5 | **I-32** model resolution synchronous | **FIXED** — the vars pass and both model passes run on the blocking pool. |
| 0.5 | **I-33/I-34** checksums block a worker | **FIXED** — `verify_checksum`/`generate_checksum` are async over `spawn_blocking`; the planner's two template hashes run concurrently. |
| 0.5 | **I-35** ~51s Windows restore point as a silent barrier | **FIXED** — starts first, joined immediately before the first mutating command, aborted on a refusal, and announced. `-NoProfile -NonInteractive` added. `Y4`, V.118. |
| 0.5 | **I-36** state deep-cloned to save it | **FIXED** — `snapshot()` serialises under the lock; the string crosses the thread boundary. |
| 0.5 | **I-37** 48 synchronous `stat`s in `async fn` bodies | **NOT DONE** — see below. |
| 1 | **I-5** `AppCore`/`AppServices` dead duplicate | **FIXED by deletion.** |
| 1 | **I-6** two `command_exists`, the slow one dead | **FIXED by deletion** — `utils/command.rs` removed. |
| 1 | **I-7** dead dependencies | **FIXED** — `rayon`, `nonzero_ext`, reqwest's `blocking` dropped; tokio narrowed from `full`. `mlua`/`rhai`/`tera` are a feature decision and untouched. |
| 1 | **I-8** a second HTTP client spawning a thread per request | **FIXED** — `http()` variables use the shared pool. |
| 2 | **I-9** `upgrade` serial next to its parallel twin | **FIXED** — root-needing managers serial, the rest overlapped. `Y2`, V.116. |
| 2 | **I-10** `fleet` one host at a time | **FIXED** — both passes fan out at `network_parallel`. |
| 2 | **I-11** `check health` probes ~55 backends serially, twice | **FIXED** — one concurrent pass shared by the rollup and the detail view; the two O(n²) scans beside it are indexed. |
| 2 | **I-12** `adopt` scans serially, twice | **FIXED** — both crawls concurrent and overlapped with each other. |
| 2 | **I-13** the priority chain one declaration at a time | **FIXED** — all chains at once, verdicts applied in declaration order so the lock file is byte-identical. The O(n²) `questions.iter().find` is indexed. |
| 2 | **I-14** `registry.available()` re-probes PATH at 20+ sites | **FIXED** — memoised in the executor, invalidated after any mutating command. flatpak's redundant per-backend cache deleted. |
| 2 | **I-15** essentials asked serially, and once per package | **FIXED** — concurrent, and hoisted out of `purge-unmanaged`'s per-package loop. |
| 2 | **I-16** OSV advisories one request at a time | **FIXED** — deduped then fetched concurrently over the pooled client. |
| 2 | **I-17** dependency expansion serial | **FIXED** — concurrent and memoised, so the planner's two passes ask once. |
| 2 | **I-18** other serial loops | **MOSTLY** — orphan listing, cache cleaning, reachability, health probes and `web`'s HEADs are done. **NOT DONE:** `github.rs`'s per-pick asset downloads, `apply/prereq.rs`'s triple-nested probes, `apply/dotfiles.rs`, `export.rs`'s per-format `try_exists`, `shell/mod.rs`. See below. |
| 3 | **I-19** one number doing three jobs | **FIXED** — `network_parallel`. `Y2`, V.116. |
| 3 | **I-20** three hardcoded caps | **FIXED** — all three read a knob. |
| 3 | **I-21** `search` bounded by its slowest backend | **FIXED (2 of 3)** — per-backend deadline, and the two micro-costs (the `format!` dedup key, the per-comparison `to_lowercase`). **Streaming results as they arrive is NOT DONE** — see below. |
| 3 | **I-22** eager serial startup | **PARTLY** — the four independent I/O operations overlap via `try_join!`, and the diagnostics DB compiles its regexes once. The `rhai::Engine` is still eager and the diagnostics DB is still read on every run: both are **NOT DONE**. |
| 3 | **I-23** config parsed twice | **NOT DONE** — see below. |
| 3 | **I-24** regexes recompiled per call | **FIXED** — `utils/regex_cache.rs` for patterns that come from configuration; `Lazy` for the two that are literals. All eleven sites. |
| 3 | **I-25** `which::which` on every Windows spawn | **FIXED** — the launch path is memoised, so the `.ps1` stat goes with it. |
| 3 | **I-26** state written pretty-printed | **FIXED.** |
| 3 | **I-27** up to 998 stats to pick a filename | **FIXED** — one `read_dir`. |
| 3 | **I-38** three lowercase copies of every command's output | **FIXED** — built once, and not at all for a policy with no markers. |
| 3 | **I-39** download backends rewrite their state per package | **FIXED BY I-1** — `save_state` was always called once per `install()` call; batching makes that one call per wave. The pretty-printing is **deliberately kept**: these files hold a dozen entries and are the thing a person opens when a `github:` install goes wrong. |
| 3 | **I-40** reachability spawns a subprocess per backend serially | **FIXED.** |
| 3 | **I-41** `generate:` one script at a time | **FIXED** — refusals still checked first and in declaration order, so which one you are told about is deterministic. |
| 3 | **I-42** O(n²) `Vec` scans | **PARTLY** — the four on hot paths are done (`check.rs` ×2, `resolver.rs`, and state's). The `model/` and `verbs/` clusters are **NOT DONE** — see below. |
| 3 | **I-47** health checks serial, port probe unbounded | **FIXED** — concurrent, and bounded at 5s with the reasoning written down. |
| 4 | **I-28** allocation counts | **PARTLY** — `get_filtered`'s per-comparison `String`, `sanitize`'s three allocations, and `search`'s two are done. `Package::properties` and the `format!("{}:{}")` map keys are **NOT DONE** — see below. |
| 4 | **I-29** the scheduler rescans the whole graph per completion | **FIXED** — an in-degree counter with a ready queue, which also fixes the latent `in_progress` leak the finding noted. |
| 5 | **I-30** build profile correct | nothing to do, and now with a narrower tokio. |
| 5 | **I-31** a 3 MB dump inside `src/` | **FIXED by deletion** — there were two, `src/` and `tests/`. |

**Added 2026-08-05**, after the audit above, and marked so this table stays complete:

| tier | finding | state |
|---|---|---|
| 2 | **I-48** `heal` bypasses the DAG — serial, one package per command, and doomed | **NOT DONE** — measured 205.14s wall, 0.2x overlap, 27 waves against the DAG's 3.9x / 2 waves on the same host. The fix is to route recovery through the transaction engine, not to parallelise a second copy of it; the loop deciding *which* entries to run is `Q33`, which is OPEN and is the same loop. **They land together.** |
| 2 | **I-49** `github:`/`web:`/`appimage:` download and extract, then ask whether they may deploy | **NOT DONE** — 180 of one `heal`'s 205s. The refusal is already a pure function of the destination; hoisting it above the fetch is the whole change. Raised as `Q37` because it changes when a command fails. |

## What was not done, and why

Stated because an audit that quietly drops entries is worse than one that never listed them.

- **I-1's sixteen hand-written backends.** The DAG hands each of them the whole group now, so
  the batching win is theirs to take — and taking it means deciding, per manager, that its
  command really accepts a list. It mostly does and it does **not** always: `pipx install` takes
  exactly one package, and `code --install-extension` takes one per occurrence of the flag.
  Blanket-converting sixteen `for name in names` loops on the strength of "every one of these
  managers accepts multiple names on one command line" would have broken at least pipx, and the
  argv-drift gate checks subcommands and flags, not arity. They are no worse than before — the
  same number of manager invocations, now inside one node instead of across N — and each one is
  a small change with a fixture attached.
- **I-3 item 3, a targeted `has(name)`.** The memo makes `info` a set lookup after one listing
  per manager, which is the same asymptotics for a run and costs no per-backend work. A targeted
  query would beat it only for a command that asks about one package and exits — `linix info`
  — and adding one per backend is thirty argv decisions each needing a fixture in the drift gate.
  Worth doing; not worth doing blind.
- **I-46's `zypper` and `vscode`.** Both change *what a backend reports*, not how fast it
  reports it: `rpm -qa` and `zypper search --installed-only` do not return identical sets, and
  reading `~/.vscode/extensions` off disk is a different question from asking `code`. A
  performance pass is the wrong change to smuggle those into.
- **I-37, the 48 synchronous `stat` calls.** Individually microseconds, and the finding says so.
  The four that were on the fan-out's task (I-25, I-32, I-33, I-34) are fixed; the rest are in
  `bundle.rs`, `cleanup.rs`, `setup.rs` and friends, where nothing is waiting on them. A
  mechanical sweep of 48 call sites for microseconds is churn with a real chance of a behaviour
  change.
- **I-18's remainder.** `github.rs`'s serial asset downloads are the one worth doing and are the
  one with a shared rate limiter and an artifact ledger in the middle of them — it is a real
  change, not a `buffer_unordered`. The rest (`prereq.rs`, `dotfiles.rs`, `export.rs`,
  `shell/mod.rs`) are small loops over a handful of items.
- **I-21's streaming.** Printing results as they arrive changes what a user sees mid-command and
  loses the sort, which is what makes two runs of `search` comparable. It is a UX decision, not a
  performance one, and it was not asked for.
- **I-22's lazy `rhai::Engine` and lazy diagnostics DB.** Both are real. The engine is one
  `Engine::new()` and the DB is one small file read, and both now sit inside a `try_join!` with
  three other things — so the wall-clock they cost is whatever the *slowest* of the four costs,
  which is not them.
- **I-23, the double config parse.** One small file read and TOML parse. A memo on
  `Config::from_file` would collapse them, and would go stale on the one path that rewrites
  `preferences.toml` (`config init`). Not worth a staleness hazard for ~1 ms.
- **I-42's remainder and I-28's `Package::properties`.** Both are the audit's own Tier 4, and its
  own judgement applies: *"none of these is where the seconds are today — the seconds are in
  subprocesses and sockets."* After Tiers 0–2 that is more true, not less. `Package::properties`
  in particular is a public field on a type thirty backends construct; changing its shape is a
  refactor, not a sweep.

## What was measured afterwards, and what was not

**One number was re-run on the same host: `check drift`'s marginal cost per declared package.**
It is the number I-3 is about, it is the one the audit measured most carefully, and it is flat
now.

Release build of this tree (`cargo build --release`, 2026-08-02 13:15), config isolated in
`%TEMP%`, `priority` naming `winget` only, N qualified `winget:` declarations, one warm-up run
discarded:

| declared | before (audit) | after |
|---|---|---|
| 1 | 4,862 ms | 3,792 / 3,821 / 4,497 / 4,060 / 4,010 ms |
| 4 | 5,427 ms | 3,905 / 4,117 / 3,930 / 5,748 / 3,890 ms |
| 8 | 6,665 ms | 4,051 / 4,014 / 3,992 / 3,910 / 3,853 ms |
| 12 | 7,574 ms | 4,718 / 4,070 / 4,174 / 3,840 / 3,911 ms |

**~247 ms per additional declaration, to within noise of zero.** The fixed cost also fell, from
~4.86 s to ~4.0 s, which is I-43 and I-22 rather than I-3.

**And the first attempt at this measurement was wrong, in the way the audit's own appendix warns
about.** Run with N increasing — 1, 2, 4, 8, 12, in that order, once each — it produced 3,683 /
3,676 / 5,693 / 8,026 / 8,440 ms and looked like a *worse* slope than before the fix. Measuring N
in increasing order conflates N with elapsed wall-clock time, and on this host something else is
drifting under it: a fresh binary being scanned, winget's own cache warming and cooling. Running
the same points interleaved (1, 4, 8, 12 × 3 rounds) and then in decreasing order (12, 8, 4, 1 ×
2 rounds) makes the trend disappear entirely and leaves a ±700 ms spread with no direction.
**A monotonic result from a monotonic sweep is not a result**, which is the same lesson the
appendix records about a suspiciously flat curve.

**What has still not been re-run**, stated because unverified is not done:

- `linix plan` at 302 declarations (was 439.6 s). The measurement above covers the same
  mechanism at N ≤ 12 and the slope is what it tests, but the big number is not re-taken.
- `install choco:bat` end to end (was 399.48 s, of which 18.75 s was the install) and
  `uninstall choco:bat`'s pre-flight (was 7m59s). Both mutate the machine.
- The `check` rollup (was 10.4 s).
- **Nothing at all has been re-measured in the Ubuntu container**, which is where I-1's six
  processes and 12,465 ms were measured. The batching is covered by
  `core::transaction::batching_tests`, which asserts the process *count* — six packages become
  one command — and the count is the thing the container measured. The wall-clock is not
  re-taken.
- The `AtomicUsize` experiment on `spec_is_missing` that the audit proposes as *confirmation*
  that the four blocking calls were the cause of the observed width-of-1 fan-out.

---

## How to read this

The goal is stated three ways in one sentence and they are the same goal: **as parallel as
possible, as efficient as possible, as fast as possible.** LiNix is a program whose entire
runtime is spent waiting on other people's processes and other people's networks. That means
almost every win here is one of four shapes:

1. **Don't ask twice** — the same question answered once and reused.
2. **Don't ask one at a time** — independent waits overlapped.
3. **Don't ask at all** — work done eagerly that nobody needed.
4. **Ask in one breath** — N invocations of a manager collapsed into one.

### Provenance, stated honestly

- Findings marked **[MEASURED HERE]** were measured during this audit, in a disposable
  `linix-it-ubuntu` container, by wrapping each manager binary in place with a counting shim and
  by `strace -f -e trace=execve`. Method and raw output in **Appendix A**. These are the load-bearing
  ones.
- Findings marked **[MEASURED]** carry a number somebody else took. Those numbers come from
  `docs/spec/history.md` and the `GRADE-*` files; they are re-cited, not re-run.
- Findings marked **[READ]** come from reading the code in this audit and have **not** been
  timed. The mechanism is verified — the call site exists, the loop is serial, the cache is
  unread — but the magnitude is an argument, not a measurement.
- Do not quote a **[READ]** finding as a measured number. Several entries below explain the
  *shape* of a measured slowness, and the shape is worth more than a new stopwatch reading:
  the same fix that closes them is right either way.

**`I-n` ids are stable handles, not an ordering.** They were assigned as findings were made, so
they run out of sequence within the tiers. Cite them; do not renumber them. The tiers are the
ordering, and the work-order at the end is the sequence.

### The rule this document keeps tripping over

`CLAUDE.md`: *a bug you find is a representative of a family.* Every large finding here turned
out to be a family, and in most cases the docs had named exactly one member. The most common
pattern in this tree is **the right implementation exists twenty lines away from the wrong
one.** Do not fix one instance. The sibling list is given for each.

### Before touching anything

Findings that change behaviour a user would notice — parallelising `upgrade`, batching
installs, capping search per backend — are **owner rulings**, not implementation detail
(`CLAUDE.md`, "Asking while building"). They are marked **[RULING]**. Build the invisible ones;
bring the visible ones as a question with this document attached.

---

# Tier 0 — the structural ones

These four account for more wall-clock than everything else in this file combined.

## I-1 · One node per package, and the batch machinery is dead code **[MEASURED HERE] [RULING]**

> **Measured, ubuntu container, 2026-08-02.** Six packages needing install produced **six separate
> `apt` processes**, argv captured verbatim:
>
> ```
> install -y -- lolcat
> install -y -- cowsay
> install -y -- pv
> install -y -- sl
> install -y -- toilet
> install -y -- cmatrix
> ```
>
> | | processes | wall |
> |---|---|---|
> | `apt install -y <8 packages>` — one command | **1** | **3,161 ms** |
> | `linix -y sync`, 6 packages to install | **6** | **12,465 ms** |
>
> Scaling, same packages, clean state each run:
>
> | declared | `apt install` processes | `dpkg-query` calls | wall |
> |---|---|---|---|
> | 1 | 1 | 3 | 2,131 ms |
> | 2 | 2 | 6 | 4,017 ms |
> | 4 | 3 | 10 | 7,372 ms |
> | 8 | 6 | 20 | **31,901 ms** |
>
> One `apt install` per package, exactly. Superlinear wall time. The batched baseline installs
> *more* packages in a quarter of the time.

`src/core/transaction.rs:55`, `:475`, `:497` · `src/backends/generic.rs:428`

```rust
pub enum GraphAction {
    Install(PackageSpec),          // one package
    Remove { name, backend },      // one package
}
...
handler.install(std::slice::from_ref(spec), ...)   // ALWAYS a 1-element slice
handler.remove(std::slice::from_ref(name), ...)    // ALWAYS a 1-element slice
```

Every install and every removal is its own DAG node, and every node calls the backend with a
one-element slice. Meanwhile `generic.rs::install_group` is written to batch — it allocates
`Vec::with_capacity(specs.len())`, it partitions `@unverified` specs from the rest so the two
groups become *two commands*, it accumulates `names` across specs. **That code has never run
with more than one spec in the sync path.** `push_names` takes an iterator for the same reason
and is always handed one name.

So installing 50 apt packages is 50 sequential `apt-get install <one>` invocations: 50 process
spawns, 50 full package-cache reads, 50 dpkg lock acquisitions, 50 dependency-resolution
passes. `apt-get install a b c … z` does all of that once, and resolves the *shared* dependency
graph once instead of 50 times.

And it is worse than "not parallel", because of I-2.

**The fix.** Group the graph's ready set by backend before dispatch and give each backend the
whole group. The batching code is already written and already correct — it needs a caller.
Nodes with `requires` edges still serialise; nodes without them are what the batch is made of.
Rollback granularity is the real design question: a batch that fails fails as a batch, so
`Prior` needs to be captured per package before the batch runs (it already is — `Prior` is
computed per node) and the compensation loop already walks per package.

**Siblings — the 16 hand-written backends that loop where `generic` batches:**
`npm.rs:70,86` · `cargo.rs:80,94` · `pipx.rs:74,92` · `uv.rs:81,99` · `yarn.rs:126,143` ·
`pnpm.rs:81,98` · `brew.rs:62,82` · `emacs.rs:104,127` · `krew.rs:70,84` · `mise.rs:74,94` ·
`nix.rs:62,105,121` · `go.rs:162` · `snap.rs:179,245` · `vscode.rs:115,131` ·
`pubdart.rs:88,106` · `psresource.rs:205,226` · `storage.rs:149,176,431,462` ·
`service.rs:298,319` · `setting.rs:343,381` · `btrfs.rs:399,486`

Every one of these managers accepts multiple names on one command line. Every one of them is
being handed one.

## I-2 · `run_exclusive` serialises what the DAG just parallelised — and the telemetry calls it parallel **[MEASURED HERE]**

> **Measured.** LiNix's own report from the run above:
>
> ```
> Parallel Task Breakdown:
>   ✓ [apt     ] lolcat               (12413ms)
>   ✓ [apt     ] cowsay               (12413ms)
>   ✓ [apt     ] pv                   (12413ms)
>   ✓ [apt     ] sl                   (12413ms)
>   ✓ [apt     ] toilet               (12413ms)
>   ✓ [apt     ] cmatrix              (12413ms)
> ```
>
> **Six tasks, one identical duration, to the millisecond.** That is not six things taking the
> same time — it is six things each measuring the whole serialised window, because each waited on
> the per-manager mutex and the timer spans the wait. Total wall was 12,465 ms; every task claims
> 12,413 ms of it.
>
> The header says `Parallel Task Breakdown`. Nothing here ran in parallel. **A user reading this
> output is told the opposite of what happened**, which makes it a reporting defect as well as a
> performance one — and it is why the serialisation survived this long unexamined.

`src/core/executor.rs:853`

```rust
pub async fn run_exclusive(&self, lock_key: &str, ...) {
    let mutex = self.lock_map.entry(lock_key.to_string())...;
    let _thread_guard = mutex.lock().await;      // per-MANAGER mutex
    ...
    lock_file.lock_exclusive()...;               // + a cross-process file lock
```

`lock_key` is the manager name. So all `npm` installs hold one mutex, all `apt` installs hold
another. The transaction's `max_concurrent` (= `max_parallel`, 20 on the audited host) therefore
buys parallelism **only across different backends**. A config that is 200 `apt:` lines and 20
`cargo:` lines runs at an effective width of 2.

This is correct for safety — two `apt-get install` at once contend on the dpkg lock and one of
them fails. But combined with I-1 it is the worst of both worlds: LiNix neither batches the
manager's work nor overlaps it. Batching is what makes the serialisation free, which is why
I-1 and I-2 are one fix and not two.

Two further costs in the same function:

- **`lock_file.lock_exclusive()` is a synchronous blocking call in an `async fn`.** It is
  `fs2`'s blocking flock. When a second `linix` holds the lock, this parks an entire tokio
  worker thread rather than yielding. Wrap in `spawn_blocking`, or use a try-lock/backoff loop.
- **`open_exec_lock` runs `std::fs::create_dir_all` on every call** (`executor.rs:823`),
  synchronously, for a directory that exists after the first one.

## I-3 · `info(name)` is implemented as "list the whole machine, then find one" — in 18 backends **[MEASURED HERE]**

> **Measured, ubuntu container.** `linix check drift`, qualified `apt:` declarations only,
> counting `dpkg-query` invocations:
>
> | declared packages | `dpkg-query` calls |
> |---|---|
> | 1 | 2 |
> | 5 | 6 |
> | 10 | 11 |
> | 20 | 21 |
> | 38 | 38 |
>
> **One full machine listing per declared package**, exactly — `calls ≈ declared + 1`. The
> mechanism is confirmed and is manager-independent.
>
> **The cost is not.** On this container `dpkg-query -W` is ~10 ms, so 38 listings cost under two
> seconds and the fixed startup dominates. Where the listing is expensive the same mechanism is
> catastrophic: the docs measure `winget list` at **1.3 s standalone**, which at 302 declarations
> is ~390 s — and `linix plan` on that host measured **439.6 s**. Same code, two orders of
> magnitude apart in effect, decided entirely by which manager you use.
>
> This is the honest framing and it corrects an earlier draft of this document, which implied the
> cost was universal. **The defect is universal; the pain is `winget`/`choco`/`emacs`/`vscode`.**
>
> **And here is the pain, measured on Windows against a fresh build of the current tree**
> (`linix 0.7.0`, built 2026-08-02 09:55). `check drift` is a read-only command whose cost should
> not depend on how many packages are *declared*:
>
> | declared | `check drift` |
> |---|---|
> | 1 | 4,862 ms |
> | 2 | 5,036 ms |
> | 4 | 5,427 ms |
> | 8 | 6,665 ms |
> | 12 | 7,574 ms |
>
> A clean straight line: **~247 ms of marginal cost per additional declared package**, on top of a
> ~4.6 s fixed cost. (Standalone `winget list` on the same host: 1,115 ms.) Extrapolated to the
> 302 declarations the docs describe, the marginal term alone is **~75 seconds** — for a command
> that answers "what has drifted?".
>
> The fixed ~4.6 s is its own finding: see I-22 (eager startup), I-14 (PATH re-probing) and
> I-43 (repeated resolution). None of it is the user's packages.

The docs name this once, in `GenericQueryable::info`. It is **eighteen backends**, and the
call sites are worse than the implementations.

**The implementations** — each of these runs a full machine listing to answer one name:

| backend | line | body |
|---|---|---|
| `generic.rs` | 680 | `let all = self.list_installed().await?;` — covers apt, dnf, pacman, apk, zypper, winget, choco, scoop, pip, gem, bun, composer, opam, luarocks, nimble, pixi, spack, mix, helm, cabal, stack, asdf, and every user-defined backend |
| `cargo.rs` | 149 | `list_installed()` + `find` |
| `npm.rs` | 134 | `list_installed()` + `find` |
| `pipx.rs` | 141 | `list_installed()` + `find` |
| `uv.rs` | 161 | `list_installed()` + `find` |
| `yarn.rs` | 198 | `list_installed()` + `find` |
| `pnpm.rs` | 154 | `list_installed()` + `find` |
| `flatpak.rs` | 200 | `list_installed()` + `find` |
| `dnf.rs` | 163 | `list_installed()` + `find` |
| `pacman.rs` | 172 | `list_installed()` + `find` |
| `conda.rs` | 160 | `list_installed()` + `find` |
| `nix.rs` | 149 | `list_installed()` + `find` |
| `emacs.rs` | 189 | `list_installed()` + `find` |
| `xbps.rs` | 184 | `list_installed()` + `find` |
| `appimage.rs` | 281 | `list_installed()` + `find` |
| `mise.rs` | 181 | `list_installed()` + `find` |
| `go.rs` | 283 | `scan()` + `find` (a filesystem walk) |
| `krew.rs` | 124 | `scan()` + `find` |
| `pubdart.rs` | 144 | `scan()` + `find` |

Only `brew`, `snap`, `vscode`, `github`, `web`, `btrfs`, `storage`, `service` and `setting` ask
a targeted question.

And several of the "+ find" implementations then spawn **another** subprocess for a property
nobody asked for: `cargo.rs:152` `get_cargo_root()`, `npm.rs:137` `get_global_prefix()`,
`pipx.rs:144` `get_pipx_home()`, `uv.rs:164` `get_tools_dir()`, `yarn.rs:201` +`get_global_bin()`,
`pnpm.rs:157` +`get_global_bin()`. None of these are cached either.

**The call sites are the multiplier.** Every one of these is `info()` once per package, serially:

| site | what it does |
|---|---|
| `app/sync/planner.rs:544` (`spec_is_missing`) | once per *declared* package — the documented 302 listings |
| `core/transaction.rs:477` | **once per installed package, immediately after installing it, purely to read `download_size`** |
| `verbs/plan.rs:571` (`build_and_write_locks`) | once per *managed* package — **while holding the state mutex** |
| `app/export.rs:29` | once per recorded package |
| `app/insight.rs:38` | once per managed package |
| `app/run.rs:46` | once per requested package |
| `app/context.rs:619` | once per spec |
| `verbs/packages.rs:539,544` | nested: per package × per backend |

`transaction.rs:477` is the one that explains a measured number nobody had explained. The docs
record `install choco:bat` as a **399.48s transaction of which the `[choco] bat` task was
18.75s**. The post-install `info()` on a `generic` backend is a full `choco list` of the
machine, per installed package, and it is thrown away unless the manager happens to report a
download size. That is ~380 seconds of accounting.

**The fix, in order of payoff:**

1. **Delete the post-install `info()` call** or make it conditional on the backend actually
   reporting `download_size`. Nothing else reads those properties.
2. **Wire up the cache that already exists** — see I-4. One `list_installed()` per backend per
   run, memoised; `info()` becomes a set lookup. `installed_sets()` in `planner.rs:286` already
   does exactly this for the removal side, twenty lines above the code that does not.
3. **Give `Queryable` a `has(name) -> bool`** so backends with a real targeted query (`brew
   info`, `snap info`, `dpkg -s`) can answer without listing, and the listing path is the
   fallback rather than the rule.

## I-3b · The write-ahead journal rewrites itself in full, pretty-printed, under one mutex, 2–3× per package **[READ]**

`src/core/journal.rs:130`

```rust
pub fn flush(&self) -> Result<()> {
    let data = serde_json::to_string_pretty(&self.entries)?;   // the WHOLE map, every time
    persist(&self.path, &data)                                 // temp file + rename, sync
}
```

`flush()` is called from `record_start` (line 166), `record_success` (line 178) and
`record_failure` (line 191) — so **every package node writes the entire journal at least
twice**, and the journal only grows during the run. Installing 50 packages is ~100 full
serialisations of a monotonically growing structure and ~100 atomic write-and-rename cycles.
Bytes written are O(n²) in the number of actions.

Three separate costs stack here:

1. **O(n²) serialisation.** A write-ahead log is the canonical append-only structure. Appending
   one line per state change (JSONL) turns each record into a constant-size write.
2. **`to_string_pretty` on a machine-read WAL.** Roughly double the bytes, for a file no human
   opens. Same defect as I-26.
3. **It is synchronous, and it is held behind `Arc<Mutex<Journal>>`.** Every concurrent DAG
   worker must take that one mutex and then perform a blocking file write plus rename *while
   holding it*. This is a hard throttle sitting directly under the transaction's
   `max_concurrent` — the more parallelism I-1 and I-2 unlock, the more this becomes the
   bottleneck. `AUDIT-v6.org:803` already records observed contention "writing the WAL journal,
   when cargo runs several test binaries in parallel"; that was read as a test-harness artefact
   and it is the product's shape.

**Fix:** append-only JSONL, one line per transition, no pretty-printing, and move the write off
the runtime thread (or make it a dedicated writer task fed by a channel, so workers never block
on I/O at all). Recovery reads the file forward and takes the last state per id — which is what
`heal` already conceptually does.

## I-4 · `PackageCache` is constructed on every run and never read **[READ]**

`src/core/cache.rs` (150 lines) · `src/app/context.rs:96` · `src/app/services.rs:109`

```rust
pub struct PackageCache {
    installed: SmartCache<String, Vec<Package>>,   // 300s TTL
    search:    SmartCache<String, Vec<Package>>,   // 600s TTL
    info:      SmartCache<String, Package>,        // 300s TTL
}
```

`App` holds `pub cache: Arc<PackageCache>`. It is built on every single invocation. **Every
accessor has zero callers:** `get_installed`, `set_installed`, `get_search`, `set_search`,
`get_info`, `set_info`, `clear_all` — grepped across `src/` and `tests/`, only
`PackageCache::new()` is ever called. It compiles clean because the type is `pub` in a library.

So the tree contains a working, TTL'd, correctly-`Arc`-shared cache for exactly the three
questions I-3 asks 300 times per run, and nothing consults it.

Note the naming trap that probably hid this: `AppContext::get_info` (`context.rs:573`) is an
unrelated method with the same name, so a grep for `get_info` looks populated.

**The fix.** Either wire it into `Queryable` (`list_installed` checks `installed` first,
`info` checks `installed` then falls back) — which closes most of I-3 on its own — or delete it
under NO LEGACY. Do not leave it. The 300s TTL is wrong for a one-shot CLI anyway; a
process-lifetime memo is the correct semantics and is simpler.

## I-4b · Eight HTTP clients, one built per request — no connection reuse anywhere **[READ]**

`insight.rs:264` · `github.rs:166` · `node_registry.rs:28` · `pip_search.rs:22` ·
`vscode.rs:28` · `download.rs:96` · `declare.rs:150` · `vars_embedded.rs:229`

```rust
// node_registry.rs:28 — inside registry_search(), called per query
let client = reqwest::Client::builder().user_agent("linix-manager")
    .timeout(http_timeout()).build()?;
```

Every one of these constructs a **fresh `reqwest::Client` per call**, and a `reqwest::Client`
*is* the connection pool. Consequences:

- **Zero HTTP keep-alive.** Every request pays a full TCP handshake plus a full TLS handshake —
  two extra round trips and a public-key operation — even when the previous request went to the
  same host seconds earlier.
- **rustls root store re-initialised per client.** Building the TLS config parses the trust
  anchors each time.
- **No shared connection cap and no shared rate limiter.** `vscode.rs:28` builds its client
  *inside* the rate-limiter's retry closure, so a retried request builds a new client too.

Where this hurts most:

| path | shape |
|---|---|
| `insight.rs:287,292` | hundreds of **serial** OSV advisory GETs (I-16) — one full TLS handshake **each** |
| `search` | ~22 registries, and `node_registry::registry_search` is shared by npm, pnpm *and* yarn, so one search builds it three times |
| `github.rs` release resolution + asset downloads | repeated calls to the same `api.github.com` host, never reusing a connection |
| `pip_search`, `vscode` | one client per query |

**Fix:** one shared `static CLIENT: OnceCell<reqwest::Client>` (or a small set, keyed by the
handful of distinct policies — `download.rs` needs its redirect policy, the rest do not). This
is a small change with an outsized effect on I-16 and I-21, and it composes with them: parallel
requests over a shared pool is what makes the fan-out cheap.

## I-43 · The model is resolved up to 3 times per command, re-running the user's subprocesses and HTTP each time **[MEASURED HERE]**

> **Measured.** An external vars provider (`vars.sh`) that appends a line to a counter file each
> time it runs, approved via `linix lock`, then one execution of each command:
>
> | command | times the user's provider was executed | wall |
> |---|---|---|
> | `linix vars` | 1 | 9 ms |
> | `linix eval` | 1 | 8 ms |
> | `linix plan` | 1 | 15 ms |
> | `linix check drift` | **2** | 923 ms |
> | `linix check` | **3** | 1,298 ms |
>
> **This corrects an earlier draft of this document, which predicted 4–6 for `check` by counting
> `StateResolver::new` sites.** The measured number is 3. Counting construction sites overstated
> it: not every site is reached on every path, and some resolve only facts rather than the whole
> model. The finding stands — a read-only command executes the user's script three times — but the
> number is 3, and 3 is what this document claims.
>
> Corroborating: `linix check` measured 2,021 ms against `check health` 549 ms + `check drift`
> 546 ms, consistent with repeated resolution rather than section cost alone.

`39 × StateResolver::new` across `src/`, and **nothing memoises a resolution.**

```rust
// resolver.rs:301 — every resolver entry point goes through this
pub async fn facts_for_host(&self) -> Result<HostFacts> {
    let facts = HostFacts::current();
    let vars = match &self.vars_override {
        Some(frozen) => frozen.clone(),
        None => self.resolve_vars_against(&facts).await?.0,   // <-- runs EVERYTHING
    };
```

`resolve_vars_against` runs every external vars provider (a subprocess), every embedded `sh()`
variable (a subprocess) and every `http()` variable (a blocking HTTP request on a spawned OS
thread — I-8). `resolve_model()` then additionally runs every `generate:` script (I-41).

And `StateResolver::new` is not free either: it reads and parses `locks/versions.json` on
construction, at all 39 sites.

**Now count one command.** `linix check` with no section runs `check_summary`, which does:

| step | resolutions |
|---|---|
| `resolver.resolve_model()` — explicit | 1 |
| `app.priority_backends()` → `StateResolver::new` + priority resolution | 2 |
| `app.adopter().discover()` → `adopt.rs:226,274,301` | up to 5 |
| `linix::app::insight::audit(app)` → `insight.rs:536` | 6 |

`AppContext` alone builds a fresh resolver in `host_facts()` (264), `vocabulary()` (272),
`reject_unusable_line()` (293), and two more at 413 and 805 — each re-reading the lockfile,
re-detecting host facts, and **re-running the user's variable providers**.

`verbs/check.rs` has six construction sites. `verbs/plan.rs` has four — `linix lock` calls
`approve_exec_scripts` (787) and `approve_health_checks` (830) back to back, each building its
own resolver and doing a full model resolution. `app/profile.rs` has five. `adopt.rs` has three.

**This is not only slow, it is semantically wrong.** A vars provider is a program the user
wrote. Running it four times for one `linix check` means any side effect happens four times, and
any `http()` variable is fetched four times over four fresh TLS connections (I-4b). The
`vars_override` field on `StateResolver` is the escape hatch someone already built for exactly
this — it is opt-in and unused on every default path.

**Fix:** resolve once per process. An `OnceCell<Arc<DesiredState>>` (and a second for
`HostFacts`/`Priority`, which more callers want than want the full model) on `App`, handed to
whoever asks. This is the single largest *redundancy* in the tree, as distinct from the largest
*serialisation* (I-1) and the largest *repetition* (I-3).

## I-44 · `remote_has` has no implementations, its default is a search — and the caller searches again **[MEASURED HERE]**

> **Measured, counting `apt-cache` invocations for one bare name.** The duplicate is real, and it
> fires on exactly two of three paths:
>
> | case | `apt-cache search` calls | why |
> |---|---|---|
> | apt **has** it, no version pin (`ripgrep`) | **1** | `remote_has` found it; the fallback never ran |
> | apt **lacks** it (`cargo-nextest`, resolved to cargo) | **2** | `remote_has` searched, returned false, caller searched again |
> | apt **has** it, `@version=latest` | **2** | `remote_has` searched, then `remote_info` searched again |
>
> The negative-path argv, logged verbatim — same command, same argument, twice:
>
> ```
> search -- cargo-nextest
> search -- cargo-nextest
> ```
>
> **This tempers an earlier draft**, which implied the duplication was near-universal. It is not:
> the *winner* of a priority chain costs one search. Every candidate that says no costs two, and
> any version pin costs two. On a chain of length k where the winner is last, that is
> `2(k−1) + 1` searches instead of `k`. The waste is real, measured, and proportional to how much
> work the priority chain does — which is precisely the case `priority` exists to make cheap.

`src/core/manager.rs:192` is the **only** definition of `remote_has` in the tree. There are zero
overriding implementations:

```rust
async fn remote_has(&self, name: &str) -> Result<bool> {
    let results = self.search(name).await?;          // a FULL search
    Ok(results.iter().any(|p| p.name == name))
}
async fn remote_info(&self, name: &str) -> Result<Option<Package>> {
    let results = self.search(name).await?;          // ANOTHER full search
    Ok(results.into_iter().find(|p| p.name == name))
}
```

Now `resolver.rs:1211`, the resolver's per-candidate probe:

```rust
let has = match searchable.remote_has(package_name).await {     // search #1
    Ok(true) => true,
    // `false` here is not proof of absence — a backend may not implement it — so
    // an inconclusive answer falls through to a real search.
    Ok(false) => match searchable.search(package_name).await {  // search #2 — IDENTICAL
        Ok(results) => results.iter().any(|pkg| pkg.name == package_name),
```

The comment is honest about its assumption and the assumption is false. **No backend implements
`remote_has`**, so `Ok(false)` never means "could not tell" — it means the default already ran
the search and the name was not in it. The fallback then runs *the identical call with the
identical argument* and computes *the identical predicate*. On the negative path — which is
every candidate the priority chain rejects, i.e. most of them — this is exactly 2× the work.

With a `@version=` constraint it is 3×: `remote_info` (line 1228) runs a third search.

So the resolver's cost per bare name is `2 × (candidates before the winner) + 3 × 1`, where each
unit is a package-manager search — `apt-cache search`, a PyPI request, an npm registry query, a
GitHub API call. Then I-13 runs those chains one declaration at a time.

**Fix, three parts, all small:**

1. **Delete the fallback.** `remote_has`'s contract is a `bool`; if "could not tell" needs to be
   expressible, the return type should say so (`Result<Option<bool>>`), not be smuggled through
   a `false` that costs a duplicate query.
2. **Ask once and reuse.** `ask()` wants three facts from one search — presence, and the
   version. Call `search(name)` once, keep the `Vec<Package>`, answer all three from it.
3. **Then** override `remote_has`/`remote_info` where a manager has a genuinely cheaper targeted
   query (`brew info`, `apt-cache show`, `pip index versions`) — the trait defaults exist so
   that is optional, and today nobody has taken the option.

## I-45 · State lookups are linear scans over a `Vec`, and one allocates inside the fan-out **[READ]**

`src/core/state.rs:62` — `pub packages: Vec<ManagedPackage>`

```rust
pub fn is_managed(&self, backend, name) -> bool {
    self.packages.iter().any(|p| p.backend == backend && p.name == name)
}
pub fn get_package(&self, backend, name) -> Option<&ManagedPackage> {
    self.packages.iter().find(|p| p.backend == backend && p.name == name)
}
pub fn is_held(&self, backend, name) -> bool {
    let qualified = format!("{}:{}", backend, name);        // allocates, every call
    self.held.iter().any(|k| k == name || k == &qualified)
}
```

- **`is_managed` at `context.rs:686`** filters every installed package against every managed one.
  The docs put those at ~476 and ~301 on a stock Ubuntu: **~143,000 double string comparisons**
  per `installed_but_unmanaged` / `purge-unmanaged` / `adopt`.
- **`is_held` at `planner.rs:562`** is called from `spec_is_missing` — **inside the fan-out**,
  once per declared package. 302 `format!` allocations plus 302 linear scans, on the hot path,
  to answer a question a `HashSet` answers with no allocation at all.

**Fix:** keep an index. A `HashSet<(String, String)>` or a `HashMap<String, Vec<usize>>` built
once when the registry loads; `is_held` compares against a borrowed pair instead of formatting a
key. `held` is a `Vec<String>` matching either a bare or qualified name — two `HashSet`s, one per
form, gives the same semantics with no scan and no allocation.

## I-46 · Managers asked expensive questions where a cheap one exists **[READ]**

The per-backend argv audit — the class the previous pass named as unswept. Four real cases:

| backend | what it runs | the cheaper question |
|---|---|---|
| **zypper** (`registry.rs:462`) | `zypper search --installed-only` — goes through libsolv and the repo metadata | zypper is RPM-based. **`rpm -qa --queryformat`** is what `dnf.rs:163` already uses, and it is an order of magnitude cheaper. The fast implementation is in this tree, on the sibling backend. |
| **vscode** (`vscode.rs:~40`) | `code --list-extensions --show-versions` — **spawns Electron**; ~1–2 s of startup before it prints anything | the extension directory (`~/.vscode/extensions`) is a filesystem read. `go.rs` and `krew.rs` already model "scan the directory" as a `scan()`. |
| **vscode `info`** (`vscode.rs:170`) | `query_marketplace(name)` — **a network round trip per package**, with a fresh HTTP client (I-4b) inside a rate limiter | "is this extension installed?" is a local question. It is being answered by asking Microsoft. Per package, per plan. |
| **emacs** (`emacs.rs:~90`) | `emacs --batch` evaluating `(package-initialize)` and walking `package-alist` | correct in principle and genuinely expensive — `package-initialize` loads the whole package system. Combined with I-3 (`info` = list + find) it is paid per package. This one wants caching (I-4) more than a different question. |

`apt` (`dpkg-query -W -f=...`), `dnf` (`rpm -qa --queryformat`), `cargo` (`install --list`),
`npm`/`pnpm` (`list -g --depth=0 --json`) and `pacman` (`-Q`) are all asking the right, cheap
question. Noted so this table is not read as a general indictment — it is four backends, and one
of them (`zypper`) has its own fix already written next door.

Two more worth a measurement rather than a claim: `winget list` is documented at **1.3 s
standalone** on the audited host, and `choco list` is comparable. Neither has a cheaper CLI
alternative, which makes them the strongest argument for I-4 — the answer must be fetched once
per run and reused, because it cannot be made cheap.

---

# Tier 0.5 — blocking calls sitting on the async runtime

A mechanical sweep of every `async fn` body for synchronous calls (excluding anything already
inside `spawn_blocking`) returned **~90 hits**. This matters more here than in most programs,
because of a design note the planner states about itself:

> *"the futures borrow `&self` so this stays on one task (no spawn), which is all that is needed
> since the time is spent waiting on child processes."*

That is true **only while every wait is genuinely async**. All 302 buffered futures live on one
task, so a single synchronous call anywhere beneath them blocks all of them and the fan-out
becomes decorative. The docs measured exactly that — `buffer_unordered(20)` observed at **max
concurrent = 1** — and called the cause undiagnosed. Below are every blocking call that sits on
that path. At least four of them are on it.

## I-32 · The entire model resolution is synchronous — files, subprocesses **and network** **[READ]**

`src/app/sync/resolver.rs:334` is `async fn resolve_model()`. It calls
`crate::model::Resolver` and `DesiredState::resolve()` — `src/model/resolve.rs:245`, a
**synchronous `fn`** — which performs, all on the runtime thread:

| what | where |
|---|---|
| read the `active` file | `resolve.rs:409` `std::fs::read_to_string` |
| read every profile file | `resolve.rs:299` |
| read every module file | `modules.rs:57` `std::fs::read_to_string` |
| read the schedules file | `resolve.rs:501` |
| read a line-file body | `resolve.rs:329`, `resolver.rs:455` |
| **run every external vars provider as a blocking subprocess** | `vars_provider.rs:140` `cmd.output()` |
| **run every embedded `sh()` var as a blocking subprocess** | `vars_embedded.rs:140`, `:156` |
| **perform blocking HTTP for every `http()` var** | `vars_embedded.rs:229` (I-8) |

So the config-resolution phase of every command — dozens of file reads, N subprocess spawns and
N network requests — runs with the runtime unable to do anything else on that thread. On a
config with a vars provider and an `http()` variable, `resolve_model().await` is a lie: nothing
yields.

**Fix (in order):** wrap the whole `DesiredState::resolve()` call in `spawn_blocking` — one line,
immediately correct, and it also stops it starving the fan-out. Then make the vars providers
genuinely async so N providers can run concurrently instead of one at a time.

## I-33 · Checksum verification blocks a worker for the length of the file **[READ]**

`src/core/security.rs:10` / `:39` are synchronous `fn`s (correctly *streamed* — `io::copy` into
the hasher, not read-to-`Vec` — the comment says so and it is right). They are called from
async download paths:

- `github.rs:598` `verify_checksum(&dl_path, expected_sha)?` and `:600` `generate_checksum(...)`
- `web.rs:163` `verify_checksum(...)`
- `appimage.rs:150` `verify_checksum(...)`

Hashing a 150 MB release tarball is seconds of CPU-bound work holding a runtime worker thread.
`spawn_blocking` — this is the textbook case for it.

## I-34 · The planner hashes two files synchronously, per template spec, inside the fan-out **[READ]**

`src/app/sync/planner.rs:754`

```rust
async fn template_needs_update(&self, spec: &PackageSpec) -> bool {
    ...
    let s_hash = crate::core::security::generate_checksum(source);   // sync, blocking
    let t_hash = crate::core::security::generate_checksum(target);   // sync, blocking
```

Reached from `spec_is_missing` — which is *the* function whose fan-out was measured at width 1.
Two blocking file reads plus two SHA-256 passes, per template spec, on the shared task.

This is a strong candidate for the documented serialisation on any config using templates, and
it sits beside I-25's `which::which` and I-32's sync model load as the three known blocking
calls on that exact path. **Fix all three before re-running the `AtomicUsize` experiment** — it
is a confirmation instrument, not a diagnostic one, once the causes are known.

## I-35 · A ~51-second Windows restore point runs before every mutating sync, serially, silently **[MEASURED]**

`src/app/sync/mod.rs:171` · `src/core/snapshot.rs:671`

```rust
let restore_point = match self.snapshot_manager
    .auto_snapshot(SnapshotLabel::PreSync).await { ... };   // awaited before ANY work
```

Measured on Windows: `Checkpoint-Computer` **50.8s**, `Invoke-CimMethod CreateRestorePoint`
**53.3s** — the docs confirm there is no faster API to swap to. So this is **a fixed ~51-second
tax on every install and every uninstall on Windows**, taken before the transaction starts, and
the docs note plainly that *nothing in the output says it is happening*.

The code's own comment says the snapshot is *"a safety NET, not a precondition"* — policies that
genuinely require one gate on `has_provider()` upstream. If it is not a precondition, it does
not have to be a *barrier*: start it, proceed with planning and the guard checks, and join
before the first mutating command. That recovers most of the 51 seconds behind work that has to
happen anyway.

Two smaller things on the same path:

- **Say it is happening.** A silent 51-second pause reads as a hang; that is how it was
  reported. This is a **[RULING]** (user-visible output) and an easy yes.
- `snapshot.rs:566` runs `powershell -Command` with **neither `-NoProfile` nor
  `-NonInteractive`** — the docs flag this as still open. A user's PowerShell profile can add
  hundreds of milliseconds to seconds per invocation. `psresource.rs` has passed `-NoProfile`
  all along; `executor.rs:299` passes it too. Two of three.

## I-36 · State persistence clones the whole registry and writes it synchronously **[READ]**

`src/app/sync/mod.rs:196`

```rust
let state_to_save = self.state.lock().await.clone();     // deep clone of EVERY managed package
tokio::task::spawn_blocking(move || state_to_save.save())
```

The `spawn_blocking` is right. The `.clone()` before it deep-copies every `ManagedPackage`
including each one's `properties: HashMap<String, String>` — for 301 managed packages, a few
hundred HashMap allocations to hand the data across a thread boundary. An `Arc<StateRegistry>`
snapshot, or serialising to a `String` under the lock and writing the string, avoids the copy.

And **13 other `state.save()` call sites are fully synchronous inside `async fn`s**, with no
`spawn_blocking` at all: `leases.rs:87,179` · `shell/mod.rs:124` · `cleanup.rs:469` ·
`declare.rs:479,516,560` · `packages.rs:595,624,650` · `plan.rs:538` · `upgrade.rs:185,312`.
Each is a full serialise (pretty-printed — I-26) plus an atomic write on a runtime thread.
Check `cleanup.rs:420-469` and `upgrade.rs` for saves inside per-package loops; a full state
rewrite per package is the failure mode to rule out, and it is the same shape as I-3b.

## I-37 · 48 synchronous `stat` calls inside `async fn` bodies **[READ]**

`.exists()`, `.is_file()`, `.is_dir()`, `.is_symlink()`, `.canonicalize()` are all blocking
syscalls; `tokio::fs` has async equivalents and parts of this tree already use them
(`tokio::fs::try_exists` appears throughout `declare.rs` and `shell/mod.rs`). The synchronous
form survives in 48 places, concentrated in:

`bundle.rs` (7, several inside directory-walk loops) · `link.rs:328,496,568` ·
`cleanup.rs:149,329,330,331,379,385` · `setup.rs:73,87,400,439` · `btrfs.rs:402,419,500` ·
`check.rs:783,797` · `apply/dotfiles.rs:191` · `apply/extras.rs:276` · `web.rs:218` ·
`snapshot.rs:368` · `cargo.rs:163` · `go.rs:242` · `github.rs:763` · `shim_manager.rs:181` ·
`scheduler/mod.rs:111` · `insight.rs:609` · `utils/file.rs:270`

Individually microseconds. Collectively they are the reason "make everything async" is not a
completed job in this tree, and on a network filesystem or a cold cache each one is a real stall
on a shared runtime thread. Low priority individually; worth one mechanical sweep.

---

# Tier 1 — dead and duplicated machinery

`CLAUDE.md`: *when you find a second implementation of something, the task is to remove one.*

## I-5 · `AppCore` and `AppServices` are a dead duplicate of `App` **[READ]**

`src/app/services.rs`

`AppCore` (line 19) has the same thirteen fields as `App` (`context.rs:27`) — `config`, `cache`,
`registry`, `executor`, `metrics`, `progress`, `hooks`, `state`, `snapshot_manager`, `journal`,
`scheduler`, `notifications`, `diagnostics`. `AppCore::from_config` (line 83) repeats `App`'s
whole construction sequence, `debug!("assembling services")` and all. **`AppCore` has zero
references outside its own file. `AppServices` has zero constructions.**

Cost: build time, binary size, and the standing risk that someone fixes a startup bug in one
copy. Delete both.

## I-6 · Two `command_exists`, and the slow one is dead **[READ]**

`src/utils/command.rs:11` spawns the external `which`/`where.exe` program:

```rust
pub async fn command_exists(cmd: &str) -> bool {
    let check_bin = if cfg!(windows) { "where" } else { "which" };
    Command::new(check_bin).arg(cmd)...
```

`src/core/executor.rs:464` does the same job in-process, with a comment explaining precisely why
spawning the external tool was wrong (minimal fedora/arch/alpine images don't ship `which`, so
every backend read as OFFLINE there):

```rust
fn check_command(&self, cmd: &str) -> bool { which::which(cmd).is_ok() }
```

`utils::command::command_exists`, `command_exists_sync` and `get_command_version` have **no
callers**. The whole module is a spawn-per-probe implementation of a solved problem, kept
alive. Delete it, or keep only `split_command` if that has callers.

## I-7 · Dead dependencies **[READ]**

`Cargo.toml`

| crate | uses in `src/` | note |
|---|---|---|
| `rayon` | **0** | a whole data-parallelism library compiled into every build |
| `nonzero_ext` | **0** | |
| `mlua` (`lua54`, `vendored`) | 5 | **vendored builds Lua from C source on every clean build**, for one hook dialect |
| `rhai` | 15 | a *second* scripting engine |
| `tera` | 168 | a *third* template/expression engine |

Three embedded languages is the "two of everything" rule with an extra everything. `rhai` and
`mlua` both exist to run user hooks (`app/hooks.rs:146` dispatches on a `#rhai` marker vs Lua);
that is a real feature decision and a **[RULING]**, not something to delete unilaterally. But
`rayon` and `nonzero_ext` are free deletions today.

`tokio = { features = ["full"] }` pulls every tokio subsystem including ones this program never
touches. Narrowing to the used feature set is a pure build-time and binary-size win.

`reqwest` carries the `blocking` feature for exactly one call site — see I-8.

## I-8 · A second HTTP client, which spawns an OS thread per request and joins it **[READ]**

`src/model/vars_embedded.rs:226`

```rust
fn http_get(url: &str) -> Result<String, String> {
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()...;   // built per call
        ...
    }).join()                                                    // blocks the caller
```

Every `http()` variable: one OS thread spawned, one TLS client constructed from scratch, one
blocking request, one join. N variables = N threads, strictly one at a time, and the async
`reqwest` client the rest of the tree uses is right there. The thread exists to escape the
runtime the caller sits inside — which is what `spawn_blocking` is for, and better still, what
an `async` call needs no escape from at all.

Fix: use the shared async client. That also drops the `blocking` feature from `reqwest`.

---

# Tier 2 — serial where it could be parallel

A full inventory found **169 loops containing an `.await` on their first statement** in
non-test code. Many are correctly serial (ordered mutations, per-manager locks). These are the
ones that are not.

## I-9 · `AppContext::upgrade` is a plain `for` loop next to its parallelised twin **[MEASURED-adjacent] [RULING]**

`src/app/context.rs:493`

```rust
// The same rule as `update`, and for the same reason: ...
for backend in self.registry.available() {
    upgradable.upgrade(backend.sudo_for_write()).await
}
```

`update()` — the function immediately above, at line 459 — fans out with
`buffer_unordered(cap)`. The comment on `upgrade` says "the same rule as `update`" and then
copies only the *error-handling* rule, not the concurrency.

`history.md:2055` records this as deliberate: *"`upgrade` is deliberately left serial: it
changes packages, so concurrent sudo operations would interleave."* That reasoning is sound and
**over-broad**. It is true for managers sharing a system package database (apt/dnf/pacman) and
false for `cargo`, `npm`, `pipx`, `uv`, `yarn`, `pnpm`, `vscode`, `emacs`, `krew`, `go` — which
contend with nothing and are typically the slow ones (each rebuilds or refetches from a
registry).

`run_exclusive`'s per-manager mutex already provides the safety this loop is being blunt about.

**Proposed [RULING]:** group by contention domain — serialise the `needs_root()` set, fan the
rest out at `max_parallel`. On a machine with apt + cargo + npm + pipx that turns four
sequential multi-minute refreshes into one plus three overlapped.

## I-10 · `linix fleet` talks to hosts one at a time **[READ]**

`src/app/fleet.rs:102`, `:172`

```rust
for host in &hosts {
    match ssh_capture(host, "linix status --json").await { ... }
}
for h in targets {
    match ssh_capture(&h.host, "linix sync -y").await { ... }
}
```

A fleet tool whose entire purpose is N machines, contacting them serially. Every host pays the
full SSH handshake plus the remote command's runtime, added end to end. Ten hosts at 3s each is
30s where it should be 3s.

Nothing shared, nothing ordered, no contention — this is the cleanest `buffer_unordered` in the
tree and the highest ratio. Cap at `max_parallel` (or a dedicated `fleet_parallel`, since this
is network fan-out, not CPU — see I-19).

## I-11 · `check health` probes ~55 backends serially, twice **[MEASURED]**

`src/verbs/check.rs:241` and `:664`

```rust
for b in app.registry.all() {
    if let Ok(r) = b.core().check_health().await { ... }     // line 241, the rollup
}
for b in app.registry.all() {
    let mut report = match b.core().check_health().await { ... }   // line 664, the detail
}
```

Two serial passes over every registered backend. `check_health` is a real probe for several
backends (`psresource` asks PowerShell about cmdlets; `generic` backends probe their binary).
The docs measure the `check` rollup at **10.4s** with no budget.

The `list` probe that follows (line 725) *is* concurrent — at a hardcoded
`buffer_unordered(4)`, with a 60s timeout each.

Two more O(n²) scans ride along in the same function: `wanted.iter().any(|w| w == b.name())`
inside the loop (line 672), and `reports.iter_mut().find(|(n,_)| *n == name)` inside the
result loop (line 731).

## I-12 · `adopt` scans every backend serially, twice, and the concurrent version exists **[READ]**

`src/app/adopt.rs:112` and `:120`

```rust
for backend in self.registry.available() {
    for (_installer, pkg) in q.owned_system_packages().await { ... }   // pass 1
}
for backend in self.registry.available() {
    match queryable.list_manual().await { ... }                        // pass 2
}
```

`AppContext::owned_system_package_names()` (`context.rs:695`) asks the *identical* first
question through `query_backends_concurrently`. `adopt` reimplements it serially. Pass 2 is a
`list_manual()` per backend with nothing shared between them.

Also note both loops call `registry.available()` afresh — see I-14.

Fix: call `owned_system_package_names()` for pass 1; `query_backends_concurrently` for pass 2.

## I-13 · The resolver asks the priority chain one declaration at a time **[READ]**

`src/app/sync/resolver.rs:771`

```rust
for question in questions {
    ...
    let verdicts = self.ask_the_chain(&chain, &name, constraint.as_deref()).await;
```

`ask_the_chain` is carefully concurrent *within* one name (`FuturesOrdered` bounded by
`max_parallel`, with a good comment explaining why ordered and not unordered). The loop over
questions around it is fully serial. Every bare name in the manifest waits for the previous
one's remote lookups to finish.

Each `ask()` (line 1198) is `remote_has()` and, if inconclusive, a full `search()` — network
calls against registries. A manifest with 60 bare names serialises 60 chains of network round
trips.

The loop is serial because `lock.record()` mutates shared state inside it. That is a false
constraint: collect `(name, chain)` → resolve concurrently → apply lock records in the
questions' original order afterwards. Determinism is preserved because the *ordering of writes*
is restored, and `ask_the_chain` already guarantees the winner within a chain is
order-independent.

Two smaller things in the same function:

- **Line 721:** `questions.iter().find(|q| q.name == name)` inside the build loop — O(n²) over
  declarations. A `HashMap<String, usize>` index makes it O(n).
- **`ask()` results are never memoised.** Two declarations naming the same package re-ask the
  whole chain. `PackageCache::search` (I-4) is sized for exactly this and unread.

## I-14 · `registry.available()` re-probes PATH for every backend, at 20+ call sites **[READ]**

`src/backends/registry.rs:47`

```rust
pub fn available(&self) -> Vec<Arc<BackendCapabilities>> {
    self.backends.values().filter(|b| b.is_available()).cloned().collect()
}
```

`is_available()` for nearly every backend is `self.executor.command_exists_sync(binary)` →
`which::which(cmd).is_ok()` — an in-process PATH scan, **uncached**. On Windows a *miss* walks
every PATH entry × every `PATHEXT` extension, so one probe is tens of `stat` calls and a miss
is the common case (most of ~45 registered backends are not installed on any given host).

`available()` is called at **20+ sites**, including six times in `context.rs` alone (459, 493,
558, 651, 668, 695) and twice in adjacent lines in `adopt.rs` (112, 120).
`installed_but_unmanaged()` calls it and then calls `owned_system_package_names()`, which calls
it again.

Exactly one backend caches its probe:

```rust
// flatpak.rs:52
self.available.get_or_init(|| self.executor.command_exists_sync("flatpak"))
```

One backend got the fix; the other ~44 did not. This is a textbook family and it is very likely
a real component of the measured **7m59s of pre-flight before `uninstall choco:bat` reached its
first child process**.

**Fix:** memoise at the executor — a `DashMap<String, bool>` in `CommandExecutor` behind
`command_exists_sync`. One line, closes all 45 instances at once, and is strictly more correct
than the per-backend `OnceCell` because it dedupes across backends that probe the same binary
(`krew` probes `kubectl`; `yay`/`paru`/`pacman` overlap). PATH does not change during a
one-shot CLI run.

## I-15 · The removal guard asks each backend for essentials serially — and once per package **[READ]**

`src/app/sync/guard.rs:262`

```rust
for name in backends {
    match q.essential().await { ... }
}
```

Serial across backends, on every removal path. Each `essential()` is a subprocess.

Worse, `src/verbs/cleanup.rs:531`:

```rust
for spec in packages {
    ... essential_names(&app.registry, &set).await
}
```

**The whole essential-package set is recomputed per package.** Removing 40 packages runs the
per-backend essential query 40 times over. It is the same answer every time — the OS's essential
set does not change mid-command. Hoist it out of the loop and fan the inner query out.

## I-16 · `insight` fetches vulnerability advisories one HTTP request at a time **[READ]**

`src/app/insight.rs:287`, `:292`

```rust
for (qi, ids) in per_query_ids.iter().enumerate() {
    for id in ids {
        match client.get(format!("{}/{}", OSV_VULN_URL, id)).send().await { ... }
```

A nested serial loop of network GETs against the OSV API. The comment above it says advisories
are cached by id and the total is capped — good — but the requests themselves are strictly
sequential. A scan across a few hundred managed packages with a handful of advisories each is
minutes of pure round-trip latency.

Same file, line 702: `for (qb, qn) in &all_managed { mp.get_dependencies(qn).await }` — a
subprocess per managed package, serial.

Same file, line 38: `q.info(&name).await` per managed package — the I-3 family.

## I-17 · Dependency expansion in the planner is serial **[READ]**

`src/app/sync/planner.rs:650`, `:703`

```rust
for spec in targets { if let Ok(native_deps) = p.get_dependencies(&spec.name).await { ... } }
for spec in &roots  { if let Ok(deps)        = p.get_dependencies(&spec.name).await { ... } }
```

One subprocess per spec, serially, in the function that runs before `identify_needed_actions`
does its fan-out. The fan-out downstream cannot recover time lost upstream.

## I-18 · Other serial loops worth a pass

| site | what is serial | note |
|---|---|---|
| `verbs/cleanup.rs:15` | `list_orphans()` per backend | read-only, independent |
| `verbs/cleanup.rs:130` | `clean_cache()` per backend | independent per manager |
| `verbs/sync.rs:210` | `unreachable_warning()` per backend | **network reachability probes**, serial |
| `core/snapshot.rs:650` | `p.is_available().await` per provider | independent probes |
| `verbs/packages.rs:720` | `s.remote_info(&p.name)` per package | network call per package |
| `verbs/packages.rs:539,544` | nested package × backend `info()` | I-3 family, squared |
| `app/apply/prereq.rs:66,77,106` | probe per manager, per row, per name | triple-nested serial probes |
| `app/apply/dotfiles.rs:174,183` | per tree, per placement | filesystem, independent |
| `verbs/check.rs:749,753` | a command spawn per prereq row | nested serial |
| `app/export.rs:226` | `try_exists` per format | trivially concurrent |
| `app/shell/mod.rs:193,234,257` | `try_exists` / probe per path | trivially concurrent |
| `backends/github.rs:582,648,741,826,875` | per-pick download / copy / verify | **downloads, serial** — the highest-value one in this table |
| `backends/web.rs:125` | `client.head()` per spec | serial HEAD requests |

`github.rs` deserves its own look: a release with several assets downloads them one after
another, and `web.rs:125` does a serial HEAD per declared URL. These are pure network waits with
no ordering constraint between them.

---

# Tier 3 — repeated and eager work

## I-19 · `max_parallel` is one number doing three incompatible jobs **[READ] [RULING]**

`src/config/config.rs:461`

```rust
fn default_max_parallel() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}
```

Core count is the right default for CPU work. It is used for:

- **CPU/process fan-out** — `planner.rs:520`, `context.rs:458`. Correct.
- **Transaction concurrency** — `sync/mod.rs:516`. Defensible.
- **Pure network fan-out** — `search.rs:46` (semaphore over ~22 registry queries),
  `resolver.rs:1177` (the priority chain's remote lookups).

On a 4-core laptop, `linix search` runs 22 network queries in **6 sequential waves**. Nothing
about a network wait wants to be bounded by core count. This is a large part of why `search`
measures 15s–160s.

**Proposed [RULING]:** a separate `network_parallel`, defaulting to something like 16–32
regardless of cores, with `max_parallel` staying the CPU/process knob. Owner ruling 2026-07-17
kept `max_parallel` as a user-settable cap; this splits the knob rather than removing it.

## I-20 · Hardcoded concurrency caps that ignore the knob **[READ]**

| site | cap | should be |
|---|---|---|
| `app/sync/planner.rs:307` (`installed_sets`) | `buffer_unordered(8)` | `max_parallel` — the two fan-outs on either side of it use the config |
| `verbs/check.rs:725` (health list-probe) | `buffer_unordered(4)` | `max_parallel` (or `network_parallel`) |
| `core/transaction.rs:41` (`TransactionConfig::patient`) | `max_concurrent: 4` | overridden at `sync/mod.rs:516`, but it is the value any other constructor gets |

The docs already flag the first: *"`installed_sets` caps at a hardcoded `8` where the two
fan-outs either side of it use `config.max_parallel`."* It is a family of three.

## I-21 · `search` is bounded by its slowest backend, with no timeout and no cache **[MEASURED]**

`src/app/search.rs`

Measured at 15.5s / 25.5s / 48.0s / 160.2s across four runs — the docs' verdict is that "the
number is weather", which is exactly the diagnosis: **there is no per-backend deadline**, so the
command's latency is the max over ~22 registries rather than the median. One rate-limited GitHub
API call or one slow npm registry sets the whole runtime.

Three fixes, independent:

1. **A per-backend timeout.** `check health` already does this — `tokio::time::timeout(60s,
   q.list_installed())` at `check.rs:711` — with a comment explaining how the number was chosen.
   `search` has no equivalent. A backend that has not answered in N seconds contributes nothing
   and says so, exactly as `check` does. **[RULING]** — it changes what a user sees.
2. **Stream results as they arrive** instead of buffering until every backend finishes
   (line 79's `while let Some(task_result) = worker_pool.join_next().await` already has them one
   at a time; only the final `sort` forces the wait). Perceived latency drops to the *first*
   answer.
3. **Use the search cache.** `PackageCache::search` exists with a 600s TTL and is unread (I-4).

Two micro-costs in the same function: `format!("{}:{}", pkg.backend, pkg.name)` allocates a
`String` per result purely for a dedup key (use a `(&str, &str)` tuple), and
`sort_by_key(|p| p.name.to_lowercase())` allocates a fresh lowercase `String` **on every
comparison** — `sort_by` with a cached key vector, or `sort_unstable_by(|a,b|
a.name.to_lowercase().cmp(...))` precomputed, avoids O(n log n) allocations.

## I-22 · Startup does eager, serial, unconditional work on every invocation **[READ]**

`src/app/context.rs:44-105` — `App::new_with_executor_and_state_path`, run for `linix list` as
much as for `linix sync`:

```rust
let hooks    = Arc::new(LuaHooks::new(&config)?);            // constructs a rhai Engine
let registry = Arc::new(create_default_registry(...).await); // ~61 registrations
let progress = create_progress_reporter(...);
let state    = spawn_blocking(StateRegistry::load_default).await?;
let snapshot_manager = Arc::new(SnapshotManager::new(...).await);
let journal  = Arc::new(Mutex::new(Journal::at(journal_path)?));
let scheduler = Arc::new(SchedulerManager::new()?);
let diagnostics = Arc::new(FailureDiagnosticEngine::init(&config_arc).await);
```

Four problems:

1. **It is a straight line.** The state load, the snapshot manager, the diagnostics DB load and
   the journal open are independent I/O. `tokio::try_join!` overlaps them for free.
2. **`LuaHooks::new` builds a `rhai::Engine` eagerly** (`hooks.rs:22`), registering the whole
   standard package, for every command including ones that run no hooks. Make it lazy
   (`OnceCell`) — the Lua side already is (`hooks.rs:175` constructs `Lua` inside the blocking
   closure).
3. **`FailureDiagnosticEngine::init` reads and parses `diagnostics.json` on every run**
   (`diagnostics.rs:~60`) to have rules ready in case something fails. Load it on first
   failure.
4. **~61 backend registrations**, each allocating capability structs, `ExitPolicy` tables and
   `Arc`s, for a command that will touch one backend or none.

## I-23 · The config file is parsed twice per invocation **[READ]**

`src/main.rs:77` and `src/main.rs:834`

```rust
// line 77 — synchronous, on the runtime thread, to read command_aliases + verbs
let prefs = preferences_path_from_argv(&raw_argv)
    .and_then(|p| linix::config::Config::from_file(&p).ok());
...
// line 834 — the real load, correctly on spawn_blocking
let mut config = spawn_blocking(move || Config::from_file(&path)).await??;
```

Two full reads and TOML parses of `preferences.toml`, on every command. The first is also a
blocking file read on a runtime worker thread. `locate()` similarly runs at `main.rs:811` and
again inside `load_and_merge_config`.

Fix: parse once, thread the result through. The alias expansion genuinely must happen before
clap; that argues for doing the *single* parse early, not for doing it twice.

## I-24 · Regexes recompiled per call, in parsing paths **[READ]**

`regex::Regex::new` is not cheap — it compiles and optimises an automaton. These sites do it on
every invocation:

| site | context |
|---|---|
| `app/diagnostics.rs:98` | `for rule in &self.db.rules { if let Ok(re) = Regex::new(&rule.pattern) }` — **every rule recompiled on every `diagnose()` call** |
| `app/diagnostics.rs:226` | the same loop again, in the learning path |
| `core/snapshot.rs:434` | `list_pattern` recompiled on every snapshot `list()` |
| `core/snapshot.rs:397` | `create_id_pattern` recompiled per create |
| `backends/service.rs:134` | recompiled per parse |
| `backends/firewall.rs:88` | `list_pattern` recompiled per list |
| `backends/onboarder.rs:183` | recompiled per definition load |
| `parsers/utils.rs:44` | `extract_version_bracketed` — **recompiles on every package line parsed** |
| `app/sync/resolver.rs:673` | recompiled per call |
| `backends/artifact/pattern.rs:58` | per pattern (arguably correct — patterns are user data) |

The correct shape is already in this tree twice: `parsers/utils.rs:4` (`static ANSI_REGEX:
Lazy<Regex>`) and `core/validator.rs:10-32` (four `Lazy` regexes). For patterns that come from
config rather than source, compile once when the config is loaded and store the `Regex` in the
struct — `DiagnosticDb`, `SnapshotDef`, `ServiceDef` and `FirewallDef` should each hold a
compiled regex, not a `String` pattern.

## I-25 · Windows: `which::which` on every single spawn **[READ]**

`src/core/executor.rs:319`

```rust
fn windows_effective_command(cmd: &str, args: &[String]) -> (String, Vec<String>) {
    if let Ok(resolved) = which::which(cmd) {                 // sync PATH scan, EVERY spawn
        if let Some(wrapped) = windows_shim_wrap(cmd, &preferred_shim(&resolved), args) {
```

Called from `RawExecutor::execute` for every command LiNix runs. `preferred_shim` then does an
additional synchronous `ps1.is_file()` stat. Both are blocking filesystem work inside an
`async fn`, on the same task that `buffer_unordered` is relying on to interleave.

The docs name this as the first suspect for the measured **max concurrent = 1** against
`buffer_unordered(20)`, and the mechanism is real: `identify_needed_actions`'s futures all live
on one task ("the futures borrow `&self` so this stays on one task"), so a single synchronous
call anywhere beneath `info -> list_installed -> run_output` blocks all 20. This is that call,
and it runs on every spawn.

**Fix:** memoise the resolution — `DashMap<String, (String, Vec<String>)>` keyed on the command
name (the arg rewriting is a pure function of the resolved path). Same cache serves I-14. If the
serialisation persists after that, the experiment the docs propose (an `AtomicUsize` around
`spec_is_missing`) settles which half of the stack to open next — but fix the known blocking
call before running it.

## I-26 · `state.save()` writes pretty-printed JSON **[READ]**

`src/core/state.rs:172` — `serde_json::to_string_pretty(self)`

For a machine-read registry of every managed package. Pretty-printing roughly doubles the byte
count and costs serialisation time proportional to the state size, on a file rewritten in full
after every mutation. It is not a file users edit — `linix` reads it back. Use `to_string`, or
`to_string_pretty` only behind a debug flag.

Related: `state.save()` appears at 17 call sites; check `verbs/cleanup.rs:420-469` and
`verbs/upgrade.rs:185,312` for saves inside or adjacent to per-package loops — a full state
rewrite per package is the failure mode to rule out.

## I-27 · Up to 998 sequential filesystem probes to pick a filename **[READ]**

`src/app/export.rs:180`

```rust
for n in 2..1000 {
    let p = out_dir.join(format!("{}.{}", beside(name), n));
    if !tokio::fs::try_exists(&p).await.unwrap_or(false) { return p; }
}
```

One `await`ed stat per candidate, plus a `format!` allocation each. A single `read_dir` of the
target directory answers the same question in one syscall. Minor in absolute terms; it is here
because it is the clearest example of a shape that recurs.

## I-38 · The exit policy builds three full lowercase copies of every command's output **[READ]**

`src/core/exit_policy.rs:164`

```rust
fn haystack(stdout: &[u8], stderr: &[u8]) -> String {
    let mut hay = String::from_utf8_lossy(stdout).into_owned();   // full copy
    ...
    hay.push_str(&String::from_utf8_lossy(stderr));               // + stderr
    hay.make_ascii_lowercase();                                   // full pass
    hay
}
```

`ensure_status` calls three separate policy questions, and **each builds its own haystack**:

- `executor.rs:954` → `signals_failure(&output.stdout, &output.stderr)`
- `executor.rs:999` → `retryability(&output.stdout, &output.stderr)`
- `executor.rs:1005` → `names_an_absent_package(&output.stdout, &output.stderr)`

Three allocations of the entire stdout+stderr and three linear lowercase passes, **per command
LiNix runs**. The docs record a single scoop typo producing ~110 lines of bucket commits; an
`apt install` transcript or a `cargo build` log is far larger. With I-1 unfixed this is per
package.

**Fix:** build the haystack once in `ensure_status` and pass `&str` to all three predicates.
`explaining_lines` already takes a `&str` and shows the shape.

## I-39 · Download backends rewrite their whole state file, pretty-printed, per package **[READ]**

`github.rs:270` (`save_state_internal`) · `web.rs:86` · `appimage.rs:81`

```rust
let data = serde_json::to_string_pretty(state).map_err(Error::from)?;   // the whole map
crate::utils::file::persist(&self.state_file, &data).map(|_| ())
```

Called from `github.rs:929,991`, `web.rs:315,366`, `appimage.rs:212,248` — the install and
remove paths. Because I-1 makes every install its own node, installing 10 `github:` releases
rewrites the entire GitHub state file 10 times, pretty-printed, each behind an
`internal_lock` mutex. Same O(n²) shape as the journal (I-3b), same fix: batch the writes, or
write once at the end of the transaction.

## I-40 · The reachability warning spawns a subprocess per backend, serially **[READ]**

`src/verbs/sync.rs:210` → `app/reachable.rs:197` → `user_bin_dirs` → `asks_the_tool` (line 74),
which runs the manager to ask where it puts binaries (`npm prefix -g`, `go env GOPATH`, …).

One subprocess per backend, in a serial `for` loop, on the sync path. Independent per backend,
purely informational, and trivially a `buffer_unordered`.

## I-41 · `generate:` statements run one script at a time **[READ]**

`src/app/sync/resolver.rs:434`

```rust
for (cmd, origin, gates) in gens {
    ... // approval check, then run the generator and parse its stdout
}
```

Each generator is a subprocess whose stdout becomes declarations. They are independent of one
another — the merge happens afterwards — so they can run concurrently at `max_parallel` with the
results reassembled in declaration order (the same ordered-results pattern `ask_the_chain`
already uses).

## I-47 · Post-sync health checks run serially, and the port probe has no timeout **[READ]**

`src/app/sync/mod.rs:443`, `:478`

```rust
for check in &checks {
    if Self::probe_ok(&check.probe).await { ... }        // serial
}
...
Probe::Port(p) => tokio::net::TcpStream::connect(("127.0.0.1", *p)).await.is_ok(),
```

Two things:

- **Serial.** Every declared `@health=` check runs one after another, after the sync, while the
  user waits to learn whether their change is being reverted. They are independent probes;
  nothing orders them. `verify_health` already collects `failed` into a `Vec`, so a
  `buffer_unordered` with results collected changes nothing about the outcome.
- **`TcpStream::connect` carries no timeout.** A *closed* localhost port refuses immediately, so
  this is fine in the common case. A **filtered** port — a firewall rule that drops rather than
  rejects, which `apply/firewall.rs` can itself create — hangs for the OS connect timeout,
  ~21 s on Windows and ~130 s on Linux. That is a health check that decides whether to roll back
  a sync, blocking on a syscall default.

  Every other bounded wait in this tree states its number and its reasoning —
  `check.rs:711`'s 60 s, `command_idle_timeout_secs`'s 900 s. This one has no bound at all.
  `tokio::time::timeout` around the connect, with the number written down, is the fix.
  **[RULING]** only if the chosen bound is user-visible; a plain `timeout` is detail.

`Probe::Command` routes to `bisect::run_test`, which is outside the executor's
`command_idle_timeout` path — worth confirming that a wedged health command cannot hang a sync
the way I-47's port probe can.

## I-42 · O(n²) linear scans over `Vec` inside loops **[READ]**

A mechanical sweep found 73 candidate sites. Most are `HashSet`/`HashMap` lookups (O(1)) and
fine. These are genuine `Vec` scans inside a loop over a set that grows with the config:

| site | scan |
|---|---|
| `model/resolve.rs:697` | `base.retain(\|s\| keep.iter().any(...))` — **retain × any, over all statements** |
| `model/resolve.rs:458` | `wanted_modules.iter_mut().find(...)` per reached module |
| `model/resolve.rs:360`, `:573`, `:924`(×3) | linear `contains` per statement / per scope |
| `model/edit.rs:425`, `:670`, `:685`, `:723` | `out.contains(...)` / `stmts.iter().any(...)` per edit |
| `model/priority.rs:46`, `:82` | `backends.iter().any(...)` per priority entry |
| `model/profiles.rs:269`, `:457` | `out.iter().any(\|e\| e.name == ...)` per entry |
| `model/groups.rs:130`, `:132` | per member |
| `app/sync/resolver.rs:721` | `questions.iter().find(...)` per declaration (also I-13) |
| `backends/service.rs:168`, `setting.rs:178`, `firewall.rs:146` | `out.iter().any(...eq_ignore_ascii_case)` per row — case-insensitive compare inside the scan |
| `backends/nix.rs:92` | `installed.iter().find(...)` per name |
| `core/artifact_lock.rs:149` | `locked.iter().find(\|l\| &l.asset == asset)` per asset |
| `core/retention.rs:123` | `kept.contains(...)` per item |
| `verbs/check.rs:672`, `:731` | per backend / per report (also I-11) |
| `verbs/packages.rs:214`, `:241` | per edit |
| `verbs/declare.rs:318`, `:375`, `:673` | `managers.iter().any(...)` per spec |
| `app/insight.rs:707` | `deps.iter().any(...)` per managed package — inside the already-serial dependency loop (I-16) |
| `app/profile.rs:150`, `app/rebuild.rs:146`, `app/adopt.rs:151,192` | per entry |

None of these is where the seconds are today — the seconds are in subprocesses and sockets. They
matter after Tiers 0–2 land, and `resolve.rs:697` and the `edit.rs` cluster are the two worth
doing regardless, because they scale with manifest size and the manifest is the thing users
grow.

---

# Tier 4 — allocation and micro-cost

Not where the seconds are. Worth a sweep once Tiers 0–2 land, because after those the process
spends proportionally more time in its own code.

## I-28 · Counts

`.clone()` — **1,291** · `.to_string()` — **1,227** · `format!` — **1,042**, in non-test code.

For a program of this size those are not alarming numbers on their own. The concentrations are:

- **`Package` carries `properties: HashMap<String, String>`** (`core/package.rs:6`). Every
  `list_installed()` builds N of these HashMaps; `planner.rs:300` then does
  `.map(|p| p.name).collect::<HashSet<_>>()` and throws every HashMap away. A listing of 600
  packages allocates 600 HashMaps to keep 600 Strings. A `list_names()` on `Queryable`, or a
  lazily-populated properties field, avoids the whole allocation.
- **`format!("{}:{}", backend, name)` as a map key** appears throughout the planner, guard and
  search paths — inside loops over every package. `(String, String)` tuple keys or a small
  `PackageKey` newtype with a borrowed lookup form avoids the per-lookup allocation.
- **`registry.get_filtered`** (`registry.rs:58`) does
  `enabled.contains(&b.name().to_string())` — allocates a `String` per backend per comparison,
  inside an O(n·m) scan. `enabled.iter().any(|e| e == b.name())` allocates nothing.
- **`parsers/utils.rs::sanitize`** allocates three Strings per call (`replace_all` → `replace`
  → `trim().to_string()`) and runs on every command's output. A `Cow`-returning version costs
  nothing when the input has no ANSI, which is the common case on Linux.

## I-29 · The transaction scheduler rescans the whole graph per completion **[READ]**

`src/core/transaction.rs:218`

```rust
while self.completed_indices.len() < total_nodes {
    let ready_nodes: Vec<NodeIndex> = self.graph.node_indices()
        .filter(|&idx| !completed.contains(&idx) && !in_progress.contains(&idx)
             && graph.neighbors_directed(idx, Incoming).all(|d| completed.contains(&d)))
        .collect();
    ...
    if let Some(finished) = worker_pool.join_next().await { ... }   // reaps exactly ONE
}
```

Each outer pass rescans every node and every incoming edge, then reaps a single completion —
O(V·(V+E)) over the run. With 300 packages that is ~100k redundant edge checks. The standard
fix is an in-degree counter decremented when a dependency completes, with a ready queue; it is
also clearer than the filter.

Also: `in_progress.remove(&idx)` happens only on the success branch (line 274). The failure
branch leaves the index in `in_progress`, which is harmless today because a failure triggers
rollback and returns — but it is a latent correctness edge if that ever changes.

Not urgent. It only matters once I-1 makes the graph wide enough to notice.

---

# Tier 5 — build and repository

## I-30 · Build profile is correct **[READ]**

`Cargo.toml:106` — `opt-level = 3`, `lto = true`, `codegen-units = 1`, `panic = "abort"`,
`strip = true`. Nothing to fix. Noted so nobody re-audits it.

Two things that would still help: narrowing `tokio`'s `features = ["full"]`, and dropping
`rayon` / `nonzero_ext` / `reqwest`'s `blocking` (I-7, I-8). All three are build-time and
binary-size only.

## I-31 · A 3 MB untracked file sits inside `src/` **[READ]**

`src/project_dump.txt` — 3,143,593 bytes, gitignored, untracked, containing a concatenated copy
of much of the source tree. It does not compile, but it doubles every `grep`/ripgrep pass over
`src/`, and it makes tooling report each finding twice. Local artifact; delete it or move it out
of `src/`.

---

# New findings — 2026-08-05 **[MEASURED HERE]**

Found while diagnosing the Windows stall, on the same host, with LiNix's own `--timings`. Both
are new to this document; neither is in I-18's table of serial loops.

## I-48 · `heal` bypasses the transaction DAG entirely — serial, unbatched, and doomed **[MEASURED HERE]**

`app/sync/mod.rs:718` is a plain `for entry in incomplete_actions` with the install awaited inside
it, and each call is `handler.install(std::slice::from_ref(spec), sudo)` — **one package per
command**. So recovery re-implements install dispatch by hand, next to a batched parallel engine,
and gets none of it:

- **serial** — the I-9 family, one entry at a time;
- **one node per package** — I-1, which was fixed *in the DAG* and never here;
- **doomed** — it acts on `Failed` entries as well as interrupted ones, and every failed attempt
  writes a *new* journal operation, so the work grows without bound (`Q33`).

**Measured on this host, both numbers from LiNix:**

```
sync --dry-run   2.65s wall ·  21 child command(s) summing to 10.35s · 3.9x overlap ·  2 wave(s)
heal           205.14s wall ·  27 child command(s) summing to 33.31s · 0.2x overlap · 27 wave(s)
```

**27 waves for 27 commands is the definition of serial**, and 0.2x overlap against the DAG's 3.9x
on the same machine in the same minute. 23 of the 30 recovery attempts were the *same* package —
which a batched engine would have sent as one command, and a correct one would not have sent at
all.

Note also what the 205s is *not*: only 33s of it is inside child commands. The rest is I-49.

**This is the "two of everything" shape the repo keeps finding**, not merely a missing
`join_all`: the fix is to route recovery through the transaction engine rather than to
parallelise a second copy of it. The 2026-08-02 ruling covers this — *restructure if it takes
that* — but the loop that decides **which** entries to run is `Q33`, which is OPEN, and it is the
same loop. They should land together.

## I-49 · `github:` downloads the artifact, then checks whether it may deploy it **[MEASURED HERE]**

`utils/file.rs:225` `deploy_executable` takes an already-downloaded, already-extracted `src`, and
its refusal — `is_ours(dest, owned_root, recorded)` — reads only the **destination**. It needs
zero downloaded bytes and it runs after the fetch.

Measured inside one `heal`, twice, back to back:

```
 60.9s  then  119.1s   -> "refusing to deploy `fd.exe`: ...\.localind.exe already exists
                           and LiNix did not create it."
```

**180 of that run's 205 seconds were spent fetching a file it was always going to reject.**

Two things make it invisible rather than merely slow. It is an in-process `reqwest` download, so
it is **not a child command** and never appears in the `--timings` breakdown — the 205s wall
against 33s of children is entirely this. And `core/http.rs` gives downloads no whole-request
timeout, correctly (a large download must not be capped by wall clock), which leaves an
*avoidable* download both unbounded and silent. Three stalls were misdiagnosed as wedges because
of it.

**Fix:** hoist the ownership test above the fetch. It is already a pure function of `dest`.

**All three download backends share the ordering — checked, not assumed:**

| site | proof it has already paid before it asks |
|---|---|
| `backends/github.rs:887` | deploys from `downloaded[i]` (`:838`) — the artifact is fetched and picked first |
| `backends/web.rs:289` | `extract_archive(...)` runs at `:236`, before `bin_destination` at `:265` |
| `backends/appimage.rs:181` | same shape: `bin_destination` at `:180` reads a path that is already on disk |

So the wasted work is a download **and** an extraction, on every one of them, before anything
asks whether the destination may be written at all.

---

# Suggested order of work

> **Done, 2026-08-02.** Rounds 0–4 below were worked in this order and the disposition of every
> finding is the table at the top of this file. The sequence is kept because the *reasoning* for
> it — each step making the next one measurable — is the part worth reusing, and because the two
> steps it names that were **not** taken are named there too: the four measurements it asks to
> re-run, and the `AtomicUsize` experiment on `spec_is_missing`.

Sequenced so each step makes the next one measurable.

**Round 0 — one-line fixes with disproportionate reach.** Do these first; they are cheap and
several of them unblock the measurements that judge everything else.

0. **I-44** — `ask()` calls `search` once and answers all three questions from the result,
   instead of running the identical search two or three times. Pure deletion; halves the
   resolver's network cost before I-13 even parallelises it.
1. **I-4b** — one shared `reqwest::Client` behind a `OnceCell`. Restores HTTP keep-alive across
   the entire network layer. Touches `search`, `insight`, `github`, `vscode`, `pip`, `npm`.
2. **I-14 + I-25** — memoise `which::which` in `CommandExecutor` (`DashMap<String, bool>` plus a
   resolution cache). Closes ~45 backends' repeated PATH scans **and** removes a known blocking
   call from the fan-out's task.
3. **I-32** — wrap `DesiredState::resolve()` in `spawn_blocking`. One line; stops the whole
   model load starving the runtime.
4. **I-33 + I-34** — `spawn_blocking` around the checksum calls. Removes the other two known
   blocking calls from the fan-out's task.
5. **I-38** — build the exit-policy haystack once instead of three times per command.

**Round 1 — free, invisible, large.** No behaviour change, no ruling needed.

5b. **I-43** — memoise the resolved model and host facts on `App` (`OnceCell`). One command
   currently resolves 4–6 times, re-running the user's vars providers and `generate:` scripts
   each time. Largest pure redundancy in the tree, and it stops user code executing N times.
5c. **I-45** — index `state.packages`; stop `is_held` allocating a `format!` per call inside the
   planner's fan-out.

6. **I-4 + I-3(2)** — wire `PackageCache::installed` into `Queryable`, making `info()` a set
   lookup after the first listing per backend. The single largest ratio in this file.
   Also closes I-46's `winget`/`choco`/`emacs` cases, which have no cheaper question available.
7. **I-3(1)** — delete or condition the post-install `info()` at `transaction.rs:477`.
8. **I-3b** — the journal becomes append-only JSONL, off the runtime thread. This is the hard
   throttle under the DAG, and it gets *worse* as Round 3 widens the graph — do it before, not
   after.
9. **I-20** — the three hardcoded caps read `max_parallel`.
10. **I-5, I-6, I-7(rayon/nonzero_ext), I-8, I-31** — deletions.

**Then measure.** Re-run the four numbers the docs already hold, on the same host: `linix plan`
(was 439.6s), `install choco:bat` end to end (was 399.48s, of which 18.75s was real work),
`uninstall choco:bat` pre-flight (was 7m59s), `check` rollup (was 10.4s). Rounds 0–1 should move
all four substantially.

**Only then** run the `AtomicUsize` experiment on `spec_is_missing`. Four blocking calls on that
task are now known (I-25, I-32, I-33, I-34); the experiment is worth running as *confirmation
that they were the cause*, not as a diagnostic — and if the width is still 1 afterwards, that is
a genuinely new finding rather than a rediscovery.

**Round 2 — parallelism with no user-visible semantics.**

11. **I-13** — concurrent chain resolution with ordered lock writes.
12. **I-12, I-11, I-15, I-16, I-17, I-18, I-40, I-41** — the serial read fan-outs. I-15's
    hoist (`essential_names` out of the per-package loop) and I-16 (serial OSV GETs, now over a
    pooled client) are the two largest.
13. **I-10** — `fleet` fans out. Highest ratio in the file for the smallest diff.
14. **I-22, I-23** — startup overlap; parse the config once; lazy `rhai::Engine`; lazy
    diagnostics DB.
15. **I-36, I-39** — stop cloning the whole state to save it; stop rewriting the download
    backends' state files per package.

**Round 3 — needs a ruling.** Bring these as questions with this document attached.

16. **I-1 + I-2 [RULING]** — batch installs per backend. The largest single win in the file and
    the one with real rollback-granularity design in it.
17. **I-35 [RULING]** — overlap the Windows restore point with planning instead of barriering on
    it, and *say it is happening*. A fixed 51s on every Windows mutation.
18. **I-9 [RULING]** — `upgrade` fans out across non-root managers.
19. **I-19 [RULING]** — split `network_parallel` from `max_parallel`.
20. **I-21 [RULING]** — per-backend deadline on `search`, and stream results.

**Round 4 — the sweep.** I-24 (regex hoisting), I-26, I-28, I-29, I-27, I-37, I-42.

---

---

# Coverage — how completely was this looked at

The first pass was targeted and therefore not trustworthy as a completeness claim. The second and
third passes were mechanical, over **all 175 `.rs` files in `src/`**, so the *classes* below are
enumerated rather than sampled:

| sweep | method | result |
|---|---|---|
| serial `.await` in a loop | brace-tracked scan of every non-test `for` body | **169 sites**, all listed or classified |
| blocking call inside `async fn` | 20 blocking patterns × every `async fn` body, `spawn_blocking` excluded | **~90 hits**, grouped in Tier 0.5 |
| nested linear scan in a loop | pattern scan of every loop body | **73 candidates**, `Vec`-backed ones listed in I-42 |
| allocation-dense loops | ≥4 allocating expressions per loop body | **35 loops**, top ones in I-28 |
| per-call regex compilation | every `Regex::new` not behind `Lazy`/`static` | **11 sites**, listed in I-24 |
| HTTP client construction | every `Client::new`/`Client::builder` | **8 sites**, all per-call — I-4b |
| dependency liveness | usage count per `Cargo.toml` entry | 2 dead, 3 overlapping — I-7 |
| cache/dead-code liveness | caller search for every public accessor | `PackageCache` and `AppCore` dead — I-4, I-5 |
| per-manager argv choice | `list_installed`/`search`/`info` argv extracted per backend and compared against the cheaper question | **4 cases** — I-46 |
| trait defaults that hide real work | every default method body in every trait, checked for `.await`/subprocess/search | **1 case**, and it is duplicated by its caller — I-44 |
| model-resolution redundancy | every `StateResolver::new` site, traced to what re-runs | **39 sites**, 4–6 resolutions per command — I-43 |
| unbounded waits | every `connect`/`wait`/`lock` without a `timeout` | 1 uncovered — I-47 |

**What that does and does not license.** It means: for these classes, the *enumeration* is
complete over `src/`, and any instance not listed here is one the scanner's patterns did not
match rather than one I chose to omit. It does not mean every finding is correctly *prioritised*,
and it certainly does not mean the program has no other inefficiencies — a lexical sweep cannot
see an algorithm that is the wrong algorithm, a data structure that is the wrong shape, or a
question that did not need asking. **I-1, I-3 and I-3b were all found by reading, not by
scanning**, and they are the three biggest items in this document. That asymmetry is the honest
summary of how much confidence a scan buys.

**Classes deliberately not swept**, and named so nobody assumes they were:

- The `tests/` tree's own runtime (scanned only for cache callers).
- Binary size and compile time beyond the dependency check.
- Memory high-water mark — nothing here measures allocation *volume*, only sites.
- The shell/completion/TUI paths (`app/repl.rs`, `app/ui/`) beyond their appearance in the
  mechanical sweeps.
- **Parser throughput.** `config/grammar/statement.rs` is 3,117 lines and runs per manifest
  line. It appeared in the allocation sweep (`:793`, `:945`) and the nested-scan sweep, and
  nothing there looked pathological — but it was never profiled, and "looked fine on a read" is
  not the same standard as the enumerated classes above.
- **Whether the parallelism that exists is correctly *bounded*.** Several fan-outs share one
  `max_parallel`; whether the resulting total process/socket count is the right number for a
  given machine is a tuning question no measurement here answers.

---

# Appendix A — how the measured findings were measured

**Environment.** `linix-it-ubuntu:latest` (Ubuntu 24.04, the repo's own integration image, with
`linix` built into it at image-build time), run disposably via WSL Docker:

```
docker run --rm --entrypoint /bin/sh -v <script>:/m.sh linix-it-ubuntu:latest /m.sh
```

Config isolated with `LINIX_CONFIG_DIR` / `LINIX_DATA_DIR`, matching what
`docker/integration/run-in-container.sh` does. `locks/` and the data dir were deleted between
runs so no lockfile or registry state carried over.

**Counting subprocesses.** Each manager binary was replaced *in place* by a shim that logs its
argv and then `exec`s the original:

```sh
mv /usr/bin/apt /usr/bin/apt.real
cat > /usr/bin/apt <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> /tmp/cnt/apt.log
exec /usr/bin/apt.real "$@"
EOF
```

Wrapping *in place* rather than shadowing via `PATH` matters: an earlier attempt put shims on
`PATH` and reported `apt-get invocations: 0`, which was true and useless — the apt backend
declares `binary: None`, so it invokes **`apt`**, not `apt-get`. The in-place wrap cannot be
bypassed by `PATH` order, `sudo secure_path`, or absolute-path resolution.

**Counting model resolutions.** An external vars provider (`vars.sh`) that appends to a counter
file and prints one variable. It must be approved by `linix lock` first — LiNix refuses to run an
unapproved provider (II.12), which is why a first attempt measured zero executions and had to be
re-run after approval.

**Counting spawn attempts.** `strace -f -e trace=execve`. Note that a relative-name spawn produces
one `ENOENT` execve per `PATH` directory tried, so `ENOENT` count measures PATH breadth × spawns,
not distinct probes. For `check drift` with 1 declared package: 80 execve total, 65 `ENOENT`,
15 successful. With 20 declared: 232 total, 179 `ENOENT` — i.e. roughly **8 additional execve per
additional declared package on a read-only command**.

**Scripts.** `scratch-measure.sh` … `scratch-measure4.sh` in the repo root at the time of the
audit. They are throwaway instruments, not tests — delete them, or promote the useful ones into
`scripts/` as a latency gate, which is what W14/R-9 have been asking for.

**The Windows half.** I-3 and I-43 were re-measured natively against a **fresh release build of
the current tree** (`linix 0.7.0`, built 2026-08-02 09:55, `cargo build --release`), config
isolated in `%TEMP%`, using a `vars.bat` provider as the resolution counter. This is what closes
the staleness caveat below for those two findings.

**Two instrument bugs worth recording**, because both produced a confident wrong number first:

- Shims placed on `PATH` reported `apt-get invocations: 0` — true, and useless: the apt backend
  declares `binary: None`, so it runs `apt`. **Wrap in place, not on `PATH`.**
- Deleting `locks/` between Windows runs silently un-approved the vars provider, so every run
  failed fast at ~240 ms and the scaling curve came out flat. **A suspiciously flat curve is an
  instrument failure until proven otherwise** — the first draft of the Windows I-3 table was
  wrong in exactly this way and was thrown out.

**Caveats, stated because they bound what these numbers prove:**

1. **The container's `linix` was built when the image was built**, not from the working tree. It
   is the right instrument for the *structural* findings there — a spawn count, a duplicated
   argv, an execution tally — and I-43 was independently reproduced on a current-tree Windows
   build with the identical result (1/1/1/2/3). **I-1 and I-2 have not been re-measured on
   `HEAD`**; nothing in the 39 newer files touches `GraphAction` or `run_exclusive`, but that is
   an argument, not a measurement.
2. **Two platforms, two manager families.** apt on Ubuntu, winget on Windows. The mechanisms are
   manager-independent; the *costs* are not, which is the whole point of I-3's correction.
3. **Wall-clock on a busy host is noisy.** The process *counts* and *execution tallies* are exact
   and deterministic and reproduced across runs. The millisecond figures are single runs; read
   them as magnitudes and slopes, not as constants.

# What this audit did not do

Stated plainly, because unverified is not done:

- **Nothing here was timed in this session.** Every number quoted is re-cited from
  `docs/spec/history.md` or a `GRADE-*` file, and is labelled. The **[READ]** findings are
  mechanisms verified by reading the call sites — the loop is serial, the cache has no readers,
  the slice is one element — with the cost argued rather than measured.
- **No build was run.** `cargo build --all-targets` / `cargo test` / `cargo clippy` were not
  executed as part of this audit; it changed no code.
- **Async-runtime behaviour was not instrumented.** I-25 identifies a synchronous call on the
  fan-out's task and argues it is the documented `max concurrent = 1`. That is a hypothesis with
  a mechanism, not a proof. The experiment that settles it is already specified in
  `history.md` and is still worth running *after* the fix, to confirm rather than to diagnose.
- **The `tests/` tree was scanned only for cache callers**, not audited for its own runtime.
- **Windows-only paths dominate the evidence base**, because that is where the measurements in
  the docs were taken. `I-14`/`I-25` are strictly worse on Windows (PATHEXT) and still real on
  Linux.
