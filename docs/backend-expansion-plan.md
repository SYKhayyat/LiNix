# Internal Plan — Package-Manager Backend Expansion

Status: in progress. Owner: platform. Goal: add the full set of package managers
requested (Composer, Go, Portage/emerge, Guix, eopkg, slackpkg, opam, cabal/stack,
luarocks, pub, nimble, hex/mix, helm, krew, asdf, pixi, spack) and finish the
half-wired pnpm/yarn helpers.

## Background

LiNix backends come in two flavors:

1. **Generic, config-driven** — a `ManagerConfig` (argv templates) + an `OutputParser`
   registered via a small `register_*` helper in `src/backends/registry.rs`. Used for
   managers whose CLI has clean install/remove/list/search verbs (apt, gem, pip, bun…).
2. **Dedicated** — a hand-written module in `src/backends/` (cargo, github, npm…) for
   managers that need real logic (filesystem enumeration, API calls, subcommand binaries).

The registry is fully dynamic (no hardcoded backend allowlist), so a registered backend
is immediately usable as `linix install <name>:<pkg>`, in manifests, migrate, teleport, etc.

## Implementation strategy per manager

Two managers requested are the same system under two names, and are implemented once:
- **portage == emerge** — Gentoo's package system (`portage`) is driven by the `emerge`
  CLI. Implemented as the `emerge` backend.
- **hex == mix** — Elixir's `hex` packages are managed through the `mix` CLI. Implemented
  as the `mix` backend.

### Generic backends (new `register_*` in registry.rs, parsers in `parsers/ecosystem.rs`)

| Backend | Binary | install | remove | list | search | version pin | OS |
|---|---|---|---|---|---|---|---|
| composer | composer | `global require` | `global remove` | `global show --format=json` | `global search --format=json` | `{name}:{version}` | all |
| opam | opam | `install -y` | `remove -y` | `list --installed --short` | `search --short` | `{name}.{version}` | all |
| luarocks | luarocks | `install` | `remove` | `list --porcelain` | `search --porcelain` | Flag `{version}` | all |
| nimble | nimble | `install -y` | `uninstall -y` | `list --installed` | — | `{name}@{version}` | all |
| pixi | pixi | `global install` | `global remove` | `global list` | `search` | `{name}={version}` | all |
| spack | spack | `install` | `uninstall -y` | `find --format {name} {version}` | `list` | `{name}@{version}` | all |
| mix | mix | `archive.install hex --force` | `archive.uninstall` | `archive` | — | — | all |
| helm | helm | `plugin install` | `plugin uninstall` | `plugin list` | — | — | all |
| cabal | cabal | `install` | *(unsupported)* | `list --installed --simple-output` | `list --simple-output` | `{name}-{version}` | all |
| stack | stack | `install` | *(unsupported)* | — | — | `{name}-{version}` | all |
| asdf | asdf | `install` | `uninstall` | `list` | — | Flag `{version}` | all |
| guix | guix | `install` | `remove` | `package -I` | `search` | `{name}@{version}` | linux |
| emerge | emerge | `--ask=n --quiet` | `--unmerge --ask=n` | `qlist -I` | `--search` | — | linux |
| eopkg | eopkg | `install -y` | `remove -y` | `list-installed` | `search` | — | linux |
| slackpkg | slackpkg | `… install` | `… remove` | `ls /var/log/packages` | `search` | — | linux |

Managers without a real removal verb (cabal, stack) rely on a new generic behavior:
`GenericInstallable::remove` returns `Error::Unsupported` when `remove_args` is empty,
instead of running a wrong command. Managers without search/upgrade simply don't attach
that capability.

### Dedicated backends

- **go** (`src/backends/go.rs`) — `go install pkg@version`; list by enumerating the Go bin
  dir (`GOBIN` → `go env GOPATH`/bin → `~/go/bin`) and reading each binary's module path
  via `go version -m`; remove deletes the installed binary (Go has no uninstaller); no
  CLI search (pkg.go.dev is web-only); upgrade reinstalls `@latest`.
- **pub** (`src/backends/pubdart.rs`, id `pub`) — Dart/Flutter, binary is `dart`:
  `dart pub global activate/deactivate/list`. Runtime-gated on the `dart` binary.
- **krew** (`src/backends/krew.rs`, id `krew`) — kubectl plugins, binary is `kubectl`:
  `kubectl krew install/uninstall/list/search`. Runtime-gated on `kubectl`.

## Dead-code / half-implementation fixes (from the audit)

- **pnpm** `info()` built `"{root}/node_modules/{name}"` from `pnpm root -g`, which already
  returns the global `node_modules` dir → double `node_modules` bug. Fixed to the real
  path, plus a `bin_path` property from `pnpm bin -g`. Removes the dead `get_global_store`.
- **yarn** — wire the previously-dead `get_global_bin` (`yarn global bin`) into `info()`
  to record `bin_path`, matching the cargo backend. Removes the `#[allow(dead_code)]`.

## Verification

**Hermetic (any host, CI):** `cargo fmt` + `cargo check` + `cargo test`. Every new parser
has unit tests; `registry::tests::registry_capability_matrix` asserts the exact capability
set of every new backend; the Linux-only backends are type-checked cross-platform.

**Real-world integration harness** (the release-readiness sweep — you run these):

- `docker/integration/run-in-container.sh` gained a **PLAN-SMOKE** section that enumerates
  **every `[READY]` backend from `doctor`** (not a hand-maintained list) and asserts each
  one's dry-run install emits a valid JSON plan (argv construction + planner wiring, no
  compile/network), plus best-effort `list`/`search --json`. HARD for package-manager
  backends, tolerant for system/special ones (service/github/web/link/appimage/snap/btrfs).
  Nothing registered is silently untested, and future backends are picked up automatically.
  A `SMOKE_ONLY=1` mode runs discovery + plan-smoke + read-only and skips the heavy
  real-mutation lifecycle (for source-building distros like Gentoo). LuaRocks and pixi also
  get the full real install→list→remove lifecycle in the multi-backend sweep.
- `docker/integration/Dockerfile.tools` — Ubuntu base with a broad set of the cross-platform
  managers installed, wired into `run.sh` (in the default matrix) so plan-smoke exercises
  composer/go/opam/luarocks/nimble/cabal/stack/mix/helm/krew/asdf/pixi/spack against real
  binaries.
- `docker/integration/Dockerfile.gentoo` — opt-in (`DISTROS="gentoo"`), runs emerge on real
  Portage in SMOKE_ONLY mode (multi-stage: binary built on Ubuntu, copied into `gentoo/stage3`).
- `scripts/integration-windows.sh` mirrors the plan-smoke section and extends the scoop
  bootstrap (go, composer, nim, dart, kubectl, helm, luarocks, pixi) so Windows coverage
  tracks the Linux images.
- Guix / eopkg / slackpkg are distro-locked: the plan-smoke rows light up on a Guix System /
  Solus / Slackware host; their registration + parsing are covered by the hermetic Rust tests.

Backends are runtime-gated by binary presence, so each is inert (and auto-skipped) where the
tool isn't installed.
