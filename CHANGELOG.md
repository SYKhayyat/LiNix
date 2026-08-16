# Changelog

All notable changes to Shall are documented here.

## [0.8.0] — unreleased — the first published binaries

**The first release anyone can install without a Rust toolchain.** `0.7.0` named the rewrite in
this file and was never tagged, so no binary was ever published and `install.sh` compiled 448
crates on every machine that ran it. `0.8.0` was written up the same way a day earlier and was
about to repeat it — and the reason was not the missing tag. `ci.yml` listened for
`push: branches: [main]` and nothing else, so the release job's `if: refs/tags/v*` was gated on a
ref its own workflow could never be running under. A tag would have done nothing.

This is the one that ships: `shall-<target>` binaries for x86_64 Linux, both Apple architectures
and x86_64 Windows, and installers that download them.

*What follows is a day's work on top of that write-up — the 2026-08-11 assessment's order, and
five things the assessment did not know about. All of them are in this release, which is why the
entry it was written for now carries them rather than sitting above an untagged number.*

### `lock` and `unlock` take a list of nine kinds, not one of three axes

`shall lock` froze three things and there was no way to ask for less than one of them. The
vocabulary is now nine kinds in three groups, and the same words work on the command line, in
`--except`, and in `preferences.toml`.

```sh
shall lock exec,hooks                          # a list of kinds
shall lock everything --except versions:cargo  # everything, minus one manager's pins
shall lock versions:apt curl                   # apt's curl, and not cargo's
shall lock hooks:after_install                 # one hook, across every package
shall unlock --list                            # every entry, under its own kind
```

- The groups are `everything`, `packages` (`versions`, `backends`) and `scripts` (`hooks`,
  `events`, `adapters`, `exec`, `generate`, `health`, `vars`). Approving all seven script kinds
  to approve one was never a limitation of the ledger — each already had its own identity in
  `locks/hooks.toml`. It was a limitation of the word.
- `kind:qualifier` narrows below the kind. Four kinds divide (`versions`, `backends`, `hooks`,
  `events`); the other five are flat sets whose granularity is the item's own name, and asking
  for a sub-category there is refused with what to type instead, rather than silently matching
  everything.
- **`--backend` is gone from these two verbs.** The manager belongs in the word because an
  exclusion is a *list*, and "everything except cargo's pins" has no spelling as a flag. It
  cannot be a bare word either: `apt:apt` is a real package on every Debian machine, so
  `lock versions apt` has to keep meaning the package.
- `[lock] freeze` and `[lock] except` narrow what a bare `lock` freezes, in exactly those words;
  `[lock] versions` names which managers get pins; `[lock] replay` says whether an ordinary
  `sync` installs the recorded versions — which was hardcoded, so the only way to decline it was
  `--upgrade` on every invocation for ever. A `[lock]` block that will not parse freezes
  everything and says so, and `shall check config` now reads the same parser so the mistake is
  findable before the run that trips over it.
- Fixed on the way: `unlock backends:cargo` would have cleared **every** manager's recorded
  resolution, an undo wider than the thing it undid.
- **A manager pin that a sync cannot satisfy now explains itself.** When an install fails on a
  version that `locks/versions.json` recorded, the failure names the file, says Shall wrote the
  number, and gives the four ways out. Derived from disk at the moment of failure rather than
  from a provenance bit on the spec, and withheld when the manager's complaint does not quote
  the pin — so a dead mirror is never blamed on a lockfile.

### `upgrade` no longer runs every step, and a step can wait for a big enough run

- **`shall upgrade curl` used to fire every `@on=upgrade` step**, firmware included. A narrowed
  upgrade — named packages, `--backend`, `--security`, `--canary`, or a profile/module scope —
  now runs no steps unless asked. `--steps` and `--no-steps` reach both answers explicitly.
- **`@after=N` on an `exec:` line** runs the step only on a run that actually moved at least N
  packages: `exec:./firmware.sh @on=upgrade @after=5`. `@after=0` is refused rather than read as
  "always" — a threshold of nothing means the author meant something else.
- The native whole-system path (`apt upgrade` and friends) reports no per-package count, so it
  answers **unknown**, not zero, and unknown runs the step. Skipping a firmware step after the
  run that moves the most would be the wrong direction to be wrong in.

### A `setting:` is read back, so a sync that changes one says so

`check` and `plan` called every `setting:` line *unverifiable*, and unverifiable places — so a
converged machine reported work it would not do, a settled key was written on every sync for
ever, and a sync that genuinely changed a registry value printed `already up to date`. The
reason given was that the store has no "current value" command; every row in
`setting_stores.toml` carries `read`, a row whose `read` is empty is refused at load, and the
installer had been calling exactly that pair all along.

The probe and the installer now ask one function. A read only counts if it **exits clean**: a
schema the store does not know, a hive this account cannot open, or a `@scope=system` line
against a store with no machine-wide commands stays unverifiable and is never reported as drift.

### An AUR package is named under a manager that can put it back

`pacman`, `yay` and `paru` are three clients of one database, and the row that survives the
collapse used to be pacman's always. That is right for removal and wrong for a manifest: pacman
removes an AUR package and cannot reinstall it, so `pacman:<aur package>` is a line you cannot
delete and add back. The owner now speaks for the packages its repositories supply and the
helper speaks for the rest, told apart by `pacman -Qmq` — asked once per run, and only on a
machine that has both. `adopt`, `list`, the undeclared crawl and `uninstall --absent` all read
the same answer.

### Two commands now return an exit code they did not

- **`shall plan` exits `2` when the plan it wrote is not empty.** It answers the question `check`
  answers *and* writes the artifact a script consumes, so a pipeline that branches on drift
  reaches for it — and it returned `0` every time, including while printing `1 install(s), 0
  removal(s)` on the line above. The condition is `check`'s condition, over the same quantities,
  because two readings of one machine that disagree is the defect this repository keeps paying
  for. `shall list --outdated` is deliberately unchanged: a listing's subject is inventory rather
  than a verdict.
- **`shall sync` exits `1` when a declaration could not be acted on**, counted per declaration so
  a partial skip is caught. It warned and returned `Exit::Converged`, which matters most where
  nobody reads warnings: `sudo` ships `secure_path` without `~/.cargo/bin`, `~/.bun/bin` or
  `~/.local/bin`, so an unattended sync could install none of what it was asked for and report
  success — while `shall check`, on the same state one line later, reported drift and exited 2.
  A *removal* the guard declines is not this: that is the guard working, and it is the ordinary
  state of every adopted machine. The two used to share one list and are now distinct in the type.

### `@sandbox` says so when it cannot confine

- **It ran the command unconfined and mentioned it at `debug`.** On a Linux host without `bwrap`,
  `shall run -p pkg@sandbox -- cmd` executed with an unmodified environment; the one
  user-visible warning was on a branch that could not be reached, because the predicate in front
  of it (`bwrap_available() || fallback_allowed`) had already folded the condition in and was a
  constant `true` under the default. The fallback logged *"Falling back to PATH isolation"* over
  a bare command that isolated nothing.
- **The permission stays; the silence goes.** `sandbox.fallback_allowed` still defaults to `true`
  — a user who asked for confinement on a host that cannot provide it is owed the fact, not a
  program that decides on their behalf that they may not proceed. Every unconfined run is now
  announced at `warn!` before the command starts, and one function (`Sandbox::decide`) answers
  the question for `run`, `shell` and `wrap` so they cannot disagree.
- **`sandbox.require_bwrap` now does what its documentation always said.** It was declared,
  defaulted, serialised and **read by nothing**, while its Windows twin was wired — so an
  administrator who wrote it got byte-for-byte the same unconfined run. It is read, and it
  outranks `fallback_allowed`.

### An option written straight after a value is no longer swallowed

- **`cargo:ripgrep@version=1.0.0@hold` parsed as one package at version `1.0.0@hold`, with no
  hold, and said nothing.** The refusal for this existed and required a space before the `@`, so
  the same text was accepted or refused by how it was typed. All ten bare flags went the same
  way — including `@sandbox`, which decides whether a command is confined, and `@system`, which
  decides whether a package is written into the environment the OS owns.
- Values that legitimately carry an `@` stay legal: `@requires=@angular/cli`,
  `@source=github:owner/repo@v2` and semver build metadata like `@version=1.2.3+build@7`.

### `check` no longer loses one kind of drift by finding another

- **One skipped declaration erased every other kind.** The skip arm was matched before the counts
  arm, so a machine with one skip and any amount of real pending work reported the skip and
  nothing else — and a declared `link:` missing from disk vanished from the JSON as well as the
  prose, because `place`/`undo` had no key in `counts` and existed only inside the summary
  sentence that arm replaced. The row is built by appending what is true, and every quantity the
  prose can mention has a key beside it.

### `sync --dry-run` asks the guard the question `sync` asks

- **The rehearsal reported `install 0  remove 13`, exit 0, nothing protected; the same command
  without the flag exited 3 and named ten protected packages.** `plan` had it right over
  identical state the whole time, which is what made this a defect rather than a missing feature.
  The dry run now calls the same `preview_refusals` `plan` calls.

### A backend says whether it has ever met its manager

- **`check health` marks a backend no harness has ever driven** as `(unproven — no harness has
  run it)`. 62 backends ship and a substantial minority have never completed a real install →
  list → binary-on-PATH → remove anywhere; a user could not tell those from the ones with a
  lifecycle behind them, because they were listed side by side in the same words. The reasons
  live in `src/backends/proving.rs` and the coverage gate reads that table rather than its own
  copy of it.

### The project is called Shall

- **It was LiNix.** Everything that carried the old name moved with it: the binary and the crate
  are `shall`, the config directory is `~/.config/shall`, the machine file is
  `/etc/shall/machine.toml`, every environment variable took the `SHALL_` prefix, the release
  assets are `shall-<target>`, and the hook files dropped into `/etc/pacman.d/`, the zypp plugin
  directory and `/etc/sudoers.d/` are named `shall`.
- **Nothing was ever published under the old name.** No tag, no release, no crates.io entry — so
  there is no installed binary anywhere that needs replacing, and this entry is a rename rather
  than a migration.
- **The old paths are not read.** A config directory left at `~/.config/linix` is not found and
  not fallen back to; move it. P2 applies to a name exactly as it applies to a file format, and a
  dual lookup added here would only need deleting later.

### `watch` no longer disables the rest of the CLI

- **A command whose duration is a person's or a loop's takes the data lock at the write, not for
  its run.** `Commands` now answers `LockScope::Reader`, `Writer` or `Deferred`, exhaustively, so
  the next unbounded verb does not compile until it says which it is. `watch` was a whole-run
  writer and never returns — so for as long as the documented GitOps daemon was up, every writing
  Shall command on that machine waited 120 seconds and then failed, including `shall install` and
  the `hook-reconcile` a hand-typed `apt install` fires. `shell` (an interactive `$SHELL`) and
  `run` (a command Shall does not own) were the same shape. This bug had already been found and
  fixed three times, for `edit`, `fleet` and `history`, each time for one verb.

### `@hold=true` in a manifest now holds

- **It was read by nothing.** The option is in `PACKAGE_OPTION_KEYS`, the grammar refuses it
  beside `@version` as a contradiction, and II.2 documents it — and the only writer of the held
  set was the imperative `shall hold`, so a declaration carrying it parsed, validated, and did
  nothing at all. Found by making `tests/grade6_option_edit_reaches_the_machine_tests.rs`
  table-driven over `PACKAGE_OPTION_KEYS` itself: every one of the 24 keys now declares where its
  value ends up, and the six that are read only while installing are a confession under a ceiling.
- **There were four readers of the hold set, not two, and a file-level check saw two.**
  `upgrade --security` copied the ledger into a closure of its own, so it matched no grep for the
  ledger's readers and silently remediated a package the manifest had frozen — a change to a
  declared package, against the declaration. The "holds are not enforced by a native whole-system
  upgrade" note counted the ledger, so somebody whose holds were all declared was told nothing.
  And `shall hold` with no arguments — the command whose entire job is *tell me what is held* —
  answered `No packages are held.` over a manifest holding three. `app::holds::Holds` is the
  union now, no other module may reach the ledger at all, and the listing says which command
  releases each hold, because the two are released differently.

### Hooks, and the answers Shall gives about a machine

- **All three hook subcommands stand down when Shall started the manager.** The guard matched
  `hook-reconcile` alone — what apt, dnf, zypper, apk, xbps, portage and eopkg invoke. It did not
  match `hook-record`, which is what Shall installs as pacman's `PostTransaction` hook, so every
  pacman transaction inside a sync waited the full 120-second lock timeout in silence and lost
  the record anyway. It is a property of the command now, not a third match arm.
- **A resolve that failed is no longer reported as a machine that lacks the package.** `info
  cargo:ripgrep` on a host whose `priority` does not list cargo printed *"is not installed on
  this machine"* at exit 0 — a claim about the user's computer, arrived at by discarding an
  error. Same class fixed one call further on in `list --outdated`, which printed *"Everything is
  up to date"* over a manager whose registry was down.
- **A sudo refusal is remembered.** The success was cached and the failure was not, so a verb
  that continues past a failing backend re-spent the 120-second password bound per manager — the
  two 900-second wedges the `tools` nightly reported every night.

### Argv, output and the gates

- **Four terminator rows replaced inferences with measurements** (`winget`, `choco`, `launchctl`
  now terminate; `stack` does not). The differential probe disagreed with all four; `stack` was
  the unsafe direction, where Shall was passing a `--` the tool reads as a package name.
- **Nothing writes ANSI escapes into a pipe.** The tracing subscriber never asked whether stderr
  was a terminal, and `TERM=dumb` was honoured by nothing.
- **CI has a concurrency group**, and — the half nobody had looked at — **it listens for tag
  pushes**. Without `tags: [ 'v*' ]` the release job was gated on a ref its workflow could never
  be running under, so the two releases this file has named could not have been published even
  with a tag.
- **`unzip` is in five container images.** bun's installer needs it and `|| true` hid that.
- **The fan-out commands have a budget at last.** `list`, `search`, `check` and `adopt` carry no
  ceiling in seconds and correctly so, but `--timings` has computed the overlap ratio and the
  wave count since it was written and nothing read either. A change that serialises the fan-out
  now fails a test instead of staying inside a budget of `None` for ever.

### Guard

- **A removal ceiling per kind, and one over every change** (`N8`). `max_port_closures`
  (default 20) splits ports out of `max_extra_removals`; `max_total_changes` (default 0, off)
  counts everything one command does — installs and upgrades, removals of every kind, resources
  written, ports opened and closed. A refusal now names every ceiling it hit rather than the
  first, and `shall protected` prints all five instead of `max_removals` alone.

### Other package managers, and the processes Shall starts

*Both found by the arch integration leg, which kills Shall mid-sync on purpose and then asks the
machine to converge. It could not, twice, for two different reasons — and neither was about the
crash.*

- **Shall waits for another package manager instead of failing at it** (`Q51`). An `apt upgrade`
  in your other terminal, an unattended-upgrade timer, a GUI updater: `shall sync` used to retry
  four times over about three and a half seconds and then say *"this is not the transient failure
  its output looks like"* — which was false in exactly that case. It now asks the machine which
  of three states the lock is in and does the matching thing: **wait** for a live holder,
  announcing who it is; **fail at once and name `shall heal`** for a lock left behind by a killed
  run, because waiting on that never ends; **back off as before** if the holder let go in
  between. Bounded by `manager_lock_wait_secs` (default 300, `0` to opt out), as one budget
  across the whole retry loop. Nothing is scanned unless the manager already said the word, so a
  successful install costs nothing.
- **`heal` waits for a manager that is still finishing before deciding its lock is stale.** It
  surveyed once at the top, correctly left a lock alone because a manager was alive, and then
  watched that manager — an orphan of the run it was called to recover — exit during the
  recovery. The lock went stale after the only step that could clear it had run, so `heal` ended
  by advising you to run `heal`.
- **`pacman` and `yay` no longer fight each other.** Backends that drive the same manager took
  their own exclusive locks — an ordinary Arch config with repos from one and the AUR from the
  other ran them concurrently and let `db.lck` arbitrate, which it does by failing whichever
  lost. Same for `apt`/`apt-get` and `dnf`/`yum`/`microdnf`.
- **A package manager is asked to stop before it is killed** (`Q52`). `kill_on_drop` and the idle
  timeout were SIGKILL, which cannot be caught — so a manager Shall stopped never rolled its
  transaction back or unlinked its lock, and Shall was manufacturing the wedged machine `heal`
  exists to repair. Worse, Shall's child is usually `sudo`: SIGKILL killed `sudo` and left the
  real manager running as root with its parent gone. Now SIGTERM, a grace period, then SIGKILL
  only for a child that will not go.
- **Seventeen places that started a process now own it.** Dropping a future that awaits a command
  does not kill it — the process is detached — so a `generate:` command outlived the sync that
  asked for it, a hook outlived the node that fired it, and a secret decrypt outlived its own
  timeout under a comment promising it would not. None of them was bounded either. Everything now
  goes through one of three doors, and a test fails the build on a new one that does not. Ten of
  the seventeen were found by that test on its first run, not by reading.
- **Blocking waits no longer park a runtime worker.** A confirmation prompt, the history and
  preview TUIs, `git` after every sync, a `--help` probe, an external `vars` provider, and the
  data-directory lock's two-minute poll each held a thread for their whole duration.

### A `dnf` failure that read as success

- **`dnf` reported success over a transaction that never happened** (`S83`). `dnf check-update`
  exits 100 when it *finds* updates, so Shall rightly forgives that code — but with no failure
  phrasing able to contradict it, every dnf run ending on 100 read as a success, including one
  that did nothing. It is the same defect that once let choco's 3010 stand over an install that
  installed no package. Fixed by giving dnf its own words, measured in the Fedora image rather
  than guessed: `Failed to resolve the transaction`.
- **The gate that should have caught it was only ever pointed at Windows.** It walked the
  backends that register on the machine running the test, so it audited choco, winget and scoop —
  and never apt, dnf or pacman. It now walks the policy table, so all eighteen managers are
  audited on every platform.

### Supply chain

- **Every dependency is at its current release, majors included.** Ten Dependabot proposals in
  one pass: the sixteen-crate minor group (tokio 1.53, clap 4.6.6, regex 1.13, serde 1.0.229 and
  the rest), four crate majors — `thiserror` 1→2, `mlua` 0.9→0.12, `zstd` 0.11→0.13, `cron`
  0.12→0.17 — and five GitHub Actions: `checkout` v4→v7, `cache` v4→v6, `upload-artifact` v4→v7,
  `download-artifact` v4→v8, `action-gh-release` v2→v3. No source change was needed for any of
  the four majors, which is worth writing down rather than assuming: the surface Shall uses of
  each is small, and `mlua` 0.9→0.12 across three minor versions of a vendored-C binding was the
  one that could have cost a day.

- **`zip` 0.6→8 and `bzip2` 0.4→0.6 came with them, and neither was proposed.** Taking `zstd`
  0.13 alone would have left the tree compiling that C library **twice**, because `zip` 0.6 pins
  0.11 — a duplicate `cargo deny` reports as a warning and nobody would have read. Bumping `zip`
  collapsed it and introduced the same problem one crate over (`bzip2` 0.6 beside our 0.4), so
  that moved too. One `zstd`, one `bzip2`, and a `zip` that is no longer three years old. No
  source change for either: Shall's whole use of `zip` is `ZipArchive::new` and `extract`.

- **`bzip2-1.0.6` joins the allowed licences, and it grants nothing new.** `bzip2` 0.6 uses
  `libbz2-rs-sys` — pure-Rust libbzip2 — which *declares* bzip2's own BSD-style licence where the
  old C wrapper declared only its own `MIT OR Apache-2.0` and vendored the same upstream code
  under the same terms. Shall has shipped that code since it could open a `.tar.bz2`. The row
  names an obligation that was already being met and was previously invisible.

- **One advisory is silenced, and it says so.** `RUSTSEC-2026-0249` — `smartstring` is
  unmaintained, its repository archived on 2026-05-03. It is not a vulnerability and there is no
  upgrade: it is a non-optional dependency of `rhai`, which is one of the three hook dialects and
  what `vars.shall` runs on, and the newest rhai still carries it. The entry in `deny.toml` names
  the advisory, why no fix exists, and the one-line check that ends it. A test now audits that
  ignore list the way every other exemption table here is audited — a supply-chain gate with a
  quietly growing list of exceptions is not a gate.

### Looking

- **`shall check adapters`**, the ninth section: extension files that are written and not in
  use. A malformed `adapters/*.toml` still warns and is skipped mid-sync — a typo in an optional
  file must not stop you installing a package — so this is where it is a non-zero exit instead.

### Shipping

- **CI ran for the first time in ten commits.** A step ending in `pty_tests::` made the workflow
  file unparseable, which fails the run rather than a job: zero seconds, no log, nothing to open.
  A `cargo test` gate now reads every workflow for the class of scalar YAML re-reads as a key.
- **Release assets are named for their target.** All four builds produce a file called `shall`,
  so the release job as written would have published one binary and let three platforms download
  the wrong architecture.

### Performance

*The whole of `docs/INEFFICIENCIES.md`, which audited every place in the tree slower than it has
to be. Shall spends its entire runtime waiting on other people's processes and other people's
networks, so all of this is one of four shapes: don't ask twice, don't ask one at a time, don't
ask at all, ask in one breath.*

- **`list --outdated` asks each manager once instead of each package** (`Q44`). It walked the
  installed set calling `lookup(name)` — and `lookup` defaults to a whole *search* for that one
  name, so a 280-package machine ran 280 registry searches. Measured **771.4 s, against 2.9 s for
  the `list` that fed it**; after, **25.6 s**. Thirteen managers now answer in one call, and every
  parser but brew's is written against output captured from the real tool (apt, dnf, pacman, apk
  and zypper from containers). A manager with no such verb is still asked per package, but
  concurrently — `cargo` has no outdated check at all, and that is stated rather than hidden.
- **Five backends stopped running one command per package** (`Q45`) — brew, nix, mise, vscode and
  snap's removal. `brew install a b c` resolves the dependency graph once; one at a time was N
  resolutions and, under `run_exclusive`, N serialised lock acquisitions. Verified by running the
  real tool in containers for nix (`removed 2 packages, kept 17`), mise and brew; vscode and snap
  are argv-tested only and the register says so. `snap install` still cannot batch — it chooses
  per package between `install` and `refresh --channel=`.
- **pixi, dotnet and scoop are read from JSON where the tool offers it** (`Q43`), instead of a
  box-drawing tree and two fixed-width tables. **Asked for, never assumed:** the flags are
  version-dependent, so a manager that refuses is read from its text listing, once per run.

- **Packages going to the same manager in the same wave now share one command line** (`Y1`).
  Measured before: six declared packages produced **six separate `apt` processes** and
  12,465 ms, against **3,161 ms** for `apt install` of eight packages as one command — and one
  at a time the cost was superlinear (8 packages took 31,901 ms). A dependency edge still splits
  the wave, an install and a removal are still two commands, rollback is still per package, and
  the line is bounded so it fits. **The batching code was already written and had never been
  handed more than one package.**
- **A manager is asked what it has installed once per run** (`Y1`). Eighteen backends answered
  `info(name)` by listing the whole machine and finding one entry, and the callers asked once per
  *declared* package: measured as exactly `declared + 1` `dpkg-query` calls for a read-only
  `check drift` on Ubuntu, and **~247 ms per additional declaration** on Windows, where
  `winget list` takes over a second.
- **The post-install listing is gone.** Every install ran a full `info()` — on a generic backend,
  a whole `choco list` of the machine — to read a `download_size` property **no backend in this
  tree has ever produced**. The docs record an `install choco:bat` as a 399s transaction of which
  18.75s was the install.
- **`network_parallel`** (default 16), separate from `max_parallel` (`Y2`). Sockets are not
  bounded by core count; a four-core laptop ran `search`'s ~22 registry queries in six
  sequential waves.
- **`search` gives up on a backend that will not answer** — twice the configured network
  timeout, floor 30s — and names it (`Y3`). Its latency was the maximum over every registry
  rather than the median; it measured anywhere from 15s to 160s.
- **The pre-sync restore point runs alongside the pre-flight instead of in front of it, and says
  it is happening** (`Y4`). On Windows it is a measured 50.8s, and nothing in the output
  mentioned it, so the pause read as a hang.
- **`upgrade` overlaps the managers that contend with nothing** — `cargo`, `npm`, `pipx`, `uv`,
  `yarn`, `pnpm`, `vscode`, `emacs`, `krew`, `go`. The ones that share a system package database
  still run one at a time (`Y2`).
- **Variables resolve once per invocation**, which Part IX has always required. Measured: one
  `shall check` ran the user's `vars.sh` **three times**, so its side effects happened three
  times and any `http()` variable was fetched three times.
- **One HTTP connection pool** instead of a fresh `reqwest::Client` per request in eight places.
  Every OSV advisory GET, every registry query and every asset download paid a full TCP and TLS
  handshake to a host the previous request had just finished talking to.
- **Serial fan-outs that had no reason to be serial now overlap**: the priority chain across
  every bare name, `adopt`'s two crawls, `check health`'s ~55 backend probes (which ran twice —
  once for the rollup, once for the detail view), the removal guard's per-backend essential
  queries, OSV advisory fetches, dependency expansion, `fleet`'s hosts, `generate:` scripts, the
  post-sync health checks, orphan listing, cache cleaning and the reachability probe.
- **PATH lookups are memoised.** `is_available()` is a `which` call on ~45 backends and
  `registry.available()` is called at 20+ sites; on Windows a *miss* walks every PATH entry ×
  every `PATHEXT` extension, and a miss is the common case.
- **The write-ahead journal is append-only** (`data/journal.jsonl`, one JSON value per line). It
  re-serialised the entire map, pretty printed, through a temp file and a rename, on **every**
  state change — O(n²) bytes — while holding the one mutex every concurrent worker takes.
- **Removing a resolver defect that cost 2–3× the network work.** `remote_has` and `remote_info`
  both defaulted to a full search and neither was ever overridden, so the resolver could not tell
  an honest "no" from an unimplemented one and re-ran *the identical search with the identical
  argument*. One `lookup` answers presence and version together.
- Smaller: the exit policy builds its haystack once per command instead of three times;
  regexes from configuration are compiled once instead of per use; startup's four independent
  I/O operations overlap; the state registry is compact JSON and is serialised under the lock
  rather than deep-cloned across a thread boundary; `--dry-run heal` no longer moves a damaged
  WAL aside.

### Changed

- **`[backend_settings.flatpak]` takes `scope = "user" | "system"`, not `user = "true"`**
  (`Y22`). **Breaking: there is no fallback.** A row in the data-driven backend table substitutes
  a settings *value* into argv (`--{setting.scope|system}`), and a boolean cannot be written into
  a flag name without the placeholder growing a conditional — which is where a data path stops
  being data. The old key is refused **by name**, with a message saying what to write, because a
  `user = "true"` that silently stopped meaning anything would install for every account under
  a line asking for the opposite. Scope is parsed once, at registration, through the same
  `Scope` type as `@scope=` on `setting:`/`link:`/`shim:`; a value that is neither word is
  refused rather than defaulted.

- **`purge-unmanaged` is now `purge-undeclared`** (`Q31`). **Breaking: there is no alias.** The
  word `unmanaged` named two different sets on two screens of the same program — *what `adopt`
  would take* (1 package on the measured host) and *every installed package nothing declares*
  (34) — and the command that deletes was named after the one it does not act on. `unmanaged`
  keeps the first meaning; `undeclared` is the second, and it is now the word on `check drift`,
  in `plan --json`, in the readme and in the verb. Scripts that call the old name get clap's
  unknown-subcommand error, which is the right outcome for a command that deletes: a
  compatibility spelling that silently still works is worse than a failure.
- **`adopt` declares OS-essential packages instead of commenting them out** (`Q47`). They were
  written into a commented-out section on the grounds that a live line's deletion means
  uninstall — but the guard already refuses to remove anything a backend reports as essential,
  so the comment defended against something that could not happen, while leaving the 33 packages
  the machine cannot boot without outside the model entirely: no drift detection, nothing to
  heal, nothing to put back. They are now ordinary declarations, the guard is unchanged, and the
  manifest header names the exception rather than promising a deletion the guard will refuse.
  `adopt` also stops running one `essential` subprocess per backend it no longer consults.
- **`check unmanaged`'s skip line stopped blaming every skip on OS-essential.** It printed
  `found.skipped.len()` — the total across every reason — under a sentence naming one of them.
  It now prints the same per-reason breakdown `adopt` does.
- **`data/journal.json` is now `data/journal.jsonl`.** There is no old-format reader (NO
  LEGACY). A journal only records in-flight actions, and a wholly unreadable one is still moved
  aside and named rather than swallowed.
- **The `Parallel Task Breakdown` lines say when several packages shared one command.** Six
  identical durations under that heading used to be the signature of a fully serialised run.

### Removed

- **2.5 MB of specification, cut to 1.1 MB** (`Y21`, owner ruling): `docs/archive/` (twelve
  grade rounds and readiness reviews), `docs/spec/proposals/` (six designs, all ruled and folded
  into Part II), `docs/spec/history.md` (8,390 lines organised by *session*) and
  `docs/INEFFICIENCIES.md` (an audit with every finding dispositioned). 17,900 lines. The rules,
  their reasons and the rulings stay at full fidelity; what those files were *for* is thirty-one
  lines in `docs/attic/lessons.md`, which opens by telling agents not to read it. All of it is in
  git.
- `Config::validate()` — a validation function with no callers anywhere, whose one rule
  (`max_parallel > 0`) every one of its eleven consumers already enforces with `.max(1)`.

- `AppCore`/`AppServices` — a dead thirteen-field duplicate of `App` with no references.
- `PackageCache`/`SmartCache` — a TTL'd cache whose every accessor had zero callers. Replaced by
  a once-per-run listing memo, not resurrected.
- `utils::command` — a spawn-per-probe `which` with no callers, twenty lines from the in-process
  implementation that replaced it.
- The `rayon` and `nonzero_ext` dependencies (zero uses), reqwest's `blocking` feature, and
  tokio's `features = ["full"]` narrowed to what is used.


### Fixed
- **A `#!` hook runs on Windows** (`Y17`). It never had: Windows has no shebang mechanism, so a
  script file reached `CreateProcess` and came back *"not a valid application for this OS
  platform"* — a message blaming a script that was fine. Shall now reads that first line itself
  and names the interpreter on the command line, on every platform. `#!/usr/bin/env python3` is
  looked up as `python3`, then **`python`, then `py`**, because that is what a Windows install is
  usually called; an absolute interpreter that exists is used as written, so Unix launches exactly
  the binary the kernel would have; and an interpreter the machine lacks is refused **by name**,
  listing every spelling tried. The `#!` line itself needs no stripping — every language a shebang
  names treats it as a comment.
  - **`exec:` scripts and event hooks read it too.** They were the other two callers of the same
    file and ignored the first line on *both* platforms, since `sh <script>` does not consult a
    shebang either — so a `#!/usr/bin/env python3` event hook was already broken on Linux.
  - **A PATH candidate with bytes in it beats one without.** `which python3` on Windows returns a
    zero-length `WindowsApps` reparse point; configured it runs Python, unconfigured it opens the
    Microsoft Store — and **the two are identical to inspect**, so the dead one is out-preferred
    rather than detected. `winget`, which has no other form, keeps its alias.
  - **A `vars.py` provider finds whatever this machine calls Python.** It named literally
    `python` on Windows and literally `python3` elsewhere, so a machine with only the other
    spelling had a provider it could not run. Providers still choose by extension, not by
    shebang (IX.6) — only the name lookup is shared.
  - **The hook's temporary file dropped from 0755 to 0600.** The execute bit was there for the
    kernel; an interpreter named on the command line only reads it.
- **A read that fails no longer becomes an empty answer** (`Q40`). `run_output` ignored exit
  status by design — "no such package" is an ordinary non-zero reply — but it ignored the *silent*
  failures too. Measured without Shall present: 3 of 16 concurrent cold-start `winget list` exit
  `0x8A150001` having written zero bytes anywhere. Through Shall that became `Ok("")` → no
  packages → **`shall list --backend winget` printing nothing and exiting 0 on a machine with 280
  packages**, and `info` reporting an installed package as absent. Now a non-zero exit that said
  nothing on either stream is a failure; one that printed keeps what it printed. Fixed at the
  primitive and at the three callers that turned it into a claim.
- **Retryability is read from the exit code as well as the output** (`Q41`), and a read
  classified transient is retried (`read_retry_attempts`, default 3). Every marker list was text,
  and the failure above writes none — so the one signal present was read by nothing but
  `is_benign`. Reads only: a read is idempotent, a mutation retried on a guess installs twice.
- **`adopt` declares only what a manager can put back** (`Q36`). `winget list` reports 186 of 280
  rows as `ARP\…`/`MSIX\…` identifiers it synthesises from the registry; `winget uninstall` takes
  them and **`winget install` refuses every one**. `adopt` now reads `winget export` — 78
  declarations that all work — and names what it left out and why.
- **`gem` no longer reports `default: 4.0.10` as a version.** RubyGems marks the gems shipped with
  Ruby, and the marker is not part of the version — no `@version=` could ever match it.


- **A command that stops talking no longer stops Shall forever.** An uninstall sat 76 minutes on
  a Windows restore point that had already been written; nothing in Shall bounded a child
  process, because the only timeout in the tree covers the transaction DAG and snapshots, state
  reads, the guard and `plan` all run outside it. Two earlier hangs had been killed by hand and
  never diagnosed. A child that produces nothing on either stream and does not exit is now
  killed, and the error names the argv.
- **Ten spawns outside the executor were leaving stdin inherited while capturing both output
  streams** — `git` on every invocation, the `--version` and `--help` probes, `generate:`
  scripts, vars providers, the `sh()` builtin, download commands, the Windows sandbox probe. A
  child that prompted there asked into a pipe nobody displays and then waited on a terminal it
  was never handed. All ten close stdin now; the deliberately interactive ones (`shall run`, the
  shell, `$EDITOR`, the history TUI, the bisect oracle) are unchanged.

### Added
- **`query_idle_timeout_secs`** (default 120) — a read's own bound on silence (`Q42`).
  `command_idle_timeout_secs` is sized for `Checkpoint-Computer`, a mutation legitimately silent
  for minutes, and every read inherited it, so a wedged 1.5 s listing cost fifteen minutes.
- **`read_retry_attempts`** (default 3) — how many times a transient read is asked again.
- **`outdated_args` and `machine_list_args` in `adapters/backends.toml`** — a custom backend can
  declare the same two capabilities the built-ins gained. Absent means *cannot be asked*, never
  *nothing to report*.


- **`command_idle_timeout_secs`** (default `900`, `0` disables). Bounds **silence, not
  duration** — a build that prints for an hour is never touched, a manager that has stopped
  talking and stopped exiting is. Raise it if you drive something legitimately silent for
  longer.

## [0.7.0] — 2026-07-31 — v7, the declarative rewrite

> **The version is `0.7.0` and the design is "v7"**, which are two different numbers and were
> confusing each other. `Cargo.toml` said `0.1.0` while every document called the rewrite v7, so
> a user reading `shall --version` had no way to tell which tree they had. The crate version now
> tracks the design generation; the leading `0.` says what is true — this has never been
> installed from an artifact by anyone.
>
> **What "released" means here:** the tag is not pushed by this commit. `ci.yml`'s release job
> fires on a tag and publishes; that is an outward act and it belongs to the owner:
>
>     git tag -a v0.7.0 -m "v0.7.0" && git push origin v0.7.0


v7 is a rewrite, not an upgrade. The model changed: **one file says what should be installed,
and `sync` makes the machine match it.** Everything that used to be a separate mechanism for
getting there is gone, because editing the file and syncing already did it.

There is no migration path and no compatibility shim. Nothing reads a v6 config.

### The model

- **One grammar, one parser.** `backend:name` is parsed in exactly one place. Options take a
  short form (`@version=1.6`) or a block form for anything containing a comma; an unrecognised
  line is an error naming the file, the line, and what was expected, rather than being read as
  a package name.
- **`when` gates lines everywhere it appears** — packages in a module, imports in a profile,
  backends in `priority`, profile names in `active`. One rule, no per-file exceptions.
- **The repo layout is `modules/`, `profiles/`, `active`, `priority`, `schedules`, `locks/`.**
  A module is a list; it does nothing until an active profile `use`s it.
- **History is git.** `shall git init` makes the config directory a repo, every sync commits,
  and `shall rollback <commit>` restores those manifests and converges the machine. There is no
  second generation store.
- **`activate` sets, `activate -a` adds, `deactivate` removes.** Several profiles can be active
  at once; their package sets are unioned, and deactivating one removes only what nothing else
  still needs.
- **You choose which file a release installs.** `formats` is an ordered preference over a closed
  vocabulary (`deb rpm appimage tarball zip exe msi pkg dmg binary`), defaulting to something
  sensible for your OS and distribution so most repos never write it. `@asset=` narrows by
  filename or glob when a release ships two files that both fit; `@bin=` names the executable
  inside an archive when the guess would be wrong. Assets your machine cannot run are filtered
  out before any of this, so there is no architecture option to get wrong.
  - This replaces a scoring heuristic that had **no tie-break**, so the same declaration could
    install a different file on two machines depending on the order the GitHub API returned
    assets in. It also picked a "best" asset even when every candidate was for another
    platform. Selection is now reported and recorded, so a pinned declaration cannot quietly
    resolve to a different file later.
  - Nothing matching your `formats` is an error listing what the release actually offered and
    why each asset was skipped — not a fallback to whatever came first.
  - **`@asset=all` installs every file it matches.** One file is deployed under the repo's name
    as always; several each keep the name of the program inside them, and two that would land
    on the same name is an error naming both files rather than one overwriting the other.
    Everything is downloaded, checked and unpacked before anything reaches your `PATH`.
- **Storage is declarable.** `btrfs:/mnt/fs/srv@quota=20G,mount=/srv`, `zfs:tank/media@quota=500G`
  and `lvm:vg0/data@size=100G` are declarations like any other — they have a size and a
  mountpoint rather than a version, which is the only thing that makes them different. A declared
  mount is written to `/etc/fstab` so it survives a reboot, and taken out again *before* the
  volume is destroyed. Deleting one of these lines erases a filesystem, and it goes through the
  ordinary removal guard: protectable, counted against `max_removals`, previewed first.
- **An option is legal exactly where something reads it.** `@classic` on a snap, `@size` on a
  logical volume, `@quota` and `@mount` on a storage object — each is refused *by name* on any
  backend that could not act on it, rather than being accepted and ignored. The grammar's list
  and the keys backends read are one list with a test across the join, which is what stops a key
  being read by code no line can reach.
- **`shall path` and `shall edit` find your files for you**, so neither you nor your scripts
  have to hard-code `~/.config/shall`. `shall path --set DIR` records the repo location in
  Shall's own settings file — the one file that lives outside the repo, because a key inside
  the repo saying where the repo is cannot be read before you know where the repo is. That
  file holds exactly one key and the parser refuses any other, naming `preferences.toml` as
  where behaviour settings belong. `--config-dir` overrides it for one run; the order is
  `--config-dir`, `$SHALL_CONFIG_DIR`, the settings file, the default, and
  `shall path --explain` says which one won.
- **Behaviour lives in `preferences.toml`, inside your repo.** `config.toml` is gone — it was
  never in the spec's file list, and it held the key that said where the repo was, which could
  only ever be read from the directory it was moving away from. An unknown key is now an error
  naming the key rather than a silent shrug, which it had been while eight documented settings
  no longer existed. `shall config path` and `config edit` are gone too; `shall path` and
  `shall edit` answer those questions, and `shall edit preferences.toml` re-checks that the
  file still parses when you save it.
- **You can name your own conditions.** A `vars` file holds `role = desktop` lines with `when`
  blocks that override them, and `when $role == travel` gates packages anywhere `when` is
  legal. The `$` keeps your names and Shall's detected facts in separate namespaces, so new
  facts can be added forever without changing what an existing file means. A variable needs a
  default at the top level — a `when` block may override one but never introduce it, so every
  variable is defined on every machine and a typo is always an error rather than a block that
  quietly never fires. Values may be built from other variables (`tier = ${role}-heavy`),
  resolved in dependency order with loops reported by name.
- **`shall rebuild` repairs what `sync` cannot see.** `sync` applies the difference between
  your files and the machine, so a package that is declared and installed but broken produces
  no difference and `sync` reports success over it forever. `rebuild` asserts the declared set
  from scratch instead — one backend at a time, all of its packages down and then all of them
  back up, which is what actually lets a shared dependency orphan and be collected. Backends
  that need root go first (a crate can need a system compiler; no system package needs a
  crate). There is no default scope, it never touches undeclared software, it names and skips
  protected packages rather than removing them, and it cannot be put in `schedules`.

### Safety

- **One guard, every removal path.** Removal count ceilings, the protected list, and the OS's
  own essential flags are enforced in a single function that every deleting command calls.
  `--allow-mass-removal` answers the count and nothing else; protection is a refusal, not a
  confirmation, so nothing overrides it.
- **`remove-orphans` previews, guards, then asks.** It lists what each manager considers
  orphaned, puts the whole set through the guard, and removes exactly what it showed.
- **`export` never silently overwrites.** A taken filename is written beside the real file
  (`package.shall.json`); `--force` overwrites deliberately.
- **File-backed backends no longer report a removal that failed.** If the binary could not be
  deleted, the package stays recorded rather than becoming drift nothing can see.
- **A crash aged out of the write-ahead log is still healable**, so an interrupted run left
  unattended for hours is still repaired rather than dropped.
- **`sync`, `rollback` and `remove-orphans` refuse to apply unconfirmed** in a non-interactive
  shell without `--yes`.
- **Commits are made as you, and signatures are shown.** Shall no longer authors its commits as
  `shall <shall@localhost>` — your git identity and your `commit.gpgsign` decide. `git log` and
  `history` show each commit's signature and signer, and a signature git will not vouch for is
  never displayed as a good one. `require_signed_history` (off by default) refuses a rollback to
  a commit git cannot verify.
- **A `link:` line that writes outside your home directory asks first.** `@target` can still
  point anywhere — that is what the link backend is for — but a destination like `/etc/cron.d/x`
  is listed and confirmed before the first install places it. Dotfiles under `~` are unaffected.
- **Secrets in a public config repo.** `link:` takes `@decrypt=age` or `@decrypt=sops`: the
  encrypted file is what git holds, the plaintext is written at `@target` (owner-only on Unix)
  and removed when the line goes. `--dry-run` never decrypts. Documented in the readme — the
  capability shipped earlier and went unmentioned until 2026-07-23.

### Removed

Each of these was a second way to do something the model already does. Deleted, not deprecated.
Where a name survives, it is because the command was rebuilt as "edit the line, then sync" —
the old engine underneath it is gone.

- **`teleport`'s implementation** — it built its own transaction graph and executed it
  **without calling the guard**, so it could remove a protected package every other path
  refused to touch. The command survives with none of that: `teleport ripgrep apt` rewrites
  wherever `ripgrep` is declared and syncs, so the move goes through the plan and the guard
  like any other change.
- **`shim`** — shims are declarative (`@shim=true` on a line). The imperative command was a
  second path that the next sync undid, and its required `--source` flag was never read.
- **`clean`** — split into `remove-orphans` and `clean-cache`. The old command ran
  `apt autoremove -y` / `pacman -Rs --noconfirm` across every backend with no preview and
  outside the guard, and on four backends it was cleaning *caches* while reporting orphan
  removal.
- **`generation` and `lease`** — history is git; leases are `@expires` on the line.
- **`managed` modes and the keep-list file** — the manifest says what is managed.
- **`prune`, `clone`, `migrate`, and the `-g` flag** — drift removal is what `sync` does,
  `adopt` takes over a machine, and `fleet` compares machines.
- **`cockpit`** — renamed `history` (alias `tui` kept), because it browses your manifest
  history and the old name did not say so.
- **Marketing language, emoji, and the theatrical house voice.** 149 log lines lost a
  `Component:` prefix, status lines were demoted to debug, and a normal run is now quiet the
  way `apt` and `dnf` are.

### Fixed

- **`shall --help` panicked on every debug build.** `status` carried an alias `diff` that
  collided with the real `diff` command; clap's debug assertions aborted before `main`. The
  test suite stayed green throughout, because nothing in it ran the binary.
- **A manager reporting "No packages found." was parsed as a package named `No`** — a phantom
  entry that `adopt` would write into a manifest and `purge-unmanaged` would try to delete.
- **`rollback` overwrote your manifests before the confirmation gate**, so a non-interactive
  run without `--yes` rolled the files back and then refused to converge the machine.
- **Failed snapshot deletions were counted as pruned.** `prune` now reports only what it
  actually removed and names what it could not.
- **A rollback that could not reinstall a just-removed package said nothing.** It now reports
  every compensating failure by name and returns an error.
- **A failed state write during auto-remediation was discarded**, leaving a package installed,
  in memory, and unrecorded — so the next sync read it as drift.
- **`unmanage` always printed "0 lines removed"** — the writer and the reader used different
  JSON keys.
- **`network_timeout_secs` was ignored below 10**, and `max_parallel` did not detect the core
  count.
- **A rebuild reported its removals as removals.** `shall rebuild` prints its own plan, but the
  two transactions underneath it ran through the ordinary sync path, whose summary said
  "Removals: N" on a run where all N come straight back. It now reads "Reinstalled" and
  "Removed to reinstall"; plain "Removals" means removals that stay removed.
- **`shall plan` did not say when a variable caused a removal.** `sync` named the variables that
  had moved since the last sync and `plan` — the command you read first — did not.
- **`shall status` reported packages only.** A deleted `service:` / `link:` / `repo:` / `shim:` /
  `setting:` / `schedule:` line is drift that `sync` undoes, and status called it nothing to do.
- **`shall init` did not create the `vars` file** it documents. It now writes a commented one,
  with no variable invented for you.
- **The Linux build did not compile.** `registry.rs` used `OrphanDryRun` in the apt block
  without importing it — invisible on Windows, where that block is `cfg`-ed out, so the whole
  container matrix failed to build until a run on Linux said so.
- **A refused `install` wedged the config.** `install` writes the line and syncs after it
  (S15), and the write happened before anything checked whether the backend was one Shall
  uses — so `shall install dnf:jq` on a machine without dnf left `dnf:jq` in
  `modules/imperative.txt`, and from that moment `status`, `plan`, `check`, `why`, `upgrade`,
  `conflicts`, `activate` and every later install were a hard parse error until someone edited
  the file by hand. `App::declare` now refuses such a line before writing it, which covers
  every landing (imperative, hooks, adopted) and `absent:`/`repo:` lines with it. A name
  nothing can resolve is still written and then withdrawn — that one is a failed install, not
  an unusable line.
- **`teleport` had the same fault one file over.** Moving a package to a manager `priority`
  does not list rewrote the line in place — leaving a backend nothing can parse, with the
  original already gone. Refused before the rewrite now.
- **`gem` could not install anything.** It was listed as a manager that ends its options at
  `--`, but RubyGems' `--` introduces the **build arguments** for a C extension — so
  `gem install -- colorize` named no gem at all and failed with "Please specify at least one
  gem name". Every `gem` install and removal through Shall had been broken since the option
  terminator was introduced.
- **`krew` reported READY on a machine without krew.** Its probe asked for `kubectl`; krew is
  a *plugin*, so `kubectl krew …` works only when krew has installed `kubectl-krew`. Every
  krew command failed with `unknown command "krew"` — and took `shall update` down with it.
- **One backend could cancel every backend after it.** `update` and `upgrade` swept the
  registry and gave up on the first failure, so a single manager that could not refresh
  silently skipped the rest. Each failure is named now and the sweep finishes.
- **scoop's `list` counted a failed install as installed.** scoop keeps such a row forever
  with an empty Version and Source and `Install failed` in Info; splitting on whitespace read
  that as a package named `jq` at version `2026-07-21`, so `sync` thought there was nothing to
  do and no `jq` was ever on PATH. `scoop list` and `scoop search` are sliced by header
  offsets now, sharing one table reader with the winget parser.
- **`cargo test` wrote into the repository on Linux.** Three test helpers fell back to the
  current directory when neither `TMP` nor `TMPDIR` was set — which is every plain Linux
  shell — leaving `shall-embedded-*.shall`, `shall-marker-*` and `shall-vars-test-*/` in the
  working tree. All three use the platform temp directory now.

## [6.0.0] — 2026-07-02

Class-defining cross-ecosystem features that are only possible because Shall sits above
every package manager at once, plus safety and honesty fixes.

### Added (features)
- **`audit`** — one security scan across every ecosystem. Queries OSV.dev for all managed
  packages (apt, npm, pip, cargo, gem, go…) and reports known vulnerabilities with fixed
  versions. `--json` supported.
- **`sbom`** — emit a single CycloneDX 1.5 software bill of materials spanning all backends.
- **`why <pkg>`** — provenance (which manifest/module/imperative action introduced it) plus
  cross-package reverse dependencies.
- **`upgrade --canary --test <cmd>`** — snapshot → upgrade → run health check → automatic
  rollback to the snapshot on failure.
- **`bisect --test <cmd>`** — binary-search system snapshots to find the change that
  introduced a regression (pure algorithm unit-tested).
- **`clone <user@host>`** — replicate another machine's installed packages over SSH,
  translating each to a backend available locally.
- **`fleet [hosts…] [--sync]`** — compare machines over SSH against their manifests, report
  drift, and optionally reconcile.
- **Policy gate (`policy.toml`)** + `policy` command — `deny_packages`, `allow_backends`,
  `pinned_only`, `require_snapshot`, `deny_vulnerable`, enforced before `sync`/`upgrade`.
- **`init`** — scaffold the directory layout and a starter manifest on a fresh machine.
- **Flight plan** — concise pre-flight summary (counts, backends, root, service restarts)
  before applying a sync/upgrade.

### Added (safety / config)
- **`prune_scope`** (`managed` default vs `system`) — optionally reconcile the *entire*
  system to your manifests, sparing protected packages.
- **`protect_imperative`** (default true) — imperatively-installed packages are shielded
  from drift pruning even when absent from manifests.
- **Lease enforcement** — expired temporary installs are swept on every state-changing run.
- **`fleet_hosts`** config for default `fleet` destinations.

### Fixed
- **Honest `clean_orphans`** — backends with no orphan concept now return `Unsupported`
  (reported as a benign skip) instead of silently succeeding; apt gains real `autoremove`.
- **Centralized sudo policy** — write sites route through `sudo_for_write()`; reads never
  escalate.

### Chore
- Version bumped to 6.0.0; repository is now `rustfmt`-clean; `clippy -D warnings` passes.

## [5.0.0] — 2026-06-26

This release closes the capability gaps across backends, fixes a data-loss-class bug in
scoped upgrades, makes parallelism configurable, and adds first-class application config.

### Fixed (correctness)
- **Scoped `upgrade` is now non-destructive.** `shall upgrade --module X` / `--group X` /
  `--profile X` previously scheduled removal of *every managed package outside the scope*
  (scope filtering ran before global drift-removal). Removal planning is now skipped
  entirely when a scope is set; a targeted upgrade only installs/upgrades within scope.
  Guarded by a regression test.
- **Scope matching is exact-segment, not substring.** `--module dev` no longer also matches
  `module:dev-tools`, while composite sources like `config:group:editors` still match
  `group:editors`.
- **`nix` multi-package removal** no longer removes the wrong packages. `nix profile`
  renumbers elements after each removal; removals now run highest-index-first (with a
  name-based fallback).
- **`is_protected` uses exact (case-insensitive) matching.** Protecting `libc`/`apt`/`kernel`
  no longer shields `libc-bin`/`aptitude`/`kernelshark` from removal.
- **`cargo list_installed`** skips indented binary lines (no more empty-named packages).
- **`yarn`** scoped-package parsing (`@scope/pkg@1.0.0`) no longer drops the name.
- **`flatpak update`** passes `-y --noninteractive` (won't hang on a prompt in automation).
- **`vscode` health check** no longer always reports OK; a missing `code` binary is detected.
- **`lease list`** no longer panics on a corrupt/out-of-range expiry timestamp.
- **`Config::from_file`** reads directly (no TOCTOU existence pre-check); a missing file
  cleanly falls back to defaults.
- **`winget` list/search parsing** is now column-position based. The old whitespace split
  corrupted multi-word names (`7-Zip 25.01 (x64)`) and ARP IDs, and failed to strip
  winget's bare-`\r` progress-spinner header — so `list`/`unmanaged`/`search` emitted
  garbage rows. (The previous unit test passed only because its fixture wasn't
  column-aligned; replaced with realistic fixtures.)
- **`repo list`** no longer prints the table header (`Name`/`Argument`) as a repository row.

### Added (capabilities)
- **`Searchable`** for `brew`, `cargo`, `npm`, `pnpm`, `yarn`, `mise`, `snap`, `flatpak`,
  `nix`, `emacs`, and `pip`. (npm/pnpm/yarn share an npm-registry HTTP search; pip uses an
  exact-name PyPI JSON lookup, since PyPI has no public search API.)
- **`RepoManager`** for `dnf` (`config-manager`), `pacman` (drop-in files under
  `/etc/pacman.d/` + a single `Include` in `pacman.conf`), and `winget` (`source` commands).
- **`Upgradable`** for `vscode` (per-extension `--install-extension --force`) and `emacs`
  (`package-refresh-contents` + `package-upgrade-all`).

### Added (safety & reproducibility)
- **`sync` no longer removes drift by default.** Drift removal is now opt-in: `sync` only
  installs/upgrades unless `prune_on_sync = true`. Removal is an explicit, separate step.
- **`shall prune`** — remove packages installed but no longer in your manifests (with a
  confirmation prompt; honors `confirm_destructive`/`--yes`).
- **`shall status`** (alias **`diff`**) — read-only report of what `sync` would install,
  what drift `prune` would remove, and what's installed-but-unmanaged. `--json` supported.
- **Per-backend version pinning for reproducible/locked installs.** Each backend now honors
  `options["version"]` in its native syntax: `apt`/`apk`/`zypper` `name=ver`, `dnf`
  `name-ver`, `pip`/`pipx` `name==ver`, `npm`/`pnpm`/`yarn`/`bun`/`mise` `name@ver`,
  `cargo`/`gem`/`winget`/`choco` via flags, `vscode` `ext@ver`. `brew` is best-effort
  (versioned formulae); `pacman`/`snap`/`flatpak`/`nix`/`mas` don't support fixed-version
  pins (rolling/channel/flake/store models) and install latest.
- **`shall lock`** — record the installed version of every managed package to
  `locks.json`, so `sync --locked` reproduces those exact versions on another machine
  ("reproducible inputs"; see README for the honest limits vs. Nix).

### Added (CLI)
- Previously-silent subcommands now work: **`teleport`**, **`unmanaged`**, **`update`**,
  **`shim`**. `orphans` now *lists* drift non-destructively (distinct from `clean`).
- **`shall config init | path | show`** to scaffold/inspect the application config.
- **`shall completions <shell>`** to emit a shell completion script (the generator
  existed but was never wired to a command).
- `install`/`remove` honor `--json` (with `--dry-run`) and emit a machine-readable plan.
- **Richer output:** `info` now shows version, description, install path, properties, and
  dependencies (previously only name+backend — the data was collected but discarded);
  `search`/`list` show versions inline; `search --json` added.

### Added (config)
- **`max_parallel`** now drives the install/remove transaction engine, not just search.
- New options: **`network_timeout_secs`** (HTTP search timeout), **`nix_gc_age`** (replaces
  the hardcoded 30d in nix GC), **`confirm_destructive`** (extra guard before removals).

### Changed / hardening
- Repo `name`/`url` and emacs package names are validated/escaped before being interpolated
  into shell commands or eval'd elisp.
- Cross-platform path handling for `npm`/`cargo`/`mise` install paths (Windows `.exe`,
  `node_modules` layout, `PathBuf`).
- Cleared all clippy warnings on the active target; added a GitHub Actions CI matrix
  (Linux/Windows/macOS) running build + `clippy -D warnings` + tests.
- Removed committed source-dump artifacts; `.idea/` is gitignored.
- **Registry refactored** from one ~590-line `create_default_registry` into per-backend
  `register()` functions (each specialized backend module owns its registration; generic
  CLI-config backends use small `register_*` helpers). Adding a backend is now a localized
  change. Backend count and behavior unchanged (verified live on Windows).
- **Resolver no longer drops duplicate sources.** A package listed in multiple sources
  (e.g. a manifest *and* a module) now accumulates all origins in its `__source` tag, so it
  stays visible to every scoped `upgrade --module/--group/--profile` it belongs to.
- `teleport`'s not-found error is no longer double-wrapped.

### Notes
- `pnpm`/`yarn` search returns npm-registry results (not the manager's own index).
- `pip` search is exact-name resolution only (PyPI has no public search API).
