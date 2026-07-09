# Changelog

All notable changes to LiNix are documented here.

## [Unreleased]

### Added (backends)
- **MacPorts** (`macports`, macOS) — install/remove/list/search/upgrade via `port`.
- **pkgsrc** (`pkgin`) — cross-platform pkgsrc binary packages; gated on the `pkgin` binary.
- **.NET global tools** (`dotnet`) — `dotnet tool` install/list/search/upgrade, with
  `--version` pinning. Cross-platform; the plain (project-scoped) NuGet surface is out of
  scope by design.
- **Conda** (`conda`) — environment-scoped install/remove/list/search/upgrade over
  `--json`. The target environment is configurable via `backend_settings.conda.env`
  (default `base`).
- **PowerShell Gallery** (`psresource`, Windows) — modules via the modern PSResourceGet
  cmdlets (`Install-PSResource`, …), invoked through PowerShell with strict name
  validation to foreclose command injection.
- **uv** (`uv`) — Astral's fast Python application installer (`uv tool`), the modern
  successor to pipx. Install/remove/list/upgrade with `name==version` pinning.
  Cross-platform; gated on the `uv` binary. Project/venv-scoped `uv pip` is out of scope
  by design, mirroring the `dotnet` global-tools decision.
- **XBPS** (`xbps`, Linux) — Void Linux's package system. Full install/remove/list/
  search/upgrade + orphan cleanup across its `xbps-install`/`xbps-remove`/`xbps-query`
  binaries. Rolling, so no version pinning (matches pacman).
- **AUR helpers** (`yay`, `paru`, Linux) — Arch's user repository, as first-class
  backends (`yay:pkg` / `paru:pkg`). They speak pacman's syntax, so they reuse the pacman
  parsers but label packages with their own backend. Run unprivileged (never as root).

### Added (composition)
- **Module/group exclusion** — `@module:dev -vim` (or `-apt:vim`) includes a module but
  drops a package from *its* contribution. Exclusions are scoped to that module's whole
  recursive expansion and propagate through `requires`, but are **not** global: if another
  source independently asks for the package, it still gets installed. Lets you reuse a
  shared module and trim it per machine instead of forking it.

### Added (generations & retention)
- **Generations** — after each change, LiNix records a generation capturing the realized
  state (exact installed set) *and* a frozen copy of the manifests that produced it. Manage
  them with `linix generation list|pin|unpin|rollback`, or the shorthand `linix rollback
  <id>`.
- **Rollback that realizes the change** — `linix rollback <id>` drives the backends to make
  the system match the generation (installing, removing, and *downgrading to the recorded
  version* where the backend supports pinning), through the normal transaction engine so it
  keeps snapshot + WAL safety and records itself as a new generation. A full rollback also
  restores the manifests (backing up any drifted file first) so a later `sync` won't undo
  it. **Scoped rollback** — `--package <name|backend:name>` and/or the global `--backend`
  roll back just one package or one backend, leaving everything else untouched.
- **Manifest archive** — an independent history of the manifest files themselves
  (`[retention.manifests]`), captured after each change with de-duplication so an unchanged
  manifest never spawns a redundant entry.
- **Declarative retention** (`[retention.generations|snapshots|manifests]`) — each history
  keeps its own policy: `keep_last`, `keep_days`, and a `keep` pin list. Rules combine as a
  union (kept if it matches any), the most-recent entry is always kept, and an empty policy
  keeps everything. All three are enforced automatically after each sync; snapshot
  retention only ever deletes LiNix-created snapshots.
- **Snapshot ↔ generation pairing** — restoring a filesystem snapshot via `linix undo` also
  restores the generation that was current at that snapshot's time, so the manifests and
  state record match the system you rolled back to.

### Added (config-file management)
- **Backup-before-overwrite for the `link` backend.** Before LiNix replaces a file that
  was already on disk (a real file it didn't create), it saves the original once to
  `<target>.linix-backup` and logs where it went — so adopting a machine can never
  silently destroy a config file you'd written. Symlinks and directories are skipped; the
  original is preserved even across repeated syncs and later hand-edits. Honors `--dry-run`.
- **Inline config-file contents** (`[managed_files]` in config) — declare a file's body
  directly (`"~/.gitconfig" = '''…'''`) instead of pointing `link` at a separate source
  file. Each entry is materialized as a managed `link` file that installs, self-heals on
  drift, shows up in `status`, and is removed by `prune` when you delete the entry — the
  same lifecycle as any other managed item.

### Added (multi-machine)
- **Per-host backend allow-lists** (`[hostname_backends]`) — a machine can manage only a
  subset of backends. A non-empty per-host entry overrides the global `enabled_backends`;
  manifest entries for non-managed backends are skipped on that host (not an error), and
  search / system-scope prune respect the same set. Empty = all backends (unchanged
  default). Complements the existing `[hostname_packages]`.

### Added (interface)
- **NuShell shell completions** — `linix completions nushell` now emits a NuShell
  completion script, alongside Bash, Zsh, Fish, PowerShell, and Elvish.

### Fixed
- **Backends no longer read as OFFLINE on minimal images that lack `which`.** Availability
  was checked by spawning the external `which`/`where` program, absent from minimal
  fedora/arch/alpine — so every backend showed OFFLINE and `remove` (which only searches
  *available* backends) found nothing, even for installed packages. Now resolved in-process
  via the `which` crate (PATH/PATHEXT), no external program needed.
- **Dropped the OpenSSL dependency** (switched `reqwest` to rustls). OpenSSL made static
  musl/Alpine builds fail to link and required `openssl-dev` on every distro; rustls is
  pure-Rust and matches what the email path (lettre) already uses.
- **apt (and any manager whose query tool is a separate binary) can now list installed
  packages.** The generic backend ran the list command as `apt dpkg-query …` (backend name
  + args), but `dpkg-query` is its own program, not an apt subcommand — so apt's
  `list`/`info`/`status`/`migrate`/`remove` silently saw *zero* installed packages. Added a
  `list_binary` option to the generic config; apt now lists via `dpkg-query`. Verified on
  real Linux (0 → 609 packages visible). Found by the container harness.
- **No more "Aborted (core dumped)" when output is piped to `head`/`less`.** A closed
  output pipe made `println!` panic with EPIPE, and `panic = "abort"` turned it into a core
  dump. A panic hook now exits quietly on a broken pipe (SIGPIPE stays ignored for sockets,
  so network I/O is unaffected).
- **`linix remove backend:pkg` now actually removes the package** (affected *every*
  backend). `remove` passed the whole `"backend:name"` string to backends' `info()`/
  `remove()`, which expect the *bare* name — so it silently found nothing and reported
  "not under active management", while `install backend:pkg` worked. `remove` now parses
  the `backend:name[@opts]` syntax the same way `install` does (a recognized prefix scopes
  the removal to that backend; the bare name is what's queried/removed). Found by the
  container harness (apt) and confirmed on Windows (uv, scoop); regression-tested via
  `split_removal_target`.
- **`sync` no longer hangs in non-interactive shells.** It unconditionally launched the
  ratatui confirmation TUI (unless `--yes`/`--json`), which blocks forever with no TTY
  (CI, pipes, scripts) — and it did so even under `--dry-run`. Now: `--dry-run` prints the
  plan (or JSON) and exits without prompting; the interactive TUI runs only with a real
  terminal; and a non-interactive run without `--yes` errors cleanly ("pass --yes to
  proceed, or --dry-run to preview") instead of hanging or silently applying. Found while
  building the container integration harness.
- **Windows scoop backend now actually works** (three stacked bugs, all found by running
  the real binary against a live Windows machine):
  1. *Couldn't launch.* `scoop` ships as `scoop.ps1`; `where`/`which` find it (so it showed
     `[READY]` in `doctor`) but `CreateProcess` can't launch a `.ps1`, so every scoop op
     failed "program not found". The real-execution path now routes `.ps1` (and `.cmd`/
     `.bat`) shims through their interpreter.
  2. *No output captured.* scoop emits PowerShell objects that only render through the
     formatter; `-File` and a trailing `; exit` both drop the table when stdout is piped.
     It's now invoked by bare name through `-Command`, captured via `Out-String`, then
     exits with the tool's code — args single-quoted (no injection surface).
  3. *Parser out of date.* `scoop search`/`list` parsers expected the old `name (version)`
     format and dropped the modern table (`Name Version Source Binaries`); they now parse
     the table and skip its header/separator.
- **Per-package `before_install` / `after_install` hooks now actually fire.** These were
  documented in the example config but only `before_sync` / `after_sync` were ever
  executed. Hooks now run per package at the moment it installs — interleaved with the
  parallel transaction engine, keyed by package name with a `*` wildcard fallback. A
  failing `before_install` aborts that package's install (its prerequisite was not met);
  an `after_install` failure is logged without undoing a healthy install.

### Added (extensibility)
- **The onboarder** — teach LiNix any CLI package manager entirely from
  `~/.config/linix/custom_backends.toml`, no source changes. Each `[[backend]]` supplies
  the argv templates plus a declarative `parser` (`lines` | `columns` | `json` | `regex`)
  interpreted at runtime. Custom backends load last and never override a built-in
  (collisions are skipped with a warning).

## [6.0.0] — 2026-07-02

Class-defining cross-ecosystem features that are only possible because LiNix sits above
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
- **Scoped `upgrade` is now non-destructive.** `linix upgrade --module X` / `--group X` /
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
- **`linix prune`** — remove packages installed but no longer in your manifests (with a
  confirmation prompt; honors `confirm_destructive`/`--yes`).
- **`linix status`** (alias **`diff`**) — read-only report of what `sync` would install,
  what drift `prune` would remove, and what's installed-but-unmanaged. `--json` supported.
- **Per-backend version pinning for reproducible/locked installs.** Each backend now honors
  `options["version"]` in its native syntax: `apt`/`apk`/`zypper` `name=ver`, `dnf`
  `name-ver`, `pip`/`pipx` `name==ver`, `npm`/`pnpm`/`yarn`/`bun`/`mise` `name@ver`,
  `cargo`/`gem`/`winget`/`choco` via flags, `vscode` `ext@ver`. `brew` is best-effort
  (versioned formulae); `pacman`/`snap`/`flatpak`/`nix`/`mas` don't support fixed-version
  pins (rolling/channel/flake/store models) and install latest.
- **`linix lock`** — record the installed version of every managed package to
  `locks.json`, so `sync --locked` reproduces those exact versions on another machine
  ("reproducible inputs"; see README for the honest limits vs. Nix).

### Added (CLI)
- Previously-silent subcommands now work: **`teleport`**, **`unmanaged`**, **`update`**,
  **`shim`**. `orphans` now *lists* drift non-destructively (distinct from `clean`).
- **`linix config init | path | show`** to scaffold/inspect the application config.
- **`linix completions <shell>`** to emit a shell completion script (the generator
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
