# Changelog

All notable changes to LiNix are documented here.

## [Unreleased] — v7, the declarative rewrite

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
- **History is git.** `linix git init` makes the config directory a repo, every sync commits,
  and `linix rollback <commit>` restores those manifests and converges the machine. There is no
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
- **`linix path` and `linix edit` find your files for you**, so neither you nor your scripts
  have to hard-code `~/.config/linix`. `linix path --set DIR` records the repo location in
  LiNix's own settings file — the one file that lives outside the repo, because a key inside
  the repo saying where the repo is cannot be read before you know where the repo is. That
  file holds exactly one key and the parser refuses any other, naming `preferences.toml` as
  where behaviour settings belong. `--config-dir` overrides it for one run; the order is
  `--config-dir`, `$LINIX_CONFIG_DIR`, the settings file, the default, and
  `linix path --explain` says which one won.

### Safety

- **One guard, every removal path.** Removal count ceilings, the protected list, and the OS's
  own essential flags are enforced in a single function that every deleting command calls.
  `--allow-mass-removal` answers the count and nothing else; protection is a refusal, not a
  confirmation, so nothing overrides it.
- **`remove-orphans` previews, guards, then asks.** It lists what each manager considers
  orphaned, puts the whole set through the guard, and removes exactly what it showed.
- **`export` never silently overwrites.** A taken filename is written beside the real file
  (`package.linix.json`); `--force` overwrites deliberately.
- **File-backed backends no longer report a removal that failed.** If the binary could not be
  deleted, the package stays recorded rather than becoming drift nothing can see.
- **A crash aged out of the write-ahead log is still healable**, so an interrupted run left
  unattended for hours is still repaired rather than dropped.
- **`sync`, `rollback` and `remove-orphans` refuse to apply unconfirmed** in a non-interactive
  shell without `--yes`.

### Removed

Each of these was a second way to do something the model already does. Deleted, not deprecated.

- **`teleport`** — a backend move is "rewrite the prefix, sync". It also built its own
  transaction graph and executed it **without calling the guard**, so it could remove a
  protected package that every other path refused to touch.
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

- **`linix --help` panicked on every debug build.** `status` carried an alias `diff` that
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
