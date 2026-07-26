# LiNix — Code Review & TODO

> Review date: 2026-06-26 · Reviewer: Claude (Opus 4.8)
> **Implementation pass: 2026-06-26** on branch `v5-review-fixes` (off `v5`).
> Status verified on Windows: `cargo build` ✓ · `cargo clippy --all-targets -D warnings`
> **0 warnings** ✓ · `cargo test` **16 binaries, lib 26/26, 0 failures** ✓ · version → **5.0.0**.
>
> **Hard limit:** this Windows host has no Linux toolchain, so the
> `#[cfg(target_os = "linux")]` block (apt/apk/zypper/pacman/dnf) and macOS brew/mas were
> **written but not compiled here**. They are the deferred WSL/CI gate (see §6).

Legend: `[x]` done & verified on Windows · `[~]` written, pending Linux/macOS gate ·
`[ ]` **intentionally deferred** (out of scope this pass — follow-up).

---

## 1. Errors / bugs that must be fixed

- [x] **P0 — Scoped `upgrade` is destructive (data-loss).** Fixed in
  `planner.rs::plan()`: the entire removal loop is now guarded by
  `if matches!(scope, ScopedFilter::None)`, so a scoped upgrade emits **no** `Remove`
  nodes. Regression test `scoped_plan_is_non_destructive`.
- [x] **P0 — `nix` multi-package removal removes the wrong packages.** `nix.rs::remove()`
  now removes highest-index-first (Nix renumbers between removals) with a name-based
  fallback; hazard documented in-code.
- [x] **P1 — `cargo list_installed` emits empty-named packages.** `cargo.rs` skips indented
  binary/continuation lines; only column-0 crate headers are parsed.
- [x] **P1 — `yarn` scoped-package parsing drops the name.** New `split_name_version`
  (`rsplit_once('@')`, scope-aware) + test `parses_plain_and_scoped_names`.
- [x] **P1 — `is_protected` substring matching → false positives.** Now exact
  (case-insensitive) match only. Test `is_protected_is_exact_not_substring`.
- [x] **P1 — `vscode check_health` always Ok.** Override removed; uses the default
  `BackendCore::check_health` (Critical when `code` missing).
- [x] **P1 — `flatpak update` can block on prompt.** Now passes `-y --noninteractive`.
- [x] **P2 — `handle_lease` panic.** `from_timestamp(...).unwrap()` replaced with a guarded
  match; bad timestamps print `<invalid expiry>` instead of panicking.
- [x] **P2 — `from_file` TOCTOU.** Reads directly and maps `NotFound` → defaults; no
  existence pre-check.

---

## 2. Partially implemented / missing features

### 2a. Silently-dropped CLI commands (P0)
- [x] `Update` → refreshes metadata across backends (`app.update()`).
- [x] `Teleport` → dispatches `app/teleport.rs::Teleporter`.
- [x] `Unmanaged` → lists installed-but-unmanaged packages.
- [x] `Shim` → wired to `app.create_shim()`.
- [x] Silent `_ => Ok(())` catch-all removed; dispatch is now exhaustive.

### 2b. `--json` flags that produce no output (P1)
- [x] `install --json --dry-run` and `remove --json --dry-run` emit a machine-readable plan.

### 2c. `Orphans` aliased to `Clean` (P2)
- [x] `orphans` now **lists** drift non-destructively (dry-run of removals); `clean` keeps
  the deep-clean behavior.

### 2d. Missing trait implementations
- [x] **`Searchable` ×11:** brew, cargo, npm, pnpm, yarn, mise, snap, flatpak, nix, emacs,
  pip. (npm/pnpm/yarn share an npm-registry HTTP helper `backends/node_registry.rs`;
  pip uses exact-name PyPI lookup `backends/pip_search.rs`.) All wired in `registry.rs`.
- [x] **`RepoManager` — winget** (`source add/remove/list` via `GenericRepoManager`).
- [~] **`RepoManager` — dnf** (`DnfRepoManager`, `config-manager`) — impl compiled on
  Windows; the registry-wiring line is inside the linux `cfg` block → **gate**.
- [~] **`RepoManager` — pacman** (`PacmanRepoManager`, drop-in `/etc/pacman.d/` + `Include`)
  — same: impl compiled, registry wiring linux-gated → **gate**.
- [x] **`Upgradable` — vscode** (`VscodeUpgradable`, per-extension `--force` reinstall).
- [x] **`Upgradable` — emacs** (`EmacsUpgradable`, `package-upgrade-all` + fallback).

### 2e. `upgrade --module` / `--group` (P0)
- [x] Fixed via the non-destructive guard + exact-segment scope match (see §1).

---

## 3. Architectural problems / things to reconsider

- [x] **P0 — `max_parallel` was dead for the work path.** `sync/mod.rs` now sets
  `tx_config.max_concurrent = config.max_parallel.max(1)`.
- [x] **P1 — Scope filter used `contains` substring.** Replaced with exact-segment match
  (`source_matches_scope`), `;`-multi-source aware.
- [x] **P1 — First-write-wins source tagging hides packages from scoped upgrade.** Fixed:
  `resolver.rs` now merges all origins into a `;`-joined `__source` tag (helper
  `merge_source_tag`, with a unit test); the planner already matches `;`-split segments.
- [ ] **P1 — `info()` is O(n) full-list scan** (generic/dnf/pacman). *Deferred* — left as-is;
  acceptable for single lookups, documented here as a known cost.
- [x] **P1 — Command failures swallowed into empty results** (`search.rs`). Fixed: failed
  backends are collected and surfaced ("N backend(s) failed and were skipped: …") instead
  of looking identical to "0 results".
- [x] **P2 — `create_default_registry` is one 590-line function (OCP).** Refactored: each
  specialized backend module owns a `register(reg, exec, cfg)`; generic backends use
  `register_*` helpers; `create_default_registry` is now a short orchestrator. Backend
  count unchanged (24 on Windows), verified live.
- [ ] **P2 — `needs_root` handling is ad hoc per call site.** *Deferred* — not centralized.

---

## 4. SOLID / anti-patterns

- [ ] **DRY — duplicated `info()` "list then find".** *Deferred* — no shared helper added.
- [x] **OCP — registry monolith.** Done — per-backend `register()` + orchestrator (see §3).
- [x] **ISP — good.** Kept; every backend that can search/upgrade/manage-repos now registers
  the capability.
- [ ] **LSP — `GenericUpgradable::clean_orphans` silent `Ok(())`.** *Deferred* — no
  `Unsupported` error variant introduced.
- [ ] **Untyped stringly `backend_settings`.** *Deferred* — typed per-backend structs not
  added (documented as future in `examples/config.toml`).
- [x] **Magic numbers / hardcoded retention.** `nix` GC age now from config (`nix_gc_age`);
  transaction concurrency now from `max_parallel`.
- [x] **Injection surface.** emacs package names validated (`validate_symbol`) + search terms
  escaped; pacman/dnf repo name/url validated; `GenericRepoManager` rejects shell
  metacharacters in repo name/url.

---

## 5. Warnings & hygiene

- [x] **All clippy warnings cleared** across lib, bins, **and** integration tests
  (`cargo clippy --all-targets -- -D warnings` → 0 on Windows). Mix of auto-fix +
  manual (`next_back`, `first`, `from_ref`, `Default for BackendRegistry`,
  `is_empty` on `SmartCache`, `matches!`, module-inception allow, params-struct allows).
- [~] **Re-run clippy on Linux & macOS.** The linux `cfg` block was not linted here → CI
  matrix does this (see §6).

---

## 6. Cross-platform correctness — **the deferred gate**

- [~] **Build & test on all three OSes.** Only Windows verified. CI matrix added
  (`.github/workflows/ci.yml`: ubuntu/windows/macos × build + `clippy -D warnings` + test).
  Triggers fixed (`master`/`v5`), actions bumped off EOL versions. **Run on WSL after
  reboot; do not tag until green.**
- [x] **`npm` info path** branches on `cfg!(windows)` (`node_modules` vs `lib/node_modules`),
  built with `PathBuf`.
- [x] **`cargo` info path** uses `PathBuf` + `.exe` on Windows.
- [x] **`mise` data dir** is cross-platform (`MISE_DATA_DIR` → platform default), `PathBuf`.
- [x] **`yarn` Berry** — search resolves via npm registry (documented limitation).
- [ ] **`pnpm`/`yarn` fallback dirs** — *partially* addressed; primary queries OK, some
  fallback paths still Linux-shaped. Low priority.
- [x] **`winget` list/search/repo parsing (found during smoke testing).** Rewrote
  `parsers/windows.rs` winget parsing to be column-position based (real `winget list` is
  fixed-width with space-containing names + a bare-`\r` spinner header); fixed
  `GenericRepoManager::list_repos` to skip header/separator rows. Verified live on Windows;
  replaced the misleading unit fixture with realistic column-aligned fixtures (+3 tests).

---

## 7. Feature additions

- [x] **Application config file.** `max_parallel` wired into the engine; `linix config
  init|path|show` added; new options `network_timeout_secs`, `nix_gc_age`,
  `confirm_destructive` (all wired); every field documented in `examples/config.toml`.
- [x] **`is_empty()` companion** for `SmartCache::len`.
- [ ] **Backend health summary** in search/sync output. *Deferred* (ties to §3 swallowed
  errors).
- [x] **Dry-run JSON parity** for install/remove/upgrade.
- [x] **Rich output (found during review):** `info` now shows version/description/install
  path/properties/dependencies; `search`/`list` show versions; `search --json` added;
  `completions <shell>` command wired (the generator existed but had no command).
- [ ] **AUR/`yay` awareness for pacman.** *Deferred.* (pip PyPI search **done** in §2d.)

---

## 8. Repo hygiene

- [x] Removed committed cruft: `dump_files.ps1` + `project_dump.txt` (root/src/tests, 6 files)
  and the untracked `tests/New Text Document.txt`.
- [x] `.gitignore`: added `.idea/`, `.vscode/`, and the dump artifacts.
- [x] **Docs:** new `CHANGELOG.md`; updated `readme.md`; version bumped to `5.0.0` in
  `Cargo.toml` + `args.rs` + runtime strings.
- [~] **Tag the release.** Held until the Linux/macOS CI/WSL gate is green.

---

## Remaining work (explicitly deferred)

1. **WSL/CI gate (required before tag):** build + clippy + test the linux backends and the
   dnf/pacman RepoManager registry wiring; verify macOS brew/mas.
2. **Architectural follow-ups (optional):** multi-source resolver tagging; registry OCP
   refactor; targeted `info()` queries; `Unsupported` error for unsupported `clean_orphans`;
   typed `backend_settings`; backend-health summary; centralized sudo handling; AUR/`yay`.
3. ~~**Formatting:** repo is not `rustfmt`-clean (pre-existing); the CI fmt step is
   non-blocking until a dedicated format pass is run.~~ **DONE 2026-07-26.** `cargo fmt`
   swept the tree; `continue-on-error` is off in CI and `release-check.sh`'s fmt gate is
   HARD, so it cannot drift back unnoticed.
