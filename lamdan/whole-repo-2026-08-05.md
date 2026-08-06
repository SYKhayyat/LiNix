# Lamdan — LiNix, whole repo

**2026-08-05 · branch `grade/2026-07-29` @ `3d321bf` · 390 tracked files, 93,909 lines of Rust in
`src/`, 17,667 in `tests/`, 36,043 in docs.**

> **Provenance, 2026-08-05 (later).** `grade/2026-07-29` was fast-forwarded into `main` and
> deleted, along with `grade/2026-07-28`; `main` is now the only ref. The branch name above is
> kept because it is where this review was run — `3d321bf` still resolves, on `main`. Every line
> count and citation below describes the tree at `3d321bf` and has not been restated against
> anything later. The one change since that touches the files reviewed here is a whole-tree
> `cargo fmt`, which moved line breaks in 26 source files and no line numbers cited below are
> guaranteed to survive it. **Findings are as-of `3d321bf`; none have been re-checked against
> `main`, and none are marked resolved by this note.**

This is not a bug review. It argues about whether the code should exist and whether this is the
way to build it. Correctness findings appear only where a design choice is the reason the bug is
possible.

## Coverage

Twelve regions, every tracked file in exactly one, each read by a separate reviewer with no
knowledge of the others' conventions. **Excluded:** `Cargo.lock`, `target/` (untracked),
`tests/fixtures/**` contents (read as data by the tests region, not reviewed as code),
`src/backends/artifact/real_releases.txt`.

| # | Region | Lines | Coverage |
|---|---|---|---|
| 1 | `src/config/**` — the grammar | 6,950 | full |
| 2 | `src/model/**` — the resolved model | 12,017 | full (non-test bodies) |
| 3 | `src/core/**` — runtime, locks, transaction | 14,237 | full |
| 4 | `src/backends/**` | 23,670 | ~60% full, 13 manager files by symbol+argv scan |
| 5 | `src/parsers/**` | 4,113 | full |
| 6 | `src/app/sync/**`, `apply/**`, `adopt`, `context` | 7,916 | full (non-test) |
| 7 | `src/app/*` — everything else | ~10,575 | 31 of 32 full |
| 8 | `src/cli/**`, `src/verbs/**`, `main.rs` | 10,972 | full |
| 9 | `src/utils/**` | 1,004 | full |
| 10 | `tests/**` | 17,667 | 10 full, 16 skimmed, ~50 measured mechanically |
| 11 | docs, spec, readme | 36,043 | ~15,000 read; ~21,000 measured |
| 12 | Cargo, CI, scripts, docker, examples | ~5,500 | full |

**Honest downgrades:** region 4 judged 13 manager modules by scanning every `run_output`/argv call
site rather than reading the bodies — enough to answer "is this argv or not", nothing more.
Region 10 read 10 of 76 test files in full. Region 11 did not read `history.md` (6,473),
`decisions.md` (6,094), `why.md` (3,519) or `target-state.md` (2,239) line by line.

Every finding below marked **[verified]** was re-checked by hand against the source after the
sweep. Unmarked findings rest on a region reviewer's citation.

---

## §1 — What I committed to before reading any implementation

Laddered want: *declarative package management across every manager* → *rebuild a machine, or
bring a second one into line, without a day of remembering* → *the Nix promise without Nix's
model*. Minimum that satisfies it: a list of `backend:name`; per manager, list/install/remove; a
set difference; a record of what the tool put there so it never deletes what it didn't install.

I sketched: three sets (`desired`, `present`, `owned`), two differences, backends as **data rows**
not Rust files, one guard at the point a removal becomes argv, parallelism only in the query
phase, ~5–8k lines. I refused, for v1: a TUI, a REPL, an embedded Lua interpreter, a second
scripting engine, a templating engine, SMTP, desktop notifications, a firewall manager, a service
manager, filesystem snapshots, SSH fleet management, `bisect`, SBOM, secrets, and a scheduler that
writes systemd units.

**Where the sketch was wrong, before anything else:**

- **The guard.** I said one choke point where a removal becomes argv. That is wrong and the repo
  is right: `max_removals` is a ceiling over the *whole plan*, and a per-argv check cannot see a
  plan. `guard.rs`'s `also_removing` exists so packages and extras count against one ceiling.
  That is a property my design could not express. Conceded completely.
- **Backends as data.** Right, and the repo proved it itself on 2026-08-04, one day before this
  review — `b84dff1` turned eight formulaic backends into eight rows, deleting 1,900 lines with
  byte-identical argv. But I underestimated how much of this product *is not package management*:
  `github`/`web`/`appimage` run no CLI, `link` writes symlinks, `nix` removes by profile index.
  About half the 22 hand-written modules are honestly earned.
- **Model reachability.** I expected most of `src/model/` to be orphaned niche code. 23 of 30
  files are reachable during a `sync`. The region is wired; that was a lazy guess.

---

## Owner ruling, 2026-08-05: **everything is the product**

Asked whether `firewall:`, `setting:`, `storage:`, `service:` and `schedule:` are the product or
scope the grammar made cheap to admit, the owner ruled: **everything is the product.** LiNix
converges declared machine state; packages are one kind of declared object, not the subject.

This **withdraws** the want-lens finding that these are "six other products wearing a package
manager's clothes." That framing is dead and does not appear below.

It **promotes** the craft findings underneath it from peripheral to core, and **creates F-0**.
Findings keep their original numbers; the ranking is restated after F-0.

## The framing finding, restated after the ruling: one product, six engines

Region 6 traced `linix sync` end to end: **21 top-level stages, ~50 sub-stages.** Of the 7,916
lines in the sync path, **~640 are `desired`, `present`, `owned` and two set differences** —
`planner.rs:413` (present), `planner.rs:522` (owned), `resolver.rs:413` (desired),
`planner.rs:503-578` and `:646` (the differences). Four of the ~50 stages.

The rest is two things, and they are not the same thing:

**Safety, which is earned.** `guard.rs` (671 lines, 605 of tests), the WAL, the prior-state probe
that stops rollback uninstalling software the user already had, the snapshot/health-check/restore
loop. This is the best-argued code in the repository and I would not cut a line of it.

**The other half of the product, converged six separate times.** `firewall:`, `service:`,
`setting:`, `storage:`/`btrfs:`, `schedule:`, `dotfiles:` — 2,350 non-test lines in
`src/backends/` plus 1,667 in `apply/`. Per the ruling these belong. **What the ruling does not
excuse is that they are six copies of one converge loop wearing a trait named `Installable`.**
`ZfsInstallable::install` (`storage.rs:148-171`) reads existence,
creates if absent, then applies `@quota`/`@mount`. `service.rs:380-398` is enable-then-start.
`setting.rs:342` is read-before-write. `btrfs.rs:398` is create-plus-fstab. Four converge loops,
no shared machinery, under a verb named "install". And `apply/extras.rs` then rebuilds the whole
declared→ledger→probe→place/undo cycle a second time, with its own guard and its own ledger,
because these nouns need drift detection too.

And `verbs/sync.rs:258-264` records the bill in the code's own words: the phase list drifted into
two copies and *"every statement kind added since was missed by one of them: extras, then
`exec:`, then `dotfiles:`, then `firewall:` — four times."* Four misses, in the product's centre.

**The name is the mechanism.** If the product is *converge declared objects to a stated
condition*, the central abstraction is `Converge` and a package is one kind of object. Calling it
`Installable` is why the six loops share nothing — there was no shared noun to hang an engine on,
so each noun grew its own. That is not a taste complaint; it is the causal chain by which four
hand-written converge loops exist.

Two structural consequences, both now core rather than peripheral:

- **Half the product lives in a field called `extras`.** `resolve.rs:29` —
  `Vec<(Statement, Origin)>`, seven `filter_map` accessors, and `has_non_package_work` as a
  hand-written `||` chain the compiler never checks. Already forgotten a fourth time:
  `verbs/sync.rs:143` documents that `repo:` is not covered and works around it. A
  `Statement::phase() -> Phase` makes that an exhaustive match.
- **The resource registry is built five times.** `firewall_adapters.toml`,
  `init_providers.toml`, `setting_stores.toml`, `snapshot_builtins.toml`,
  `prereq_builtins.toml` — same shape (detect + optional `os` + argv templates with
  `{placeholder}`), five structs, five loaders, five merge functions. `applies_to_this_os` is
  byte-identical in four files; `is_usable() -> Option<&'static str>` is written seven times.
  Meanwhile `firewall.rs:3-7` cites *"Rows, not Rust — K17"* for firewalls, init systems and
  settings stores, while packages remain 2,246 lines of Rust struct literals.

---

## F-0 · The write-ahead log covers packages only — `rewrite` **[verified]**

**Created by the 2026-08-05 ruling. This is the top finding on the accuracy axis.**

`readme.md:738` is the headline safety claim: *"A write-ahead log records every mutation before it
runs. If LiNix is killed mid-transaction, the next run heals it."*

`JournalAction` (`core/journal.rs:22-25`) has exactly two variants:

```rust
pub enum JournalAction {
    Install(PackageSpec),
    Remove { name: String, backend: String },
}
```

Both are packages. **There is no journal representation for a link, a port, a service, a setting,
a schedule, or an exec.** All nine `apply/` modules — `extras`, `dependents`, `dotfiles`,
`firewall`, `schedules`, `execs`, `prereq`, `repositories`, `bootstrap` — contain **zero**
references to `journal`, `Transaction` or a write-ahead log. `heal()` (`sync/mod.rs:693`) replays
interrupted package actions only.

And the ordering seals it: `verbs/sync.rs:213` runs `engine.sync(changes, scope)` — the whole
transaction, including its journal cleanup — and `:217` then runs `apply_non_package_phases`.
**Every resource mutation happens after the transaction has closed.**

When packages were the subject, that sentence in the readme was true. Under the ruling it is false
for the majority of what LiNix converges.

**Steelman, and it is a good one.** Resources are converged *idempotently from declarations* on
every run: a half-applied firewall or a half-placed symlink tree is recomputed and finished by the
next `sync`. Packages need a WAL because an interrupted `apt install` wedges dpkg in a state LiNix
cannot recover from a manifest. The asymmetry is principled — for `link:`, `setting:`, `service:`,
`schedule:` and `firewall:`, which are all read-then-write converges.

**It fails for exactly three phases, and they are the three that mutate irreversibly:**

- **`apply/execs.rs:92`** — a half-run `exec:` script is not idempotent unless the user made it
  so, and there is no declaration to recompute from. `:192` runs an arbitrary `@undo=` shell
  command with no guard and no record.
- **`apply/dotfiles.rs:191`** — `remove_file` on an existing destination. Killed between the
  remove and the write, the user's original file is gone and nothing recorded that it existed.
- **`apply/extras.rs:119`** — half an undo of undeclared extras.

**The change.** Widen `JournalAction` to the objects the product actually converges — one variant
per `Phase` (see the `Statement::phase()` note above), carrying enough to revert or replay — and
move `apply_non_package_phases` *inside* the transaction window rather than after it. If that is
too large a first step, the minimum honest version is: journal the three irreversible phases, and
amend `readme.md:738` to say what the WAL actually covers until the rest lands.

**The cost.** Moving the phases inside the transaction means `SyncChanges` carries resource
actions, which touches the plan type and `SavedPlan`. Journaling the three alone is contained —
`execs.rs`, `dotfiles.rs`, `extras.rs` plus three enum variants.

---

## Findings, ranked by wrongness × cost of leaving

**Re-ranked after the ruling**, on the owner's three stated axes (speed, accuracy, code quality —
all vital, none tradeable):

| Axis | Rank | Finding |
|---|---|---|
| Accuracy | 1 | **F-0** — WAL covers packages only; execs/dotfiles/extras mutate irreversibly outside it |
| Accuracy | 2 | **F-3** — `activate`/`deactivate` plan removals across every backend |
| Accuracy | 3 | **F-4** — `apply` writes no journal, so `heal` cannot recover it |
| Quality | 1 | **The `Converge` engine** — rename `Installable`, one engine, `Phase` enum instead of `extras` (framing section above) |
| Quality | 2 | **F-5** — two backend paths; nine false exemptions; `dnf`/`pacman` lose `--` hardening |
| Quality | 3 | **F-2** — gates scoped to artifacts, not properties |
| Speed | 1 | **F-1** — DAG edges split the batch the code's own comment measures at 10× |

F-1 is unchanged in substance and drops only because a half-run `exec:` with no journal loses
data, while a slow `apt install` loses time.

### F-1 · The dependency DAG pays N subprocesses per run to make installs ten times slower — `rewrite` **[verified]**

> **Actioned 2026-08-06 — `Y9` in `decisions.md`, rule in II.7 + II.19, reason in V.115a.**
> Accurate, and understated on one axis. The edges are gone because the *machinery that
> manufactured them* is gone: `direct_dependencies` and `expand_transitive_dependencies` are
> deleted, and `build_execution_graph` now wires only the `@requires` a user wrote. What this
> review filed as a speed finding was also a data one — every discovered dependency became an
> install node, and `sync/mod.rs:632` writes one `state.add` per install node, so a declared
> package took ownership of its dependencies in `registry.json` with `source: None`. II.7 then
> points at them, and `direct_dependencies` dropped a spec's entry on any error, so a single
> failed `apt-cache depends` moved the whole set into drift at once.
>
> **Two parts of the proposed change were not made, deliberately.** The `StableDiGraph` stays: a
> `@requires` edge is a user's declaration, `Y1` binds it explicitly, and the graph is what
> carries cycle detection, `unreachable_from` and the per-node rollback history. And the 13
> `MetadataProvider` stubs stay — the trait has two live consumers this review did not name
> (`insight.rs:731` for `linix why`'s reverse dependencies, `verbs/packages.rs:965` for the
> `Dependencies:` line), so deleting the trait would delete a feature. Reporting dependencies
> was never the bug; planning from them was.

This is the strongest finding in the review and nothing in eight grade rounds has named it.

`transaction.rs:536-539` carries the measurement, in the code, in a doc comment:

> *Measured on Ubuntu, six declared packages produced six separate `apt install` processes and
> 12,465 ms; `apt install <8 packages>` as one command took 3,161 ms. Eight packages one at a
> time took 31,901 ms — superlinear, because each invocation re-reads the package cache,
> re-takes the dpkg lock and re-resolves a dependency graph the batch resolves once.*

Ten times. Now read the sentence four lines above it, describing what may go in one batch:

> *Every node here is ready at the same moment, goes to the same manager, and is the same kind of
> change, **with no edge between any two of them**.*

And read where edges come from. `planner.rs:820-851` adds an edge for each `spec.requires`, and
for each *native* dependency — discovered by `direct_dependencies` (`planner.rs:784`), which
spawns **one `get_dependencies` subprocess per declared spec**: `apt-cache depends`, `dnf
repoquery`, and so on across 20+ registered `MetadataProvider`s. An edge is wired whenever the
dependency is also in this run's install set (`install_map.get(&dep_key)`).

So: you declare `apt:nginx` and `apt:libfoo`, nginx depends on libfoo, both are in the plan, an
edge is wired, and `batches()` (`transaction.rs:477`) puts them in two groups. **One
`apt install nginx libfoo` becomes two sequential `apt install` invocations**, and the repo's own
test pins it — `transaction.rs:1358-1362`:

```rust
assert_eq!(h.counters[0].calls.load(Ordering::SeqCst), 2,
    "a required package and its dependent cannot go on one command line");
```

A green test asserting that LiNix will take the slow path. And `rebuild --backend apt` — which
takes every one of that backend's packages down and puts them all back up together — is precisely
the command that maximises the number of such edges.

**The edges buy nothing.** `planner.rs`'s own recursion-guard comment says it: *"Every real
package manager resolves and installs the full transitive closure itself at install time, so
LiNix re-deriving it is redundant."* It then re-derives one level of it anyway. And the graph
cannot even produce cross-manager parallelism: `run_exclusive` (`executor.rs:1231`) already takes
a per-backend tokio `Mutex` **and** a cross-process `flock`, and `generic.rs:712` routes every
install through it. Two concurrent `apt` commands are structurally impossible regardless of what
the scheduler wants.

**Steelman, and I tried three.** (a) *Cross-backend ordering matters* — a crate needs a system
compiler. True, and `rebuild` relies on it. But that is a **sort key on backends**, four lines,
not a per-package graph. (b) *The plan must be a graph for preview and rollback.* No —
`SavedPlan` already flattens it and drops the edges entirely (`saved_plan.rs:55-74`), so the
graph is already not the wire format. (c) *`requires` is a user-facing feature.* It is, and
ordering two co-declared packages is exactly what one `apt install a b` does correctly on its
own. None of the three survives.

**The change.** Delete `direct_dependencies` and the native-dep edge loop. Group the ready set by
`(backend, is_install)` into a `HashMap`, `join_all` one command per group, keep the retry loop,
`falsify_transience`, and the prior-state probe. Keep `requires` as a *sort* within a backend's
batch, not as an edge that splits it. First commit: delete `planner.rs:784-851` and the 13
`MetadataProvider` stubs that return `Ok(vec![])`.

**The cost, honestly.** `GraphAction`-in-a-`StableDiGraph` is the wire format for "a list of
packages" across six files — `ui/preview.rs:125,231`, `saved_plan.rs:87`, `verbs/plan.rs:345,374`,
`sync/mod.rs:329,394,634,846`, `verbs/sync.rs:475`. Three of those (`verbs/plan.rs:345`,
`verbs/sync.rs:475,484`) already build the graph with `add_node` only and no edges at all, purely
to satisfy an API that demands a graph. This is a real refactor. It also removes ~300 lines of
scheduler, a `petgraph` dependency, N subprocesses per sync, and makes the program faster.

### F-2 · Every gate is drawn around the artifact that was reviewed, never around the property — `rewrite` **[verified]**

Eight grade rounds have independently named "a check that cannot fail" as this repo's signature
defect. Rounds 2, 7 and 8 name it in nearly identical words. **Nobody has diagnosed why it keeps
coming back, and re-reporting it a ninth time would be worthless.** Here is the mechanism.

The repo builds excellent gates. `removal_guard_enumeration_tests.rs:153` scans all of `src/` and
fails the build when a removal call appears without a named guard — and self-tests the instrument
at `:211`. `argv_drift_tests.rs:222` walks every subcommand LiNix invokes against the real
manager's `--help`. `help_map_tests.rs` gates `args.rs`'s command map against `--help` in both
directions, and its header cites the `undo` disease as the reason it exists.

**Each one is scoped to the file that was under review when it was written.** The property escapes
through the next copy of the fact:

- `help_map_tests` gates `args.rs`. There is **no top-level `status`, `doctor` or `undo` verb**
  — the three `Status` variants at `args.rs:897/917/972` are `hooks status`, `git status`,
  `service status`. Outside the gate, invoking commands that do not exist:
  - `app/fleet.rs:111` — `ssh <host> "linix status --json"`. **`linix fleet` cannot return
    "in sync" for any correctly-installed host.**
  - `scripts/install.sh:84` — `"$LINIX" doctor || true`, and `:96` signs off *"Try `linix status`
    or `linix doctor`"*. `install.ps1:54,61` the same. **The first thing a new user runs, and the
    health check that vouches for the new binary, both address deleted commands.**
  - `verbs/cleanup.rs:348` — after `purge-undeclared`, the most destructive command in the
    program, prints `Undo with 'linix undo <id>'.`
  - `verbs/check.rs:967`, `verbs/setup.rs:354,640`, `app/apply/dotfiles.rs:70`, `readme.md:610`.
  - `readme.md:670-673` lists `unmanaged`, `absent`, `conflicts`, `doctor` as commands — **four of
    eight rows in one table** — thirty lines after correctly explaining they were folded into
    `check`.
- `removal_guard_enumeration_tests` covers nine removal paths in `src/`. Outside it:
  `apply/firewall.rs:127` closes undeclared ports, `apply/execs.rs:192` runs an arbitrary `@undo=`
  shell command, `apply/dotfiles.rs:191` `remove_file`s existing destinations.
- `scripts/harness-logic-test.sh:250-273` cross-checks every subcommand a harness invokes against
  `--help` — added, per `ci.yml:86-88`, because *"six of them had been folded into `check
  <section>`"*. At `:551-555` it **explicitly exempts the install scripts**, with the comment:
  *"`install.*` is what a user pipes from the web"*. That is the argument for including it,
  written down as the reason for excluding it.
- `scripts/decision-count.sh` gates the register's own counts. It prints `unrecognised 2` and
  then exits 0, because `OTHER` is never added to `BAD` (`:47`, `:157`). `SPEC.md:16` and
  `decisions.md:79` both advertise a total of 164 against 166 entries.
- `tests/backend_is_data_not_code_tests.rs` requires each hand-written backend to state what the
  generic machinery cannot express. **Nine of the reasons are refuted by the code beside them**
  (see F-5).

**The change.** One property-scoped gate, roughly forty lines: grep the entire tree — `src/`
string literals, `scripts/`, `docker/`, `readme.md`, `docs/` — for `` `linix <word>` `` and
`"linix <word>"`, and assert each word is a live subcommand of the clap surface. That single test
catches every bullet above. Then fix `decision-count.sh:157` to fail on `unrecognised`, and delete
the `install.*` exemption in `harness-logic-test.sh:553`.

**The cost:** one test file, one afternoon, and a handful of string edits. This is the cheapest
high-value change in the review.

### F-3 · `activate` plans removals across every backend on the box — `rewrite` **[verified]**

`ChangePlanner::with_enabled` restricts which backends a *removal* may be scheduled from —
`backend_enabled` is consulted at `planner.rs:375`, inside `declined`, and *"an empty scope means
every backend"* (`planner.rs:353`).

`with_enabled` is called at exactly two sites: `verbs/sync.rs:115` and `verbs/plan.rs:49`.

`app/profile.rs:438-465` — `sync_now`, reached by `linix activate`, `linix deactivate` and
`profile save` — builds its own `SyncEngine`, its own `StateResolver`, and
`ChangePlanner::new(...).plan(&desired, None)` **with no `with_enabled` call**. So `linix sync`
confines removals to the managers your `priority` file names, and `linix activate Work` does not.

It also skips `enforce_policy`, `bootstrap().offer`, `prereqs().offer`, `repositories().apply`,
dotfiles, firewall, schedules, execs and the extras teardown. It is a second, stripped reconcile
loop, and it is the one two user-facing verbs go through.

**Steelman:** activating a profile is a narrower operation than a full sync, so a narrower path is
defensible. That argument would hold if the narrowing were *toward* safety. It narrows away from
it — the one thing it drops relative to `sync` is a restriction on what may be **deleted**.

**The change.** `sync_now` calls `verbs::sync::reconcile`. There is one reconcile or there is
not one; right now there is not, and the second one is unguarded on the dangerous axis.

### F-4 · `apply` is the one change path `heal` cannot recover — `rewrite` **[verified]**

`plan`/`apply` is sold in the readme as the Terraform story: freeze what `sync` would do, review
it, apply exactly that. `verbs/plan.rs` contains **zero** references to `Transaction`, `journal`,
or `execute_with_telemetry`. `handle_apply` walks `installs` serially calling `inst.install`
(`plan.rs:525`), then `removals` serially calling `inst.remove` (`plan.rs:558`).

No write-ahead log, no transaction, no auto-rollback, no snapshot, no health check, no hooks. So
`linix heal` — which reads the journal (`verbs/setup.rs:489`) — **cannot recover an interrupted
`linix apply`, because `apply` never wrote one.** The safety story that justifies the whole
`core/transaction.rs` subsystem has a hole exactly where the feature named after review and
deliberation lives.

It also cost 200 lines of hand-rolled guard scaffolding (`plan.rs:296-333`, `:490-509`) that
`sync` gets for free from being one call. And it loses ordering: `saved_plan_to_changes`
(`plan.rs:340-359`) adds nodes and no edges, so a plan whose `requires` ordering mattered applies
in `Vec` order. (If F-1 lands, that last part dissolves — but the WAL gap does not.)

**Steelman:** applying a frozen plan is meant to be simpler than planning one, and re-entering
`SyncEngine::sync` risks re-planning and defeating the point. Real — but `SavedPlan` →
`SyncChanges` already exists at `plan.rs:340` and is currently used only to feed the TUI.
Rebuild `SyncChanges` from the saved plan and hand it to `SyncEngine::sync`; the freeze survives
and the WAL comes with it.

### F-5 · Two paths for everything, and the second path is where the safety falls off — `delete` nine modules

A backend is a `ManagerConfig` row (40 backends, 2,246 lines of table + 1,371 of shared engine
= **56 lines each**) or a hand-written module (22 backends, 8,861 lines = **403 lines each**).

Nine of the 22 exemptions are refuted by the code beside them. Three checked closely:

- **`dnf.rs`** claims it *"reads its own history to distinguish user-installed from dependency,
  which is a second command whose output changes what the first one means."* It runs
  `dnf repoquery --userinstalled` and parses it with **the same function** as `fetch_installed`
  (`dnf.rs:151` and `:164`). That is verbatim `ManualListing::Command { format: SameAsInstalled }`
  (`generic.rs:151-158`). Apt's row does the strictly harder version as data.
- **`xbps.rs`** claims two binaries plus a third for the manual/automatic split. Those are three
  existing fields — `binary`, `remove_binary`, `list_binary` — and `generic.rs:274-277`'s doc
  comment names OpenBSD `pkg_add`/`pkg_delete` as exactly this case. `register_pkg_add_openbsd`
  already ships that shape.
- **`pacman.rs`** claims the removal guard needs pacman's essential data. `grep -n essential
  src/backends/pacman.rs` returns nothing; there is no `essential()` impl. Meanwhile
  `register_aur_helper` (`registry.rs:317-389`) registers `yay`/`paru` **through the data path
  with pacman's character-identical argv** — `["-S","--noconfirm","--needed"]` at `registry.rs:334`
  against `pacman.rs:115`. The proof that pacman is a row sits 200 lines below the claim that it
  cannot be.

**And the split has already cost behaviour.** `core/argv.rs:66-76` lists `dnf`, `pacman`,
`xbps-install` and `xbps-remove` as `--`-terminating. Every data backend gets the terminator
through `argv::push_names` (`generic.rs:704,776`). `dnf.rs` and `pacman.rs` call `push_names`
**zero times** and build argv by hand — so the argv table records `Runs("apt install -y -- jq")`
beside `Runs("dnf install -y jq")` and `Runs("pacman -S --noconfirm --needed jq")`. **The two
root-privileged system managers lose the injection hardening that every user-scope backend gets,
because they are on the other path.**

The same two-paths shape recurs everywhere and always with the second path losing something:
`src/parsers/` (40 managers) vs eight backends parsing inline (`brew.rs:118` is
`ecosystem::ws_name_version` verbatim; `flatpak.rs:188` is `common::parse_simple_list` verbatim);
`dry_run.rs`'s atomic vs `executor.rs:848`'s `dry_run` field, in the very module written to stop
that divergence; `hooks.rs` (Lua) vs `events.rs` (scripts), which share a ledger and the event
name `after_sync` and once fired one config key twice.

**The change.** Move the 40 rows into `onboarder.rs`'s TOML format — which already exists, is
already the format users are told to write, and is currently the one format nothing shipped uses —
then convert the nine false-exemption modules to rows. ~2,834 lines out, ~504 in. `capability.rs`
stops being a second copy of six columns of the same table.

### F-6 · Dead subsystems, and the four dependencies they hold hostage — `delete`

- **`app/scheduler/notify.rs` — 183 lines, zero callers.** `NotificationManager` is constructed
  on **every run**, including `linix path` (`context.rs:115`), stored as a field
  (`context.rs:38`), and `notify()` is never called anywhere in `src/` or `tests/`. **[verified]**
  It is the sole use site of `lettre` (a full async SMTP client with rustls) and `notify-rust`
  (D-Bus / WinRT). There is no config key to enable it either: `examples/preferences.toml` is 207
  lines that claim to document every key and contains no `smtp`, `notif` or `email`. The want is
  already served better by `events.rs` — `on_drift` plus a shell script is how you talk to Slack.
- **`mlua` — 4 use sites, and it is the *fallback* branch.** `hooks.rs:143-151` dispatches `#!`
  → subprocess, `#rhai` → in-process, **everything else → Lua**. **[verified]** So a vendored
  Lua 5.4 — 23,252 lines of C, compiled on every clean build and **nine times per CI push** (3
  matrix targets + 6 container images with no shared cargo cache) — is load-bearing by accident of
  fall-through. `#rhai` already provides in-process scripting, and `rhai` is independently
  justified by `model/vars_embedded.rs`. Delete the Lua arm; `mlua` leaves the manifest.
- **`app/diagnostics.rs:140-247` — `handle_failure`/`remediate`, zero callers.** Dead code that
  prompts with dialoguer and then **installs packages** (`:214-237`), writing them into the state
  registry with `source: "diagnostics"`. Dead code that installs software is the worst kind: it
  looks maintained and is never exercised.
- **`app/fleet.rs` — 265 lines, broken since `status` was deleted** (F-2).
- **`utils/progress.rs` — 132 lines, two traits, four types, one dependency, one config key, one
  CLI flag and four struct fields, to render one spinner** around one `await` at
  `sync/mod.rs:611-615`. `start()`, `println()` and four of `ProgressHandle`'s five methods have
  zero callers. The one live handle ends in `finish_and_clear()`, so it leaves no output.
- **`app/repl.rs` — its own docstring concedes the case** (`repl.rs:3`: *"Every question this
  answers is one `linix eval | jq` can answer too"*).
- **`app/ui/` — 641 lines of ratatui.** The preview's headline feature is `cycle_backend`
  (`preview.rs:218`) and **both call sites pass `HashMap::new()` for alternatives**
  (`verbs/sync.rs:197`, `verbs/plan.rs:475`) — the `b` key is permanently wired to nothing.
  `history.rs:223-255` drops out of the alternate screen to run `sh -c`: a terminal emulator
  inside a package manager.
- **`StateRegistry::ghosts` — written on every removal, read by nothing.** **[verified]** An
  unbounded map in `registry.json` growing one entry per package ever removed, whose only
  constructor hardcodes two of its four fields empty (`state.rs:301-302`).
- **`ManagerConfig::flag_map` — declared once, assigned at 25 sites, read at zero.** **[verified]**
  Also absent from `CustomBackendDef`, so no user could set it if anything read it.
- **`utils/`: 11 of 33 public items have zero callers**, surviving because `lib.rs:13` makes the
  module public and `pub` silences the dead-code pass. Includes `refresh_path` (`mod.rs:48`),
  whose doc asserts *"Backends that install a toolchain (mise, cargo) must call this"* — nothing
  does, and there is no `cargo.rs`. Either the bug it describes is live and unguarded, or the
  comment is fiction.

Net: **~2,300 lines and four dependencies** (`mlua`, `lettre`, `notify-rust`, `ratatui`) with no
user-visible loss except that `linix fleet` stops lying.

### F-7 · `.tar.zst` is selected as installable and cannot be opened — `delete` the fifth copy **[verified]**

`format.rs:90` lists `.tar.zst` and `.txz` under `Format::Tarball`. `utils/archive.rs:16-34`
handles gz, xz, bz2, zip — and falls through to `fs::copy` for anything else. There is no `zstd`
in `Cargo.toml`. So a `.tar.zst` release asset is chosen, downloaded, copied verbatim into the
destination, the walkdir finds no executable, and **the install reports success having deployed
nothing.** `format.rs:332` is a passing test pinning the selection.

That is what a five-way split of one fact produces. Archive-extension knowledge exists in
`utils/archive.rs:16` (the extractor), `utils/archive.rs:74` (`is_archive`, dead),
`utils/file.rs:138` (`ARCHIVE_SUFFIXES`, 15 entries incl. `.zst`), `format.rs:90` (`Format`), and
`backends/web.rs:249` (an inline list matched with `.contains()` rather than `ends_with`). List 4
grew a format list 1 never learned about. Delete the dead `utils::is_archive` — the same question
`Format::is_archive` answers live — and move `strip_archive_suffixes`/`ARCHIVE_SUFFIXES` next to
`Format`, where the vocabulary belongs. Then either add `zstd` or remove it from the table.

### F-8 · The review apparatus consumes the product it reviews — `rewrite`

**36,043 lines of documentation against 93,909 lines of source (38%), of which `readme.md` (956)
is the only user-facing file.** Seven dated `GRADE-*.md` in nine days, plus AUDIT, READINESS,
FINDINGS, DIRECTIONS, SESSION, INEFFICIENCIES, PRODUCTION-READINESS-REVIEW, and seven files under
`spec/`.

The rounds are individually excellent — every one measures, every one reports what it could not
test, several correct themselves mid-document. **They are not landing:**

- `cargo fmt --check` went **26 diffs → 0 → 0 → 0 → 12 → 60** across rounds. Closed at the
  mechanism (`E4`: "fmt is HARD in release-check.ps1:68, matching CI"), and the mechanism was
  never run.
- `G-4` (gate-parity compares basenames, not gates) was closed on 07-29 **with a mutation test the
  reviewer watched go red** — and reopened on 07-31, same ID, same defect.
- "A check that cannot fail" appears in **all seven rounds**.
- The meta-finding *"the correct behaviour already exists at a different site"* is named as **the**
  headline by rounds 2, 7 and 8 in nearly identical words.
- The grade does not converge — C+ → B− → B → B− → B− → B− → C → B+ — because rounds 7 and 8 each
  introduced a **new rubric**. Three rubrics, one product.
- `decisions.md` is **160 ANSWERED of 166**; the single OPEN item is what licence to use. Seven
  questions asked on 2026-08-05 read *"ANSWERED — ruled 2026-08-05, and built the same day"*. The
  register documents decisions; it does not gate them.
- `docs/spec/proposals/` contains six files, **all titled "Proposed:" and all six built**.
- Seven spec files for one specification: `why.md` (3,519) is **1.6× the spec it annotates** and
  `history.md` (6,473) is **2.9×**. `principles.md` + `target-state.md` is the spec; the other
  18,193 lines are a changelog, a bug tracker and a design diary filed under `spec/`.
- **~20 live ID namespaces, and two collide outright:** `BUILDER.md:231` defines work order `W9`;
  `decisions.md`'s `W9` is a register entry ANSWERED 2026-07-24. `CLAUDE.md` says stop and ask for
  anything with a `W*` ID — so a builder handed "W9" cannot tell from the ID whether to build it
  or stop. `PRODUCTION-READINESS-REVIEW.md:3-7` renamed its own IDs specifically to avoid this and
  explained why; `BUILDER.md` did it anyway, 43 times.

**The change.** `SPEC.md:126-130` already has the right rule and applies it to rulings only:
*"the ruling ships in the same commit — rewritten into `decisions.md`, and into `target-state.md`
plus `why.md`."* Apply it to **findings**. A round's output is a diff to `target-state.md` and a
test, not a new dated file. Archive the seven GRADE files, delete `AUDIT-v6.org` (self-labelled
*SUPERSEDED — DO NOT IMPLEMENT FROM THIS DOCUMENT*), `BEHAVIOR.org`, `backend-expansion-plan.md`
("Status: in progress" for finished work, naming a `migrate` command that does not exist) and
`PRODUCTION-READINESS-REVIEW.md`. Cut `readme.md`'s five verb tables — the readme itself says at
`:648-650` that `--help` cannot go stale the way a README can, and then goes stale in the next
table.

### F-9 · Smaller, but each is a family, not an instance

- **`examples/groups/` — five files naming deleted features.** `bloatware.txt:3` says *"removed
  when running `linix sync --remove-bloatware`"*; `remove_bloatware` is in `target-state.md:2035`'s
  deleted-config list and `bloatware.txt` itself is in the deleted-*files* list at `:2040`. A
  straight NO-LEGACY violation. `examples/preferences.toml` — the one example with an
  `include_str!` test behind it (`verbs/setup.rs:969`) — is the one that did not rot. That is the
  entire argument for the test.
- **Install scripts silently downgrade the supply chain.** `install.sh:52` runs `cargo install
  --locked 2>/dev/null` and on *any* non-zero exit re-runs **without `--locked`**, discarding
  `Cargo.lock` and resolving 452 dependencies fresh, with the reason hidden. `install.ps1:34-35`
  the same. The comment calls this "fall back if the lock is unavailable"; the code falls back on
  a network blip.
- **No `aarch64-apple-darwin` in the release matrix** (`ci.yml:29-34`) — every Mac sold since 2020.
  Currently masked only because the installer builds from source, which means the three published
  binaries are installed by nothing.
- **Five spellings of "split `backend:name`".** `CLAUDE.md` says there is one parser. There are
  two, and the second disagrees: `parser.rs:234` does `name_part.split('@').next()`, which has
  never heard of the Q23 rule that a leading `@` is part of an npm scoped name
  (`statement.rs:1076-1094`). `split_removal_target("npm:@angular/cli", …)` returns
  `(Some("npm"), "")`. Seven call sites carry it into `rebuild`, `cleanup` ×3, `packages`,
  `upgrade`. Q23 was fixed on the read side; its sibling on the **remove** side is live.
- **`pkgsrc.rs` is `bsd.rs`, byte-identical but for one comment**, and `bsd.rs:6-7` *names* the
  duplication and rationalises it — *"kept in its own module because the backend labels differ"* —
  directly above `parse_with_backend`, which **takes the backend as a parameter**. The duplicate
  was found, and a comment was written instead of a deletion.
- **Vars is 1,214 lines across three files** (`vars.rs`, `vars_provider.rs`, `vars_embedded.rs`)
  for `role = desktop` and one `when` block, and `vars_embedded.rs:6-9` states the script's *"only
  inputs are the detected facts below"* while `register_stdlib` is called unconditionally at `:54`
  and registers `sh`, `http_get`, `read_file` and `env`. The module header asserts the opposite of
  the module. Two Rhai engines with opposite security postures ship in one binary.
- **`verbs/` is not reachable from any of the 76 test binaries.** `main.rs:10` declares
  `mod verbs;` — private to the binary, not in `lib.rs`. So ~5,500 lines of real logic (the
  lock/unlock ledger, `check_health`, the failure-attribution classifier, `reconcile` itself) can
  only be tested through `#[cfg(test)]` blocks or by spawning the binary. The module boundary is
  in the wrong place.

---

## What I could not beat

Four things I tried to design better and failed at, and one I got outright wrong.

**The removal guard beat my design.** I said one choke point where a removal becomes argv. That
cannot express `max_removals`, which is a ceiling over a whole plan, and it cannot express
`also_removing` counting packages and extras against one budget. `guard.rs` is right and I was
wrong. My *only* remaining complaint is that it is nine calls rather than a type — `SyncChanges`
is already the plan, and a `GuardedPlan` newtype that only `guard::enforce` can mint and only
`Transaction::with_config` accepts would make the promise a compile error. The 605 lines of guard
tests, including one that enumerates every `GuardScope`, exist precisely because the discipline is
not structural. But that is a refinement of a correct design, not a replacement for a wrong one.

**`core/argv.rs` is the shape the whole repo should copy.** 373 lines of table where **every row
carries the tool's own printed output as evidence**, plus a ratchet (`:510`) that lets unmeasured
rows only decrease. I cannot improve on it and most of the repo's problems are what this file
prevents.

**`text.rs::sanitize` — 102 lines, ~40 call sites, one 6-line function at the `run_output`
boundary.** Its header records that this rule was scattered and six backends forgot it. Inlining
recreates the bug by construction. This is what a shared abstraction is supposed to look like, and
it is the correct answer to the `utils/` junk-drawer question I went in expecting to win.

**The test suite's *content* is better than what I would have written.** Not its shape — 76 binaries
named after review rounds, 13 copies of `struct Fixture`, 18 of `fn run()`, a `mock_providers`
target with zero `#[test]`s that links a 716 KB binary on every run, and a filename convention
whose round number is consistently **one behind** the header's. But the discipline inside them:
controls that prove the test could have failed; instruments self-tested before use
(`grade3_resource_idempotency_tests.rs:186` writes a file and asserts the mtime actually moved,
*before* relying on mtime comparison); coverage gates whose exemption lists are themselves
validated (`dry_run_every_verb_tests.rs:594`, written after `undo` sat in two exemption lists
post-rename); enumeration from the source's own tables rather than hand-copied lists. I would not
have thought to self-test the instrument. That is a higher standard than mine.

**And the one thing that would make me cheerful if I were you:** the repo proved my central
prediction before I made it. `b84dff1`, dated 2026-08-04, converted eight formulaic backends to
data rows, deleted 1,900 lines, kept argv byte-identical, and left behind a ratchet named
`backend_is_data_not_code_tests.rs`. The mechanism is right and it works. F-5 is not "you should
have done this" — it is "you did this, and stopped nine modules short, and the ratchet's teeth are
false on those nine."

---

## The one gap I would close before any of the above

**Nothing in this repository tests that a machine converges.**

`tests/e2e_tests.rs:19` writes `brew:neovim`, runs resolver → planner → `SyncEngine::sync`, and
asserts `is_managed("brew","neovim")`. One package, install-only, mock executor, no second run.
**No test deletes a line and asserts the package leaves.** No test syncs twice and asserts the
second run is empty. `src/app/sync/mod.rs` — 1,102 lines containing the entire apply loop —
contains **zero** `#[cfg(test)]`.

So `install = desired − present` is proved once, `remove = (present ∩ owned) − desired` is proved
nowhere end to end, and the fixed point is proved for dotfiles only. Seventy-six test binaries
guard the loop from every angle — grammar, argv, guard enumeration, dry-run parity, exit codes,
fixture provenance — and not one runs it forward, backward, and forward again.

That test is thirty lines. Write it before F-1, because F-1 changes the execution engine and you
will want something that fails if the change is wrong.

---

## What I need from you

1. ~~**Are `firewall:`, `setting:`, `storage:`, `service:` and `schedule:` the product?**~~
   **Answered 2026-08-05: everything is the product.** Consequences folded in above — the
   want-lens objection is withdrawn, F-0 is created, and the `Converge` engine becomes the top
   structural finding. Note the ruling makes F-5 *larger*, not smaller: packages have a data path
   (40 rows) that the resource half has no equivalent of at all, so every resource backend is
   hand-written by construction.
2. **What is the next feature?** Half of what makes a design wrong is the change it is about to
   face, and that is not in the repo. If the answer involves more statement kinds, the `extras`
   catch-all (`resolve.rs:29`) and the phase list that has already drifted into two copies are
   where the next bug is — and given the ruling, that is now a defect in the product's centre.
3. **Is `linix fleet` used by anyone?** If yes, F-2 is urgent and F-6's "delete it" is wrong. If
   no — and it cannot currently have worked — it is 265 lines of evidence for F-2 and nothing else.

## Suggested order

The one gap below (the converge test) first, because everything else changes behaviour and you
want something that fails if a change is wrong. Then F-3 and F-0's three-phase minimum — both
contained, both accuracy. Then F-2's forty-line gate, which is the cheapest high-value change
here. Then the `Converge` engine, which is the large one and which F-1 and F-5 both become easier
after.
