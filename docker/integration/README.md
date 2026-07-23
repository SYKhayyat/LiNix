# LiNix integration harness (real package managers)

The unit/integration test suite in `tests/` is **hermetic** — it mocks command execution,
so it proves the *logic* but never touches a real package manager. This harness fills that
gap: it runs the real `linix` binary against **real** apt / dnf / pacman / apk / emerge (and,
on the host, scoop / winget / brew), doing a full **install → list → verify → remove →
verify** cycle in disposable containers. This is where real-world bugs live (output-format
drift, shim quirks, exit-code edges).

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

## What the harness asserts

Every check either passes or fails the whole run. A short list of genuinely
network/ecosystem-optional checks is reported as `soft` and never fails the run; each one
says by name what it did not exercise.

| § | What it proves |
|---|---|
| 1–3 | `init` scaffolds the II.1 repo; the read-only verbs run; `--dry-run` really installs nothing |
| 4 | `purge-unmanaged` is refused on an **unadopted** machine — the only state in which the ratio rule fires — and the refusal names *that* rule |
| 5–7 | Real install → `list` → PATH for the native manager; a second sync is a no-op; a bad name fails and leaves the model parseable |
| 8 | `adopt` takes the **manual** set: the count is compared against the manager's own user-chosen list and against every installed package, not printed |
| 9–10 | The guard keeps a protected package; `purge-unmanaged` after adopt is refused by the *protected set*, a different rule than §4's; real uninstall → gone from PATH |
| 11–12 | git-backed manifest history, `diff`, `rollback`; `rebuild` repairs and writes **no** commit (K14) |
| 13 | Backend chains and the per-host lock file; `unlock`; a pin to a manager this host lacks is not silent |
| 13b | A manager that could not answer is not one that said no (V.7c) |
| **14** | **A real install → list → PATH → remove → gone cycle for every other manager the image ships** |
| **15** | **A plan-smoke for every registered backend the image cannot run**, so its argv/planner wiring is still proven |
| 16 | **Every subcommand executed**, not `--help`'d — plus `bundle`→`restore` round-tripped, and `--help` kept as a separate, weaker pass |
| **17** | **The coverage audit** |

### The two mechanisms that keep it honest

**Real-by-default (§14).** Everything that can physically run in the image gets a REAL
lifecycle, even when it compiles from source. Install failure is **soft** (a registry outage
is not a LiNix bug); **everything after a successful install is HARD** — that split is what
caught the pixi `global remove` vs `global uninstall` bug a dry-run plan could never see. A
READY backend that cannot run a lifecycle here is named with its reason
(`no_lifecycle_reason`), and an unexplained skip is impossible: a READY backend with no
canary and no reason fails the audit.

**The coverage audit (§17).** It enumerates every backend from `doctor --json` and every
subcommand from `--help`, and **hard-fails on any that no real lifecycle and no plan-smoke
touched**, outside a named exempt set (`shell`, `undo`, `history`, `bisect`, `fleet` — each
printed with why). `<cmd> --help` is ledgered separately and does **not** satisfy it. This is
the only mechanism that can notice what is *missing* from a list of checks, and it is what
makes a backend or command added next year fail until it is covered.

### Where the images differ

- **Every image** runs the real lifecycle for whatever managers it ships, the full command
  surface, and the audit.
- **`tools`** (in the default set) — an Ubuntu base with the ecosystem managers installed
  **and initialized** (opam switch, cabal/nimble index refresh, spack compilers, conda
  channels, flathub remote, krew index, nix made READY, and the go/dotnet/pub bin dirs on
  PATH) so each runs a genuine build/install, not a dry run. A full real run compiles from
  source and can take 20–40 min.
- **`gentoo`** (opt-in; `DISTROS="gentoo"`) — real Portage in **SMOKE_ONLY** mode (baked
  into the image): discovery + plan-smoke + read-only, no source builds. A SMOKE run says so
  in its summary and names every check it skipped, so a narrower sweep cannot be mistaken for
  a full one.
- **Guix / eopkg / slackpkg** are distro-locked — run the harness on a Guix System / Solus /
  Slackware host and their real rows light up automatically; elsewhere they plan-smoke.

Toggle: `SMOKE_ONLY=1` skips real mutation entirely (discovery + plan-smoke + read-only).
`run.sh` forwards it into the container.

### One-shot release gate

`scripts/release-check.sh` (Linux/macOS) and `scripts/release-check.ps1` (Windows) run the
hermetic gates (`cargo fmt`/`clippy`/`test`/`build`) **and** the full integration matrix, then
print a single **GO / NO-GO** verdict with a non-zero exit on any hard failure:

```bash
./scripts/release-check.sh                 # hermetic gates + Docker matrix (incl. gentoo)
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
scripts/integration-windows.sh scoop jq
```

`scripts/integration-windows.sh` mirrors the container harness section for section, with the
same coverage audit. One deliberate difference: **it runs on a real machine, not a disposable
one**, so the real-lifecycle sweep covers only managers that install per-user and uninstall
cleanly (scoop, cargo, npm, pipx, uv, gem, github, brew). The machine-wide ones — winget,
choco, psresource, system pip, and the ones that rewrite a live desktop or editor profile —
are plan-smoked and **named with that reason**, not silently skipped.

## macOS — why it's not in the matrix

macOS **cannot** be run in Docker on non-Apple hardware: containers share the host's Linux
kernel, macOS needs Apple's Darwin/XNU kernel, and Apple's license forbids virtualizing
macOS off Apple hardware. There is no legitimate macOS container image. So brew testing
needs a **real Mac** or a **macOS CI runner**:

- On any Mac: `cargo build && ./scripts/integration-windows.sh brew wget` (the script is
  backend-agnostic despite the name).
- In CI: GitHub Actions provides `macos-latest` runners; run the same steps there.

## What each run proves (and its limits)

- **Proves:** every registered backend is either driven end-to-end against the real manager
  or has its argv/planner wiring exercised; every non-exempt subcommand actually runs; and
  nothing registered is silently untested.
- **Doesn't prove:** long-tail package edge cases, version-pin fidelity on every backend, or
  dependency-resolution corners. Extend the canary table in `run-in-container.sh` to cover
  more.
