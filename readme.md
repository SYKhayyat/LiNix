# LiNix: Universal Mission-Critical Package Orchestrator



**LiNix** (pronounced **L·I·N·I·X** – say the letters) is a high‑performance, asynchronous package management **orchestrator**. It is not “just another package manager” – it is the **manager of managers**. Whether you are on Linux, macOS, or Windows, LiNix allows you to define your entire system state declaratively, then synchronises it with atomic, parallel, and self‑healing transactions.



## Why LiNix?



Traditional package managers operate in a “fire and forget” mode. If a transaction crashes halfway, your system is left in a dirty state. If you move from one OS to another, you have to rewrite your setup scripts. LiNix solves this with:



- **Atomic Transactions** – A Write‑Ahead Log (WAL) ensures that if LiNix crashes, it will automatically **heal** on the next run, replaying or reverting incomplete actions.

- **DAG‑Based Parallel Planning** – LiNix builds a Directed Acyclic Graph (DAG) of your desired state and executes independent tasks concurrently, while respecting strict ordering of dependencies.  

&#x20; *Result: super‑fast synchronisations even with hundreds of packages.*

- **Universal Reach (33+ backends)** – From system managers (APT, Pacman, DNF, Winget, Brew) and language tools (Cargo, NPM, Pip, Gem) to specialised resources (GitHub Releases, direct HTTP downloads, AppImages, VS Code extensions, Emacs packages, system services, BTRFS subvolumes, and more).

- **Imperative → Declarative Bridge** – `linix install` and `linix remove` automatically write to your declarative manifests (`local.txt`). You can start with an empty config, imperatively add packages as you need them, and later run `linix sync` to clean up or replicate the same state elsewhere.

- **Automatic Drift Correction** – Packages that are no longer present in your manifests are automatically removed during `linix sync` (unless they are marked as protected).

- **Time Travel (Snapshots)** – Built‑in support for BTRFS, ZFS, Timeshift, and Windows Restore Points. Create an automatic snapshot before every `sync` or `upgrade`, and roll back your entire system with `linix undo`.

- **Sandboxing & Security** – Hardened execution using Bubblewrap (Linux), `sandbox‑exec` (macOS), and Windows Sandbox (with fallback). Define sandbox policies declaratively per package.

- **Self‑Healing & Telemetry** – Every operation is logged with retry counts, download sizes, and timings. The engine automatically retries transient failures and rolls back on critical errors.

- **Ephemeral Environments (Ghost Shell)** – Run `linix shell --packages python:3.11 node:20` and get an isolated, one‑off environment with exactly those tools – no pollution, no leftovers.

- **High‑Performance Rust Shims** – Deploy sub‑millisecond shims to `~/.local/bin` that act as transparent proxies to sandboxed applications.

- **Cross‑Backend Teleportation** – Move a package from one backend to another (e.g., `apt:ripgrep` → `cargo:ripgrep`) with a single command. LiNix updates both the live system and your declarative manifests.

- **System Profiles** – Switch between different machine identities (e.g., “work”, “gaming”, “server”) with `linix profile`. Each profile can have its own set of group files.

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

| `profile` | Switch between different system identities (work, home, server). |

| `doctor` | Check backend health, sandbox availability, and system readiness. |

| `heal` | Recover from a failed or interrupted transaction (WAL replay). |

| `search` | Parallel, cross‑backend search (e.g., `linix search ripgrep`). |

| `update` / `upgrade` | Refresh metadata and upgrade all managed packages. |

| `clean` | Remove orphans, clear caches, and purge temporary files. |

| `migrate` | Ingest all manually installed packages into LiNix management. |

| `shim` | Create a high‑performance Rust shim for a binary. |



## Architecture Highlights



### SOLID by Design



- **Capability‑based** – Backends implement only the traits they support: `Installable`, `Queryable`, `Searchable`, `Upgradable`, `RepoManager`, `MetadataProvider`.

- **Interface Segregation** – LiNix never requires a backend to provide features it doesn’t have.

- **Dependency Inversion** – High‑level planners depend on abstract `BackendCore` traits, not concrete implementations.



### Recursive Dependency Resolution



LiNix does not rely only on your declared `requires`. It queries each backend’s native metadata (e.g., `apt-cache depends`, `pacman -Si`, `dnf repoquery`) to build a **complete transitive dependency graph** before any installation begins.



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

| **Linux** | Bubblewrap (bwrap) | BTRFS, Timeshift, ZFS | apt, pacman, dnf, apk, zypper, snap, flatpak, nix, cargo, npm, pip, pipx, bun, yarn, pnpm, gem, go, composer, link, service, github, web, appimage, emacs, vscode, btrfs |

| **macOS** | `sandbox‑exec` | Time Machine (planned) | brew, mas, cargo, npm, pip, pipx, gem, go, link, service, github, web, emacs, vscode |

| **Windows** | Windows Sandbox + low‑integrity fallback | Windows Restore Points | winget, scoop, choco, cargo, npm, pip, bun, yarn, pnpm, gem, go, link, service, github, web, emacs, vscode |



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



LiNix v3.5.0 is **mission‑critical ready**. It has been hardened with:

- Thousands of lines of async‑safe I/O

- A fully decoupled, testable service architecture

- Zero `unwrap()` panics in production paths

- Real‑world testing across Linux, macOS, and Windows



**Try it today – define your system once, run it anywhere.**

