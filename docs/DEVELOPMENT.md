# Development

Day-to-day mechanics: build it, run it without wrecking your own machine, test it, and know which
of the two harnesses can actually answer the question you are asking.

Read [`ARCHITECTURE.md`](ARCHITECTURE.md) first if you have not — this assumes you know roughly
where things live. [`CONTRIBUTING.md`](../CONTRIBUTING.md) covers the rules; this covers the
commands.

---

## Prerequisites

* **Rust 1.89 or newer.** That is the declared MSRV in `Cargo.toml` and CI enforces it. It is not
  aspirational: before it existed, "does this build on my machine" was answered by whatever
  rustup the author happened to have, and a contributor on a distro-packaged Rust found out by
  watching it fail.
* **Docker**, for `scripts/unix-check.sh` and the container harness. On Windows the daemon
  typically lives inside WSL; the scripts already fall back to `wsl -- docker` and rewrite paths
  to `/mnt/c/...` for you.
* **A C toolchain**, because `mlua` vendors and compiles Lua.

Then, once per clone:

```sh
git config core.hooksPath .githooks
```

That installs `.githooks/pre-commit`, which refuses a commit `cargo fmt -- --check` would reject.
**A clone that has not run this has no hook at all** — it is not automatic. The hook is formatting
only, deliberately: clippy and the suite take minutes, and a pre-commit hook that takes minutes
gets bypassed until it gates nothing. `git commit --no-verify` when you mean it.

## Running it without touching your own machine

This is the first thing to learn, because the program's whole job is installing and removing
software, and the default target is *you*.

```sh
export SHALL_CONFIG_DIR=/tmp/shall-play/config
export SHALL_DATA_DIR=/tmp/shall-play/data
cargo run -- init
cargo run -- check
cargo run -- --dry-run sync
```

Or per-run, `--config-dir DIR --data-dir DIR` (`--data-dir` must be an **absolute** path).

**Pass both, always.** `--config-dir` moves your declarations; `--data-dir` moves what Shall
records about them. With only the first, a fresh sandbox plans against *the real machine's*
managed state — every package it thinks it owns becomes a removal candidate.

`--dry-run` is honest — it plans and prints and executes nothing — but treat it as a second
safety net rather than the first.

### Adopting first, if you do want to point it at a real machine

A fresh config makes every installed package look undeclared, and therefore a removal. Run
`shall adopt` first, then assert against a machine whose state Shall actually knows. Skipping this
is the most common way to get a scary-looking result that is entirely your own fault.

## The verify chain

```sh
cargo build --all-targets
cargo test --no-fail-fast
cargo clippy --all-targets
cargo fmt -- --check
scripts/unix-check.sh
```

Run it in that order. Some notes that are not obvious:

**`--no-fail-fast` is not optional.** Without it cargo stops at the first test *target* that
fails, so one failure in the lib abandons the integration suite and the run reports one defect out
of however many exist.

**`cargo fmt -- --check` is a real gate, not a release-time tidy.** CI rates it fatal on every
push, and it is the one gate a change containing no logic can break: a rename once re-sorted two
import groups and turned the whole board red — main plus every open dependabot PR — on a commit
that touched only names.

**On Windows, four of those five steps see one platform of two.** `scripts/unix-check.sh` is the
fifth and the only one that compiles the 45 `cfg`-gated blocks across 17 source files that a
Windows build cannot see. It runs `cargo check` in a `rust:1-slim` container, because the cheap
alternative genuinely does not exist — `cargo check --target x86_64-unknown-linux-gnu` from a
Windows host dies in `mlua`'s vendored C build for want of `x86_64-linux-gnu-gcc`.

Skipping it is not free: a tree that will not compile on Linux fails every Apple, Linux and MSRV
job *and* every distro integration job, because the container harness builds its binary in-image.
One `#[cfg(unix)]` mistake can therefore take the whole board red and the fault-injection checks
offline at the same time.

`scripts/unix-check.sh --lib` is faster and catches most of it.

## Two more gates, when they apply

Neither belongs in the chain above — most changes cannot fail them — but both are cheap and both
are enforced in CI.

```sh
cargo deny check                    # whenever Cargo.toml or Cargo.lock moves
scripts/nix-validate.sh --self-test # whenever backends/nixos.rs changes
```

`cargo deny check` covers advisories, licences, sources and duplicate versions. A dependency bump
is the only thing that can fail it, and it will: a new crate can arrive under a licence the allow
list does not carry.

`scripts/nix-validate.sh` asks a real Nix parser about every module `backends/nixos.rs` generates
*and* every `configuration.nix` it edits. No Rust test can do this — a Rust test suite cannot
parse Nix — and the file is somebody's boot configuration, so a mistake there breaks their
machine rather than Shall.

**`--evaluate` is the half that catches an option name.** Parsing says nothing about whether
`services.nginx.enable` exists or whether `allowedTCPPorts` takes numbers, which is exactly what
the module grew when `service:` and `firewall:` became NixOS attributes. This mode imports each
generated module into a real NixOS module system and forces the attributes Shall writes. The
whole gate — six modules parsed, four evaluated, two container starts — measured 25s.
`--self-test` implies it, and proves *both* gates can fail: a module that is not Nix, and a module
that is perfectly good Nix naming a service nixpkgs has never heard of.

## Running tests

One binary, `suite`, listed module by module in `tests/main.rs`.

```sh
cargo test --no-fail-fast                             # everything
cargo test --test suite -- latency_budget_tests::     # one file
cargo test --test suite -- a_machine_converges        # one test, by substring
cargo test --lib                                      # the unit tests only, fast
cargo test --test suite -- some_test:: --nocapture    # see stdout/stderr
```

**A new test file does not run until it is a `mod` in `tests/main.rs`.** `every_test_file_is_in_
the_suite` fails when the two disagree — that gate is the only reason this arrangement is safe.

The suite is slow (tens of minutes on a loaded box) because a lot of it spawns the real binary.
Background it and keep working rather than watching it.

### Two harnesses, and what only the second one can prove

The Rust suite is **hermetic**: it drives mock providers through `MockExecutor`, so it proves
logic and never touches a real package manager. That is a deliberate design, and it has a hard
edge — any behaviour depending on a *real* manager's answer is outside what it can reach.

The container harness (`docker/integration/`, its own
[README](../docker/integration/README.md)) fills that gap by running the real binary against real
apt / dnf / pacman / apk / zypper / xbps in disposable containers.

```sh
./docker/integration/run.sh                          # the default distro set, package `jq`
./docker/integration/run.sh htop                     # different canary package
DISTROS="ubuntu arch" ./docker/integration/run.sh     # a subset
```

When that harness is red, these properties are **currently unverified** — it is a list, not a job
to re-run later:

* the removal guard's OS-essential protection against a manager that actually reports one;
* crash and fault injection (`crash/midway`, `crash/completed`, `crash/groupkill`) — whether a
  killed process leaves the state the recovery tests construct by hand;
* a backend's real install → list → binary-on-PATH → remove lifecycle;
* argv and terminator behaviour of a manager as installed, rather than as the table infers it.

On Windows there is `scripts/integration-windows.sh` for the host-native equivalent.

## What CI runs, and when

`.github/workflows/ci.yml`. The important distinction, because "CI is red" is ambiguous:

**Every push:** `supply-chain` (cargo-deny), `msrv`, `shell` (shellcheck), `build` (the target
matrix, with tests, clippy, fmt and pty behaviour on the Linux leg), `containers` (six distros'
real lifecycles), `harness-mutation`.

**Nightly (`schedule`) only:** `slow-containers`, `storage` (btrfs/lvm/zfs on loopback devices),
`macos-native`, `windows-native`, `argv-drift` (asks every installed manager whether it still has
these subcommands and whether `--` still ends its options), `rust-mutation`.

So when something is red, start with `gh run list --workflow=CI --event=schedule` — the push runs
are frequently all green and it is a nightly-only job, which is by construction the half that
touches real managers.

CI also runs linters the five-step chain does not: `shellcheck`, `actionlint`, `cargo deny`,
and the MSRV build. Run those in a container before pushing shell, workflow or dependency
changes.

**And one of them is not a linter at all — it is a mode bit.** The Linux build leg runs
`./scripts/nix-validate.sh`, spelled with a leading `./`, and a script committed from
Windows carries `100644` in the index because the filesystem here has no executable bit to
record. `main` went red on `a5d5517` with `Permission denied` and exit 126, on a commit
whose own diff was documentation. `every_shipped_script_is_executable_tests` in the suite
now asks git — not the working tree — whether every file with a shebang is `100755`. If it
fails, the fix is `git update-index --chmod=+x <path>`.

## Debugging

| tool | what it tells you |
|---|---|
| `--timings` | child-command count, total child time, overlap ratio, wave count — on **stderr**, so `\| jq` still works |
| `--dry-run` | the whole plan, executing nothing |
| `--json` | machine-readable output for most read commands |
| `RUST_LOG=debug` | the tracing subscriber honours it, and it outranks `-v`/`-q` |
| `shall why <pkg>` | which declaration is responsible for a package being there |
| `shall path --explain` | which of the four config-root sources won |
| `shall check` | drift, unmanaged packages and backend health in one pass |

`--timings` is the one to reach for first on anything performance-shaped. "No child commands —
this run asked no package manager anything" is a sentence that settles arguments: it distinguishes
a slow command from a busy machine, which a wall clock cannot.

## Common tasks

### Adding support for a package manager

Try to write a **row** in `src/backends/builtin_backends.toml` first — the same table a user adds
to, parsed by the same loader. A row that lists must also say how to read the listing (`reads`,
naming a function in `src/parsers/named.rs`, or a `[backend.parser]` shape), with a fixture of
real bytes behind it. A listing nobody can parse reads as an empty machine, and `sync` answers an
empty machine by installing everything.

Only if the manager needs one of the five shapes a row cannot express (see
[ARCHITECTURE.md](ARCHITECTURE.md#backends-are-data-not-code)) does it become a Rust module — and
then it needs an entry in `backend_is_data_not_code_tests.rs`'s exemption table with a reason.

Get the manager's real output rather than reasoning about it. A container with that manager
installed has settled questions that argument could not, more than once.

### Adding a verb

The clap enum in `src/cli/args.rs` is the surface; the implementation goes in `src/verbs/`, not in
`main.rs`, so the suite can link to it. Add it to `COMMAND_MAP` in `args.rs` — `help_map_tests`
compares the map against `--help` in both directions. Classify it in `src/core/latency.rs` so it
has a budget class, and check `tests/named_commands_exist_tests.rs` for what else expects it.

### Changing the grammar, or what a config may say

`examples/` is not documentation that sits beside the code — it is parsed by the code.
`every_example_is_checked_tests` reads every `.toml` there into the real `Config` and every
line of every `.txt` through the real grammar, so a syntax you remove or an option you rename
fails the build at the example that still uses it. Update the examples in the same change; that
is the point of them.

The same applies to the words the grammar reserves. `every_prefix_advertises_a_line_this_grammar_accepts`
parses each `KEYWORDS` entry's own `means` string, because that string is what an error hands
the reader as the correct form. `service:` advertised `@state=running` when the option is
`@status`, and `shim:` advertised `@target=` when it takes `@source=` — two of the eleven
prefixes documenting an option the same file rejects. The first was found only because an
example copied the advice; the second was found by the gate, thirty seconds later.

### Adding a test file

Name it as a sentence describing the property, ending `_tests.rs`. Add it as a `mod` in
`tests/main.rs`. Watch it fail before you make it pass — for a bug fix that is not a suggestion,
it is how you find out the test can fail at all.

## Traps this repo has actually hit

Not hypotheticals. Each cost real time here.

* **`command -v` answers from the shell's hash table** and keeps naming a deleted binary. It is
  not a test for "is this package gone".
* **A CRLF `.sh` file** bind-mounted into a container makes `dash` abort with `set: Illegal option
  -` before any check runs. `.gitattributes` pins `*.sh text eol=lf`, but that governs checkout,
  not what your editor writes afterwards.
* **Git Bash rewrites anything that looks like a path** in a `docker -v` argument into nonsense.
  `MSYS_NO_PATHCONV=1` is the fix; the failure surfaces as docker exit 125, which is the CLI
  refusing to start a container and says nothing about your code.
* **A `.ps1` written from bash with a non-ASCII character** silently fails to parse. Parse-check
  before spending a UAC prompt on it.
* **`tee ... | head`** SIGPIPEs the tee and silently truncates the file you thought you were
  saving. Redirect to a file, then grep it.
* **A wall clock in a parallel test suite measures the suite.** If a timing assertion is failing
  on a different command each run, that is contention's signature, not a regression.
