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
./docker/integration/run.sh              # ubuntu(apt), fedora(dnf), arch(pacman), alpine(apk)
./docker/integration/run.sh htop         # override the test package
DISTROS="ubuntu arch" ./docker/integration/run.sh   # a subset
```

Each distro has a Dockerfile that installs a Rust toolchain (via that distro's own package
manager — which also warms its package DB), builds `linix`, and runs
`run-in-container.sh <backend> <package>`. A non-zero exit means that backend's real
install/remove path failed.

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
