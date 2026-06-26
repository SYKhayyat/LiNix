# Changelog

All notable changes to LiNix are documented here.

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
