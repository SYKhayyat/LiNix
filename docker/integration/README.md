# LiNix integration harness (real package managers)

The unit/integration test suite in `tests/` is **hermetic** — it mocks command execution,
so it proves the *logic* but never touches a real package manager. This harness fills that
gap: it runs the real `linix` binary against **real** apt / dnf / pacman / apk (and, on the
host, scoop / winget / brew), doing a full **search → install → verify → remove → verify**
cycle in disposable containers. This is where real-world bugs live (output-format drift,
shim quirks, exit-code edges).

## Linux (containers)

Requires Docker. From the repo root:

```bash
./docker/integration/run.sh              # ubuntu(apt), fedora(dnf), arch(pacman), alpine(apk), tools
./docker/integration/run.sh htop         # override the test package
DISTROS="ubuntu arch" ./docker/integration/run.sh   # a subset
DISTROS="gentoo" ./docker/integration/run.sh        # opt-in emerge smoke (large base)
```

The default set already includes the `tools` image (the broad ecosystem-managers sweep), so
a plain `run.sh` covers the four native distros **and** the expansion backends — no separate
command needed. `gentoo` stays opt-in because it pulls a large base image.

Each distro has a Dockerfile that installs a Rust toolchain (via that distro's own package
manager — which also warms its package DB), builds `linix`, and runs
`run-in-container.sh <backend> <package>`. A non-zero exit means that backend's real
install/remove path failed.

### Real-by-default coverage (every feasible backend + every feature)

The guiding rule: **everything that can physically run in the image gets a REAL
install→list→remove lifecycle — even if it compiles from source and takes minutes.** Only the
genuinely-impossible-here set is plan-smoked, and each such case is named explicitly. Three
mechanisms enforce this:

1. **REAL multi-backend lifecycle (section 11)** — a real install → list(parser) → manifest
   coherence → remove → verify-gone → coherent cycle for every feasible backend: the language
   managers (npm/pnpm/yarn/bun/pipx/uv/gem/pip/luarocks/pixi), the ecosystem managers
   (composer, dotnet, pub, krew, mix, conda, **nix**), the source-compilers (**cargo, go,
   opam, nimble, spack** — real builds; `FAST=1` downgrades these to plan-smoke), the
   no-uninstall-verb ones (**cabal** — install+list are HARD, then `remove` must report a
   graceful *unsupported*, verifying the designed contract), and the special-identifier ones
   (**github** does a real release download→symlink→remove; **link** creates and deletes a
   real symlink). Install failure is *soft* (ecosystem/network variance is not a core bug);
   **everything after a successful install is HARD** — that is exactly what caught the pixi
   `global remove` vs `global uninstall` bug that a dry-run plan could never see.

2. **FEATURE COVERAGE (section 12)** — every `linix` subcommand is exercised at least once:
   completions (all six shells), heal, clean, unmanaged, orphans, audit, sbom, why, policy,
   upgrade (+ `--canary --test`), repo add/list/remove, migrate, teleport, module
   list/create/show, snapshot list/prune, generation pin/unpin, rollback, lease set/list,
   schedule add/list/remove, run, shim — plus the install/remove/sync/prune/lock/profile/
   status surface already driven by sections 3–10.

3. **PLAN-SMOKE (section 13)** — reserved for the *only* things that cannot run a real
   lifecycle in a plain container: distro-native managers on the wrong distro (emerge/guix/
   eopkg/slackpkg/zypper/xbps/yay/paru), the Windows/macOS-native backends, and daemon/
   filesystem-gated ones (snap needs snapd, service needs systemd, btrfs needs a btrfs FS).
   Each still gets its argv/planner wiring proven via a dry-run JSON plan.

4. **COVERAGE AUDIT (section 14)** — enumerates **every `[READY]` backend from `doctor`** and
   HARD-fails if any went untouched by a real lifecycle *or* a plan-smoke, and likewise
   HARD-fails on any `linix` subcommand that was never exercised (outside a documented
   interactive/remote-SSH exempt set: `shell`, `undo`, `bisect`, `clone`, `fleet`). So nothing
   registered is silently untested, and **a backend or command added in the future fails the
   audit until it's covered.**

Where they run:

- **Every image** runs the real lifecycle for whatever managers it ships, full feature
  coverage, and the audit.
- **`tools` image** (in the default set) — an Ubuntu base with the ecosystem managers
  installed **and initialized** (opam switch, cabal/nimble index refresh, spack compilers,
  conda channels, flathub remote, krew index, nix made READY, and the go/dotnet/pub bin dirs
  on PATH) so each runs a genuine build/install, not a dry run. A full real run compiles from
  source and can take 20–40 min; use `FAST=1` for a quick pass.
- **`gentoo` image** (opt-in; `DISTROS="gentoo"`) — real Portage in **SMOKE_ONLY** mode
  (baked in): discovery + plan-smoke + read-only, no source builds.
- **Guix / eopkg / slackpkg** are distro-locked — run the harness on a Guix System / Solus /
  Slackware host and their real rows light up automatically; elsewhere they plan-smoke. Their
  registration and output parsing are also covered by the hermetic Rust tests (`cargo test`).

Toggles: `FAST=1` downgrades the heaviest source-compilers to plan-smoke; `SMOKE_ONLY=1` skips
real mutation entirely (discovery + plan-smoke + read-only). Both are forwarded into the
container by `run.sh` (`FAST=1 ./docker/integration/run.sh`).

### One-shot release gate

`scripts/release-check.sh` (Linux/macOS) and `scripts/release-check.ps1` (Windows) run the
hermetic gates (`cargo fmt`/`clippy`/`test`/`build`) **and** the full integration matrix, then
print a single **GO / NO-GO** verdict with a non-zero exit on any hard failure:

```bash
./scripts/release-check.sh                 # hermetic gates + Docker matrix (incl. gentoo)
FAST=1 ./scripts/release-check.sh          # quicker: heavy compiles downgraded
SKIP_DOCKER=1 ./scripts/release-check.sh   # hermetic gates only
```
```powershell
./scripts/release-check.ps1                # hermetic gates + native Windows sweep
./scripts/release-check.ps1 -SkipIntegration
```

### Installing Docker (if you don't have it)

Inside WSL Ubuntu (one command; asks for your password):

```bash
curl -fsSL https://get.docker.com | sudo sh && sudo usermod -aG docker "$USER" && sudo service docker start
```

Then open a **new** shell and run the harness. (On Windows without WSL, install Docker
Desktop and run the same `run.sh` from a bash shell.)

## Windows (native — not containerizable here)

Windows can't run in a Linux container, and Windows containers need a Windows host + heavy
base images. Since development already happens on Windows, test the Windows-specific
backends natively:

```bash
cargo build
scripts/integration-windows.sh scoop busybox
```

## macOS — why it's not in the matrix

macOS **cannot** be run in Docker on non-Apple hardware: containers share the host's Linux
kernel, macOS needs Apple's Darwin/XNU kernel, and Apple's license forbids virtualizing
macOS off Apple hardware. There is no legitimate macOS container image. So brew testing
needs a **real Mac** or a **macOS CI runner**:

- On any Mac: `cargo build && ./scripts/integration-windows.sh brew wget` (the script is
  backend-agnostic despite the name — or copy `run-in-container.sh` and run
  `run-in-container.sh brew wget`).
- In CI: GitHub Actions provides `macos-latest` runners; run the same steps there.

## What each run proves (and its limits)

- **Proves:** the backend spawns, its real search/list output parses, and a real package
  can be installed and removed end-to-end.
- **Doesn't prove:** long-tail package edge cases, version-pin fidelity on every backend,
  or dependency-resolution corners. Extend `run-in-container.sh` to cover more.
