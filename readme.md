# LiNix: Universal Mission-Critical Package Orchestrator



**LiNix** (pronounced **L·I·N·I·X** – say the letters) is a high‑performance, asynchronous package management **orchestrator**. It is not “just another package manager” – it is the **manager of managers**. Whether you are on Linux, macOS, or Windows, LiNix allows you to define your entire system state declaratively, then synchronises it with atomic, parallel, and self‑healing transactions.



## Why LiNix?



Traditional package managers operate in a “fire and forget” mode. If a transaction crashes halfway, your system is left in a dirty state. If you move from one OS to another, you have to rewrite your setup scripts. LiNix solves this with:



- **Atomic Transactions** – A Write‑Ahead Log (WAL) ensures that if LiNix crashes, it will automatically **heal** on the next run, replaying or reverting incomplete actions.

- **DAG‑Based Parallel Planning** – LiNix builds a Directed Acyclic Graph (DAG) of your desired state and executes independent tasks concurrently, while respecting strict ordering of dependencies.  

&#x20; *Result: super‑fast synchronisations even with hundreds of packages.*

- **Universal Reach (50+ backends)** – From system managers (APT, Pacman, DNF, Winget, Brew, MacPorts, pkgsrc, Portage/emerge, Guix, eopkg, slackpkg) and language tools (Cargo, NPM, PNPM, Yarn, Bun, Pip, Pipx, uv, Gem, Go, Composer, .NET tools, Conda, opam, LuaRocks, Nimble, mix/Hex, Dart pub, Cabal, Stack, PowerShell Gallery) to dev-environment and specialised resources (asdf, mise, Spack, pixi, Helm plugins, krew, GitHub Releases, direct HTTP downloads, AppImages, VS Code extensions, Emacs packages, system services, BTRFS subvolumes, and more). Plus an **onboarder** to teach LiNix any CLI manager from a TOML file — no code changes.

- **Imperative → Declarative Bridge** – `linix install` and `linix remove` automatically write to your declarative manifests (`local.txt`). You can start with an empty config, imperatively add packages as you need them, and later run `linix sync` to clean up or replicate the same state elsewhere.

- **Safe Drift Handling** – Packages no longer present in your manifests are reported as "drift" by `linix status` and removed only when you run `linix prune` (or by `sync` if you opt in with `prune_on_sync = true`). `sync` never removes anything by default.

- **Removal Guard** – Drift is derived from managed state, and managed state can be wrong (a mis-scoped manifest, a state file from another machine). So every command that can delete — `apply`, `prune`, `sync`, `watch`, `upgrade`, `rollback`, `canary`, `remove`, ghost-shell exit, lease expiry — refuses a removal that exceeds `max_removals` (default 20) or touches a protected package. Protection comes from a built-in list, anything you add, **and** the OS's own "essential" flags where it has them (`dpkg`'s `Essential`/`Priority: required`). Run `linix protected` to see the effective rules. The only override is `--allow-mass-removal`; **`--yes` is deliberately not one**, because every script and CI job passes `-y`, and an unattended run is exactly the one that cannot notice a system being dismantled.

- **Time Travel (Snapshots)** – Built‑in support for BTRFS, ZFS, Timeshift, and Windows Restore Points. Create an automatic snapshot before every `sync` or `upgrade`, and roll back your entire system with `linix undo`.

- **Sandboxing & Security** – Hardened execution using Bubblewrap (Linux), `sandbox‑exec` (macOS), and Windows Sandbox (with fallback). Define sandbox policies declaratively per package.

- **Self‑Healing & Telemetry** – Every operation is logged with retry counts, download sizes, and timings. The engine automatically retries transient failures and rolls back on critical errors.

- **Ephemeral Environments (Ghost Shell)** – Run `linix shell python:3.11 node:20` and get an isolated, one‑off environment with exactly those tools – no pollution, no leftovers. To execute a single command in a throwaway environment instead of an interactive shell, use `linix run --packages python:3.11 "python script.py"`.

- **High‑Performance Rust Shims** – Deploy sub‑millisecond shims to `~/.local/bin` that act as transparent proxies to sandboxed applications.

- **Cross‑Backend Teleportation** – Move a package from one backend to another (e.g., `apt:ripgrep` → `cargo:ripgrep`) with a single command. LiNix updates both the live system and your declarative manifests.

- **System Profiles** – Named software sets you can turn on and off live, with no reboot: `linix activate work`, `linix deactivate work`. **Several profiles can be active at once** — their package sets are unioned, and deactivating one only removes what no other active profile still needs. Profiles compose **relationally**: a `.profile` file can `include` another profile (the "plus"), `exclude` another profile, or subtract a single package with `-pkg` (the "minus") — e.g. `lean = base + rust - heavy`. `linix profile list/show/create/save/switch` round out management; `linix profile show <name>` prints the resolved set.

- **Imperative‑to‑Declarative Migration** – Already installed a bunch of packages manually? Run `linix migrate` – LiNix discovers all installed packages across every backend and automatically writes them into your declarative manifests, taking ownership without reinstalling.

- **Auto‑Lock Checksums** – For `web` and `github` packages, LiNix automatically calculates SHA256 checksums and stores them in a staging lock file (`locks.json`), preventing race conditions and ensuring integrity.

- **Dry‑Run Mode** – Use `linix --dry-run sync` to preview exactly what would change without touching the system.



## Quick Start



### Installation



LiNix is written in Rust. From source:



```bash

cargo build --release

cp target/release/linix ~/.local/bin/

```



### Your First Manifest (Declarative)



Create a directory and a manifest file:



```bash

mkdir -p ~/.config/linix/groups

echo 'apt:curl

brew:fzf

cargo:ripgrep

npm:typescript@version=>5.0.0

github:BurntSushi/ripgrep

web:https://example.com/app.tar.gz' > ~/.config/linix/groups/local.txt

```

A manifest line is `[backend:]name[@k=v,…]`. Beyond plain packages, these directives are
recognised:

| Directive | Meaning |
|-----------|---------|
| `@module:<name>` | Splice in a reusable module (`modules/<name>.module.txt`); `@module:dev -vim` drops one package from *its* contribution. |
| `group:<name>` | Include a named group (`groups/<name>.txt` or a config `[groups]` entry). |
| `include: <path\|url>` | Splice in another manifest file **or a remote URL** in place — compose a shared base with per‑role overlays. |
| `when <os\|arch\|host> == <v>` … `end` | Host‑conditional block (also `!=` and `in [a, b]`, and an inline `when … then <line>`), so one manifest serves a mixed fleet. |
| `<pkg>@check=port:5432` / `@check=cmd:pg_isready` | Post‑install **health probe**: after the package installs, LiNix verifies it (TCP port or shell command) and flags a failure, with the pre‑sync snapshot available to revert. |
| `<pkg>@version=…`, `@lease=<dur>`, `service:<name>@enabled=true` | Version pin, self‑removing temporary install, and declarative service, respectively. |
| `-<pkg>` | Exclude a package contributed by an included module/group. |



### Or Start Imperatively



```bash

linix install apt:curl

linix install cargo:ripgrep

```



LiNix will automatically append these entries to `local.txt`. Later, you can edit the file and run `linix sync` to reconcile.



### Synchronise



```bash

linix sync

```



LiNix will:

- Query every backend to see what is already installed.

- Resolve all transitive dependencies (e.g., `apt-cache depends`).

- Build a DAG of required installs, upgrades, and removals.

- Execute the plan in parallel, respecting ordering.

- On success, update the local state registry and optionally auto‑lock checksums.



## Command Reference



| Command | Description |

|---------|-------------|

| `sync` | Align system state with declarative manifests (`.txt` files). |

| `install` | Imperatively add a package and **auto‑commit** to your `local.txt`. |

| `remove` | Purge a package and **auto‑update** manifests. |

| `teleport` | Move a package from one backend to another. |

| `shell` | Enter an ephemeral, isolated environment with specified packages. |

| `undo` | Browse the snapshot gallery and roll back the entire system. |

| `activate` / `deactivate` | Turn profiles on/off live (multiple can be active; sets are unioned). |

| `profile` | Manage profiles: `list`, `show`, `create`, `save`, `switch`, `active`. |

| `doctor [--fix] [--json]` | Deep health check: per‑backend readiness/severity, config/state integrity, directory layout, and lockfile drift. `--fix` repairs missing dirs, stale metadata, and a drifted lockfile. |

| `heal` | Recover from a failed or interrupted transaction (WAL replay). |

| `search` | Parallel, cross‑backend search (e.g., `linix search ripgrep`). `--installed` filters to packages you already manage. |

| `update` / `upgrade` | Refresh metadata / upgrade managed packages. `upgrade` spans every granularity: bare/`--all` (native whole‑system), `upgrade <pkg>…`, `--backend <b>`, and `--security` (upgrade exactly what `audit` flags as vulnerable, pinned to the fixed version); `--except <pkg>` holds packages back. |

| `clean` | Remove orphans, clear caches, and purge temporary files. |

| `migrate` | Ingest all manually installed packages into LiNix management. |

| `shim` | Create a high‑performance Rust shim for a binary. |

| `orphans` | **List** drift/orphaned packages (non‑destructive; `clean` performs removal). |

| `unmanaged` | List installed packages not under LiNix management. |

| `unmanage <pkg>` | Stop managing a package **without uninstalling it**. Deleting a manifest line means "uninstall this"; this is how you say "forget this, leave it alone". |

| `protected [pkg]` | Show what the removal guard refuses to delete, and why. `--json` for machines. |

| `repo add/remove/list` | Manage source repositories (winget/dnf/pacman/apt/scoop/choco…). |

| `config init/path/show` | Scaffold and inspect the application config file. |

| `status` (alias `diff`) | **Read-only** preview: what `sync` would install, drift `prune` would remove, and unmanaged packages. |

| `prune` | Remove drift (installed but no longer in your manifests). Separate from `sync`. |

| `lock` | Pin every managed package to its installed version in `locks.json` for reproducible installs — now **signed** with a machine‑local key so `sync --locked` refuses a tampered lockfile. |

| `init` | Scaffold the LiNix directory structure and a starter manifest on a fresh machine. |

| `audit` | Cross‑ecosystem security scan: check **every** managed package (apt, npm, pip, cargo, gem…) against the OSV.dev vulnerability database in one pass. |

| `sbom` | Emit a single CycloneDX software bill of materials spanning every backend. |

| `why <pkg>` | Explain why a package is installed: its provenance (which manifest/module/imperative action) and what depends on it. `--json` for machine output. |

| `upgrade --canary --test <cmd>` | Health‑gated upgrade: snapshot first, run the test after upgrading, and **auto‑roll‑back** if it fails. |

| `bisect --test <cmd>` | System time‑travel bisect: restore snapshots and run the test to find the change that introduced a regression. |

| `clone <user@host>` | Replicate another machine's installed packages over SSH, translating backends per‑OS. |

| `fleet [hosts…] [--sync] [--apply]` | Compare many machines over SSH against their manifests and report drift; `--sync` reconciles the drifted ones, `--apply` pushes `sync` to every reachable host. |

| `policy` | Check the desired state against the `[guard]` install/change rules (also enforced automatically before `sync`/`upgrade`). |

| `run --packages <pkgs> "<cmd>"` | Run a single command inside an ephemeral environment with those packages, then tear it down. |

| `shell <pkgs…>` | Enter an interactive, isolated ghost shell with the given packages loaded. |

| `list` | List installed/managed packages (`--backend <b>` to filter, `--json` for machine output, `--outdated` to show only packages with a newer version available). |

| `info <pkg>` | Show detailed metadata (version, description, install path, dependencies) for a package. |

| `module list/show/create` | Manage reusable package modules referenced from manifests with the `@module` syntax. |

| `snapshot list/prune` | List system snapshots, or prune old ones per the configured retention policy. |

| `generation list/pin/unpin/rollback` | Manage generations (each records the realized package set plus a frozen copy of the manifests that produced it). |

| `rollback <id>` | Shorthand for `generation rollback`: realize a saved generation on the system. Scope with `--package` / the global `--backend`. |

| `lease list/set` | Manage temporary package leases with expirations (e.g. `lease set apt:foo --duration 30d`); expired leases are swept on each run. |

| `schedule add/list/remove` | Register native scheduled tasks (systemd / launchd / Task Scheduler) that run a LiNix command on a cron expression, with optional desktop/email notifications. |

| `completions <shell>` | Emit a shell completion script (`bash`, `zsh`, `fish`, `powershell`, `elvish`, `nushell`). |

| `conflicts` | **Cross‑backend conflict detection**: the same tool pinned to different versions by two backends, or provided by more than one (PATH‑shadowing risk) — something no single‑backend resolver can see. `--json` for machine output. |

| `watch [--interval N] [--on-change] [--pull] [--once]` | Continuously reconcile the system to your manifests (GitOps for one machine); optionally `git pull --ff-only` each tick, then apply changes unattended. |

| `hold <pkgs…>` / `unhold <pkgs…>` | Freeze packages so `upgrade` never bumps them (like `apt-mark hold`); run `hold` with no args to list. Honored by targeted upgrades and the planner; naming a held package explicitly still upgrades it. |

| `export [--format brew\|pip\|npm\|apt] [--out DIR] [--stdout]` | Emit **native** manifests (Brewfile, requirements.txt, package.json, Aptfile) from your managed set — the no‑lock‑in escape hatch. |

| `service enable/disable/start/stop/restart/status/list` | Declarative service management across systemd, OpenRC, SysVinit, launchd, and Windows `sc`; `enable`/`disable` persist to your manifest. |

| `bundle [--out DIR] [--artifacts] [--archive]` | Pack a portable, offline/air‑gapped bundle of your config, lockfile, and resolved package list; `--artifacts` pre‑downloads packages, `--archive` packs a single `.tar.gz`. |

| `plan [--out FILE]` / `apply <file>` | Freeze what `sync` would do to a reviewable file, then apply exactly that (Terraform‑style); `apply` warns on drift and offers an interactive review. |

| `generation log/diff` | `log [--oneline] [--json]` prints history git‑log style; `diff <from> [<to>]` shows packages added/removed/version‑changed between two generations (omit `<to>` to diff against the live system). |

| `self-upgrade [--git URL] [--check]` | Rebuild and install the latest LiNix from source with cargo (same mechanism as the install script). |

| `config edit` | Open the config in `$EDITOR`/`$VISUAL` and re‑validate on save. |

| `module add <source>` | Fetch a shared module from `github:user/repo` or a raw URL into your local modules. |



### Global flags

These apply to every subcommand:

| Flag | Meaning |
|------|---------|
| `-n`, `--dry-run` | Preview only; make no system changes. |
| `-y`, `--yes` | Skip confirmation prompts (required for non‑interactive/CI runs). |
| `-b`, `--backend <name>` | Force a specific backend for the operation. |
| `-c`, `--config <path>` | Use a custom `config.toml`. |
| `-g`, `--groups-dir <path>` | Use a custom directory of manifest (`.txt`) files. |
| `--progress <bool>` | Toggle progress indicators (default `true`). |
| `-v`, `--verbose` | Enable debug‑level logging (logs go to stderr; stdout stays reserved for `--json`). |
| `-q`, `--quiet` | Suppress the flight plan and transaction summary (errors still print). |

## In development (unreleased, on `main`)

Since 6.0.0, the following have landed in the codebase (see the `[Unreleased]` section of
[CHANGELOG.md](CHANGELOG.md)):

- **More backends** — MacPorts (`macports`), pkgsrc (`pkgin`), .NET global tools
  (`dotnet`), Conda (`conda`), PowerShell Gallery (`psresource`), uv (`uv`), XBPS (`xbps`),
  and the AUR helpers `yay` / `paru` as first‑class backends.
- **Generations & rollback** — every change records a *generation* (the exact realized
  package set plus a frozen copy of the manifests). Manage them with
  `linix generation list|pin|unpin|rollback`, or `linix rollback <id>` (optionally scoped
  to a single `--package` / `--backend`).
- **Declarative retention** — independent `keep_last` / `keep_days` / `keep` policies for
  generations, snapshots, and a manifest archive.
- **Module/group exclusion** — `@module:dev -vim` includes a module but drops one package
  from *its* contribution.
- **Inline managed config files** (`[managed_files]`) — declare a file's body directly in
  config; it's materialized as a self‑healing `link`, with the original backed up once
  before any overwrite.
- **Per‑host backend allow‑lists** (`[hostname_backends]`) — restrict a machine to a
  subset of backends.

## What's new (unreleased feature wave)

- **Cross‑backend conflict detection (`conflicts`).** Finds clashes no single‑backend resolver
  can see — the same tool pinned to different versions by two backends (`apt:nodejs@18` vs
  `nix:node@20`), or provided by more than one (a PATH‑shadowing risk).
- **`watch` daemon.** GitOps for one machine: continuously reconcile to your manifests,
  optionally `git pull --ff-only` first, applying changes unattended.
- **Package holds (`hold`/`unhold`).** Freeze packages so `upgrade` never bumps them — honored
  by targeted upgrades and the declarative planner.
- **Tamper‑evident lockfiles.** `linix lock` signs `locks.json` with a machine‑local key;
  `sync --locked` refuses a locally‑edited lockfile (fails closed). A fresh machine with no key
  proceeds unverified rather than breaking reproducibility.
- **`export` to native manifests.** Emit Brewfile / requirements.txt / package.json / Aptfile —
  the no‑lock‑in escape hatch and a way to interop with other tools.
- **Exact‑version security remediation.** `upgrade --security` pins each vulnerable package to
  the highest fixed version OSV reports (clearing all its advisories) instead of jumping to latest.
- **Generation `log`/`diff`, interactive rollback/apply review, `list --outdated`,
  `search --installed`, `why --json`, command aliases (`[command_aliases]`), `config edit`,
  `self-upgrade`, `bundle --archive` (`.tar.gz`), `doctor --fix` lockfile heal, per‑backend timing,
  `--quiet`, colored doctor output, `@check=` post‑install health probes, and manifest
  `include:` (local path or URL).** See [CHANGELOG.md](CHANGELOG.md) for the full list.

## What's new in 6.0.0

- **`audit` — one security scan across every ecosystem.** Checks all managed packages
  (apt, npm, pip, cargo, gem, go…) against OSV.dev and reports fixed versions. No other
  tool audits your whole machine across package managers at once.
- **`sbom`** — a single CycloneDX bill of materials for the entire system.
- **`why <pkg>`** — provenance (which manifest/module/imperative action pulled it in) plus
  cross‑package reverse dependencies.
- **`upgrade --canary --test <cmd>`** — snapshot → upgrade → health‑check → automatic
  rollback on failure, so a bad upgrade never leaves you broken.
- **`bisect --test <cmd>`** — binary‑search your snapshots to find the change that broke
  something.
- **`clone <user@host>` / `fleet`** — replicate a machine over SSH (translating backends
  per‑OS), and see/repair drift across many machines.
- **The `[guard]` gate** — nine refusals (`protected_packages`, `unprotected_packages`,
  `max_removals`, `max_installs`, `deny_packages`, `pinned_only`, `require_snapshot`,
  `deny_vulnerable`, and OS-essential) enforced before any change; `-y` cannot skip them.
- **`init`** — scaffold the directory layout + a starter manifest on a fresh machine.
- **Safer pruning** — `prune_scope` (`managed` default vs whole‑`system`) and
  `protect_imperative` shield imperative installs; leases are swept on every run; and
  backends that can't clean orphans now report honestly instead of pretending to.

## What's new in 5.0.0

- **`sync` is non‑destructive by default** — it installs/upgrades; drift removal moved to a
  separate `prune` command (opt back into sync with `prune_on_sync = true`). Preview
  everything first with `linix status`.
- **Reproducible installs** — `linix lock` pins exact versions to `locks.json`, and every
  backend now honors those versions natively on `sync --locked` (see *Reproducibility* below).
- **Scoped `upgrade --module/--group/--profile` is non‑destructive** — upgrades only within
  scope, never removes out‑of‑scope packages.
- **Search everywhere** — `Searchable` for brew, cargo, npm, pnpm, yarn, mise, snap, flatpak,
  nix, emacs, and pip; **repository management** for dnf, pacman, and winget.
- **`max_parallel` now controls install/remove concurrency**, plus new config options
  (`network_timeout_secs`, `nix_gc_age`, `confirm_destructive`, `prune_on_sync`) and
  `linix config`/`completions`/`status`/`prune`/`lock` commands.

See [CHANGELOG.md](CHANGELOG.md) for the full list.

## Reproducibility (and its honest limits)

LiNix is **not** a Nix replacement: it orchestrates imperative managers (apt, brew, winget),
whose *outputs* aren't reproducible, so it can't give you hermetic, bit‑identical builds.
What it does give you is **reproducible inputs** via a cross‑backend lockfile:

```sh
linix lock              # record installed versions -> locks.json
linix sync --locked     # on another machine: install those exact versions
```

Per‑backend version‑pin support (what `--locked` actually enforces):

| Pinning | Backends |
|---|---|
| **Exact version** | apt, apk, zypper (`name=ver`) · dnf (`name-ver`) · pip, pipx (`name==ver`) · npm, pnpm, yarn, bun, mise (`name@ver`) · cargo, gem, winget, choco (flags) · vscode (`ext@ver`) |
| **Best‑effort** | brew (only formulae with versioned variants, e.g. `python@3.11`) |
| **Not supported** (model doesn't allow it) | pacman, xbps, yay, paru (rolling) · snap (channels) · flatpak (commits) · nix (flake refs) · mas (store latest) · scoop |

For anything that truly must be reproducible, use the **`nix`** backend for that part and let
LiNix orchestrate the rest — LiNix doesn't compete with Nix, it can include it.



## Architecture Highlights



### SOLID by Design



- **Capability‑based** – Backends implement only the traits they support: `Installable`, `Queryable`, `Searchable`, `Upgradable`, `RepoManager`, `MetadataProvider`.

- **Interface Segregation** – LiNix never requires a backend to provide features it doesn’t have.

- **Dependency Inversion** – High‑level planners depend on abstract `BackendCore` traits, not concrete implementations.



### Recursive Dependency Resolution



LiNix does not rely only on your declared `requires`. It queries each backend’s native metadata (e.g., `pacman -Si`, `dnf repoquery`) for a package's **direct** dependencies to order the install graph, then lets the backend resolve and install the full transitive closure itself (which every supported package manager does). LiNix deliberately does not recursively re-derive the entire tree: that is redundant with the backend and, for managers whose dependency query answers from a local cache, pathologically slow.



### Transaction Engine



Every DAG node is executed with:

- Per‑node timeouts

- Exponential backoff retries

- Telemetry (attempt count, bytes downloaded, duration)

- Journaled Write‑Ahead Log (WAL) for crash recovery



If a node fails and `auto_rollback` is enabled, the engine rolls back successfully completed nodes in reverse order – with its own retry logic – leaving the system in a consistent state.



### 100% Hermetic Test Suite



LiNix ships with a fully isolated integration suite:

- Uses `MockExecutor` to simulate command outputs.

- Redirects all filesystem operations to temporary directories.

- Never touches the real network, disk, or system state.

- Platform‑gated tests (apt only on Linux, winget only on Windows, etc.).



## Building & Testing

LiNix is a standard Cargo project. It builds two binaries: `linix` (the CLI) and `shim`
(the tiny proxy deployed by `linix shim`).

```bash
cargo build --release        # optimized build -> target/release/linix
cargo test                   # run the hermetic unit/integration suite
```

The suite in `tests/` is **hermetic**: it mocks command execution (`MockExecutor`),
redirects all filesystem writes to temp directories, and never touches the real network or
system — so it proves the logic but never mutates your machine. Tests are platform‑gated
(apt only on Linux, winget only on Windows, etc.).

To exercise the **real** package managers end‑to‑end (search → install → verify → remove),
use the integration harnesses:

- **Linux** — `docker/integration/run.sh` builds disposable containers for Ubuntu (apt),
  Fedora (dnf), Arch (pacman), and Alpine (apk) and drives the real `linix` binary against
  each. Requires Docker. Override the test package (`./docker/integration/run.sh htop`) or
  the distro set (`DISTROS="ubuntu arch" ./docker/integration/run.sh`).
- **Windows / macOS** — `scripts/integration-windows.sh` drives the host‑native backends
  (scoop, winget, choco, brew) directly, since those OSes can't run in a Linux container.
- **Windows smoke** — `verify.ps1` and `full-test.ps1` are quick PowerShell helpers that
  build the release binary and run a handful of smoke checks.

See `docker/integration/README.md` for the full integration guide.

## Platform Support



| Platform | Sandboxing | Snapshot Providers | Native Backends |

|----------|------------|--------------------|------------------|

| **Linux** | Bubblewrap (bwrap) | BTRFS, Timeshift, ZFS | apt, pacman, dnf, apk, zypper, xbps, yay, paru, snap, flatpak, nix, pkgin, conda, cargo, dotnet, npm, pip, pipx, uv, bun, yarn, pnpm, gem, mise, link, service, github, web, appimage, emacs, vscode, btrfs |

| **macOS** | `sandbox‑exec` | Time Machine (planned) | brew, mas, macports, pkgin, conda, cargo, dotnet, npm, pip, pipx, uv, gem, mise, link, service, github, web, emacs, vscode |

| **Windows** | Windows Sandbox + low‑integrity fallback | Windows Restore Points | winget, scoop, choco, psresource, conda, cargo, dotnet, npm, pip, pipx, uv, bun, yarn, pnpm, gem, mise, link, service, github, web, emacs, vscode |



## Configuration



LiNix is configured via `~/.config/linix/config.toml`. You can:

- Set global backend priority

- Enable/disable specific backends

- Redirect snapshot and temporary directories

- Define sandboxing behaviour

- Add custom Lua/Rhai hooks



See `linix doctor` for a quick health check of your configuration and backends.



## Contributing & Roadmap



LiNix is open source (MIT license). Contributions are welcome – especially:

- More backends (e.g., Podman, Docker, Helm)

- Additional snapshot providers (e.g., LVM, rsnapshot)

- Windows Sandbox enhancements



## Final Words



LiNix v6.0.0 is **mission‑critical ready**. It has been hardened with:

- Thousands of lines of async‑safe I/O

- A fully decoupled, testable service architecture

- Zero `unwrap()` panics in production paths

- Real‑world testing across Linux, macOS, and Windows



**Try it today – define your system once, run it anywhere.**

