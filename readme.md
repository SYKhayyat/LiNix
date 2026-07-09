# LiNix: Universal Mission-Critical Package Orchestrator



**LiNix** (pronounced **L·I·N·I·X** – say the letters) is a high‑performance, asynchronous package management **orchestrator**. It is not “just another package manager” – it is the **manager of managers**. Whether you are on Linux, macOS, or Windows, LiNix allows you to define your entire system state declaratively, then synchronises it with atomic, parallel, and self‑healing transactions.



## Why LiNix?



Traditional package managers operate in a “fire and forget” mode. If a transaction crashes halfway, your system is left in a dirty state. If you move from one OS to another, you have to rewrite your setup scripts. LiNix solves this with:



- **Atomic Transactions** – A Write‑Ahead Log (WAL) ensures that if LiNix crashes, it will automatically **heal** on the next run, replaying or reverting incomplete actions.

- **DAG‑Based Parallel Planning** – LiNix builds a Directed Acyclic Graph (DAG) of your desired state and executes independent tasks concurrently, while respecting strict ordering of dependencies.  

&#x20; *Result: super‑fast synchronisations even with hundreds of packages.*

- **Universal Reach (38+ backends)** – From system managers (APT, Pacman, DNF, Winget, Brew, MacPorts, pkgsrc) and language tools (Cargo, NPM, Pip, Gem, .NET tools, Conda, PowerShell Gallery) to specialised resources (GitHub Releases, direct HTTP downloads, AppImages, VS Code extensions, Emacs packages, system services, BTRFS subvolumes, and more). Plus an **onboarder** to teach LiNix any CLI manager from a TOML file — no code changes.

- **Imperative → Declarative Bridge** – `linix install` and `linix remove` automatically write to your declarative manifests (`local.txt`). You can start with an empty config, imperatively add packages as you need them, and later run `linix sync` to clean up or replicate the same state elsewhere.

- **Safe Drift Handling** – Packages no longer present in your manifests are reported as "drift" by `linix status` and removed only when you run `linix prune` (or by `sync` if you opt in with `prune_on_sync = true`). `sync` never removes anything by default.

- **Time Travel (Snapshots)** – Built‑in support for BTRFS, ZFS, Timeshift, and Windows Restore Points. Create an automatic snapshot before every `sync` or `upgrade`, and roll back your entire system with `linix undo`.

- **Sandboxing & Security** – Hardened execution using Bubblewrap (Linux), `sandbox‑exec` (macOS), and Windows Sandbox (with fallback). Define sandbox policies declaratively per package.

- **Self‑Healing & Telemetry** – Every operation is logged with retry counts, download sizes, and timings. The engine automatically retries transient failures and rolls back on critical errors.

- **Ephemeral Environments (Ghost Shell)** – Run `linix shell --packages python:3.11 node:20` and get an isolated, one‑off environment with exactly those tools – no pollution, no leftovers.

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

| `doctor` | Check backend health, sandbox availability, and system readiness. |

| `heal` | Recover from a failed or interrupted transaction (WAL replay). |

| `search` | Parallel, cross‑backend search (e.g., `linix search ripgrep`). |

| `update` / `upgrade` | Refresh metadata and upgrade all managed packages. |

| `clean` | Remove orphans, clear caches, and purge temporary files. |

| `migrate` | Ingest all manually installed packages into LiNix management. |

| `shim` | Create a high‑performance Rust shim for a binary. |

| `orphans` | **List** drift/orphaned packages (non‑destructive; `clean` performs removal). |

| `unmanaged` | List installed packages not under LiNix management. |

| `repo add/remove/list` | Manage source repositories (winget/dnf/pacman/apt/scoop/choco…). |

| `config init/path/show` | Scaffold and inspect the application config file. |

| `status` (alias `diff`) | **Read-only** preview: what `sync` would install, drift `prune` would remove, and unmanaged packages. |

| `prune` | Remove drift (installed but no longer in your manifests). Separate from `sync`. |

| `lock` | Pin every managed package to its installed version in `locks.json` for reproducible installs. |

| `init` | Scaffold the LiNix directory structure and a starter manifest on a fresh machine. |

| `audit` | Cross‑ecosystem security scan: check **every** managed package (apt, npm, pip, cargo, gem…) against the OSV.dev vulnerability database in one pass. |

| `sbom` | Emit a single CycloneDX software bill of materials spanning every backend. |

| `why <pkg>` | Explain why a package is installed: its provenance (which manifest/module/imperative action) and what depends on it. |

| `upgrade --canary --test <cmd>` | Health‑gated upgrade: snapshot first, run the test after upgrading, and **auto‑roll‑back** if it fails. |

| `bisect --test <cmd>` | System time‑travel bisect: restore snapshots and run the test to find the change that introduced a regression. |

| `clone <user@host>` | Replicate another machine's installed packages over SSH, translating backends per‑OS. |

| `fleet [hosts…] [--sync]` | Compare many machines over SSH against their manifests, report drift, and optionally reconcile. |

| `policy` | Check the desired state against declarative rules in `policy.toml` (also enforced automatically before `sync`/`upgrade`). |



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
- **Policy gate (`policy.toml`)** — declarative rules (`deny_packages`, `allow_backends`,
  `pinned_only`, `require_snapshot`, `deny_vulnerable`) enforced before any change.
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



## Platform Support



| Platform | Sandboxing | Snapshot Providers | Native Backends |

|----------|------------|--------------------|------------------|

| **Linux** | Bubblewrap (bwrap) | BTRFS, Timeshift, ZFS | apt, pacman, dnf, apk, zypper, xbps, yay, paru, snap, flatpak, nix, pkgin, conda, cargo, dotnet, npm, pip, pipx, uv, bun, yarn, pnpm, gem, go, composer, link, service, github, web, appimage, emacs, vscode, btrfs |

| **macOS** | `sandbox‑exec` | Time Machine (planned) | brew, mas, macports, pkgin, conda, cargo, dotnet, npm, pip, pipx, uv, gem, go, link, service, github, web, emacs, vscode |

| **Windows** | Windows Sandbox + low‑integrity fallback | Windows Restore Points | winget, scoop, choco, psresource, conda, cargo, dotnet, npm, pip, pipx, uv, bun, yarn, pnpm, gem, go, link, service, github, web, emacs, vscode |



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

