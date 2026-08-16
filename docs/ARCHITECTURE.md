# Architecture

How Shall is put together, for someone who has just cloned it. [`README.md`](../README.md)
describes what it does for a user; this describes what the code does, in the order the code does
it.

The design record lives in [`docs/SPEC.md`](SPEC.md) and is the authority when the two disagree.
This file is a map, deliberately shorter than the territory.

---

## The idea, in one paragraph

You write files saying what the machine should have. Shall reads them, asks the package managers
already installed what the machine *actually* has, computes the difference, and drives those same
managers to close it. Shall installs nothing itself — it is an orchestrator that speaks apt,
pacman, brew, cargo, npm, systemd, ufw and about thirty others, plus a few kinds of thing that
are not packages at all (symlinks, settings, firewall rules, scripts).

Everything downstream follows from one property: **the file is the truth**. Delete a line and the
package goes away. That is why the removal guard exists, why there is a write-ahead log, and why
so much of the test suite is about not believing your own state file.

## Two directories, and never mix them up

| | what it holds | in git? |
|---|---|---|
| `$SHALL_CONFIG_DIR` (default `~/.config/shall`) | what you declared — modules, profiles, priority, locks, `preferences.toml` | **yes**, it is meant to be a repo |
| `$SHALL_DATA_DIR` | what Shall knows — the state registry, the journal, snapshot metadata | **never** |

The split is load-bearing. Config is portable between machines; data describes *this* machine and
would be actively wrong on another one. A test that forgets to redirect both writes into your real
user state, which is why `Config::sandboxed()` exists and why fixtures use it.

## Life of a `shall sync`

This is the spine. Almost every other verb is a shorter path through the same parts.

```
argv
 └─ src/cli/args.rs          clap types; the enum IS the command surface
 └─ src/main.rs              dispatch, global flags, exit code
     └─ src/verbs/sync.rs    `Reconcile` — what one pass should do
         │
         ├─ 1. READ       src/config/grammar/   one parser for `backend:name`
         │                 src/model/           profiles choose, modules hold
         │
         ├─ 2. OBSERVE    src/app/inventory.rs  ask every relevant manager what is installed
         │                 src/parsers/          turn each manager's stdout into packages
         │
         ├─ 3. RESOLVE    src/app/sync/resolver.rs   bare name → a concrete backend
         │                 src/app/sync/pins.rs       version pins, lockfiles
         │
         ├─ 4. PLAN       src/app/sync/planner.rs     a petgraph DAG of GraphActions,
         │                                            ordered by II.7's phases, cycles rejected
         │
         ├─ 5. GUARD      src/app/sync/guard.rs       may this many removals proceed?
         │
         └─ 6. APPLY      src/core/transaction.rs     JoinSet worker pool over the DAG
                           src/core/journal.rs        write-ahead log, so a kill is recoverable
                           src/backends/              the adapters that run the actual commands
                           src/core/executor.rs       the only place a child process is spawned
```

Three things to notice, because they are the ones people trip over:

* **Step 5 is not optional and not per-command.** Every path that removes calls the guard. A
  guard on one command is a guard on nothing — the bug that motivated it arrived through `prune`,
  which nobody had thought to check.
* **Step 6 writes before it acts.** `record_start` goes to the journal before the manager is
  invoked, so a process killed mid-run leaves evidence rather than a mystery. `shall heal` reads
  it back.
* **Step 2 is the expensive one**, and it is fanned out concurrently. `--timings` prints the
  child-command count and the overlap ratio; a change that accidentally serialises the fan-out
  shows up there and nowhere else.

## The layers

Roughly outside-in. `src/lib.rs` is the crate root and everything below is public, because the
test suite links against the library rather than shelling out to the binary wherever it can.

| directory | what lives there |
|---|---|
| `cli/` | clap types only. No logic — the enum is the surface, and `help_map_tests` checks it against `--help`. |
| `verbs/` | one module per verb family. What a subcommand *does*, kept out of `main.rs` so tests can link to it. |
| `app/` | the running application: `App` is a composition root that owns collaborators and hands out narrow facets (`Inventory`, `Declarations`, `Managers`, the `apply` facets). It has no behaviour of its own, on purpose. |
| `model/` | the declarative model — profiles, modules, vars, scopes, resolution. Mostly pure. |
| `config/` | reading `preferences.toml`, resolving where the config root is, and `grammar/`: the one parser for a declaration line. |
| `core/` | the machinery: executor, transaction engine, journal, locks, snapshots, exit codes, errors, timing. Nothing here knows about a specific package manager. |
| `backends/` | the adapters. Mostly **data** — see below. |
| `parsers/` | readers that turn a manager's stdout into structured packages. Each has a fixture of real bytes behind it. |
| `utils/` | genuinely generic helpers. If it knows what a package is, it does not belong here. |

## Backends are data, not code

Most backends are not code at all. `src/backends/builtin_backends.toml` holds them as rows in the
same table a *user* adds a row to, parsed by the same loader:

```toml
[[backend]]
name = "composer"
install_args = ["global", "require"]
remove_args  = ["global", "remove"]
list_args    = ["global", "show", "--format=json"]
```

This is the single most important thing to know before adding manager support: **start by trying
to write a row.** An adapter mechanism the built-ins bypass is one nobody has tested, so the
built-ins deliberately do not bypass it.

Six shapes genuinely cannot be a row — an install conditional on a read (`snap`'s `@classic`),
argv that is a program rather than a template (`nix`, `go`), a removal that is a filesystem
operation informed by a query (`appimage`, `web`), a probe reaching *into* JSON, per-package
identity that includes the version, and **no per-package command at all** (`nixos`, which renders
the machine's whole system configuration and then runs one `nixos-rebuild`). Those live in Rust
modules beside the table, and `backend_is_data_not_code_tests.rs` holds the exemption list with a
reason for each. Adding to that list is a decision, not a convenience.

## Invariants that will bite you

Each of these is a rule with a scar behind it; `docs/spec/why.md` has the incident for every one.

1. **One parser for `backend:name`.** Anything that splits on `:` and trusts the prefix is a bug.
   Eight places used to do it and six never checked the prefix named a real backend.
2. **Every removal path calls the guard; every install path calls the `[guard]` gate.**
3. **`--yes` never overrides a guard.** `-y` means "don't ask me questions", which every CI job
   passes; it must not also mean "yes, purge the system". `--allow-mass-removal` is the only
   override and cannot be set permanently in config.
4. **A listing nobody can parse reads as an empty machine**, and `sync` answers an empty machine
   by installing everything. That is why every row that lists must also declare how to read.
5. **`App` decides nothing.** If a method reads `config` and asks `registry` a question, it is a
   facet with its own type, not a method on `App`.
6. **Exit codes are a closed set** (`core/exit.rs`): `0` converged, `1` Shall failed, `2`
   differences found, `3` guard refused. `3` is separate on purpose — a script that retries on
   failure must not retry a refusal.

## Concurrency

Tokio multi-threaded runtime. The fan-out in `inventory.rs` and the worker pool in
`transaction.rs` are the two places concurrency actually lives; everywhere else that looks
concurrent is one of those two underneath.

Bounded by `max_parallel`, which defaults to the core count and is the one machine fact you are
allowed to configure by hand — everything else about the host is detected, never declared.
Network fan-out is bounded separately by `max_network` (16, deliberately not the core count: a
socket costs a file descriptor, not a core).

## Testing shape

One test binary, not a hundred: `autotests = false` in `Cargo.toml` and every file listed as a
`mod` in `tests/main.rs`. Cargo would otherwise link each of ~130 files against a 100k-line crate
under `codegen-units = 1`, which once produced a 194 GB `target/` and filled a 944 GB disk
mid-build.

**A new test file does not run until it is a `mod` in `tests/main.rs`.**
`every_test_file_is_in_the_suite` fails when the two disagree, which is the only reason the
arrangement is safe.

File names are sentences describing the property — `a_plan_installs_only_declarations_tests.rs`,
`a_lister_cannot_report_what_was_removed_tests.rs`. Follow it; the convention is what makes 130
files navigable.

See [`DEVELOPMENT.md`](DEVELOPMENT.md) for how to run them, and what the container harness covers
that the Rust suite structurally cannot.
