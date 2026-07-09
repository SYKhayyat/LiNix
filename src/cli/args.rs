use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// LiNix - Universal Mission-Critical Package Manager
/// High-performance, DAG-based orchestration for 33+ backends.
/// Version 6.0.0: cross-ecosystem audit/SBOM, provenance (`why`), health-gated canary
/// upgrades, snapshot bisect, SSH clone/fleet, a policy gate, and system-scope pruning.
#[derive(Parser, Debug)]
#[command(
    name = "linix",
    version = "6.0.0",
    about = "Universal Mission-Critical Package Manager"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Run without making actual system changes
    #[arg(short = 'n', long, global = true)]
    pub dry_run: bool,

    /// Skip confirmation prompts
    #[arg(short, long, global = true)]
    pub yes: bool,

    /// Force a specific backend for the operation
    #[arg(short, long, global = true)]
    pub backend: Option<String>,

    /// Path to custom config.toml
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Directory containing package group files (.txt)
    #[arg(short, long, global = true)]
    pub groups_dir: Option<PathBuf>,

    /// Toggle progress indicators
    #[arg(long, global = true, default_value = "true")]
    pub progress: bool,

    /// Enable debug-level logging
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Synchronize system state with declarative configuration (DAG-based)
    Sync {
        /// Force strict version matching against locked state
        #[arg(long)]
        locked: bool,

        /// Output the transition plan as JSON (requires --dry-run)
        #[arg(long)]
        json: bool,
    },

    /// Run a command within an ephemeral package environment
    Run {
        /// Packages to make available in the environment
        #[arg(short, long)]
        packages: Vec<String>,
        /// The command to execute
        command: String,
    },

    /// Create a permanent high-performance Rust shim for a package
    Shim {
        /// The name of the binary to create
        binary: String,
        /// The source package spec (e.g. "cargo:ripgrep")
        #[arg(short, long)]
        source: String,
    },

    /// Recover the system from an interrupted or crashed transaction (WAL)
    Heal,

    /// Perform a deep system cleanup (orphans, cache, temp files)
    Clean,

    /// Identify all packages installed on the OS but not managed by LiNix
    Unmanaged,

    /// List and remove orphaned dependencies across all backends
    Orphans,

    /// Show what `sync` would change (to install / drift to remove / unmanaged) — read-only
    #[command(alias = "diff")]
    Status {
        /// Output the report as JSON
        #[arg(long)]
        json: bool,
    },

    /// Remove drift: packages installed but no longer in your manifests
    Prune {
        /// Output the removal plan as JSON without removing anything
        #[arg(long)]
        json: bool,
    },

    /// Record the installed version of every managed package to locks.json, so
    /// `sync --locked` reproduces those exact versions on another machine
    Lock,

    /// Parallel search across all searchable repositories
    Search {
        /// Search query string
        query: String,

        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },

    /// Refresh repository metadata for all backends
    Update,

    /// Upgrade managed packages to their latest versions
    Upgrade {
        /// Limit upgrade to a specific profile
        #[arg(long)]
        profile: Option<String>,

        /// Limit upgrade to a specific module
        #[arg(long)]
        module: Option<String>,

        /// Limit upgrade to a specific group defined in config
        #[arg(long)]
        group: Option<String>,

        /// Output potential changes as JSON (requires --dry-run)
        #[arg(long)]
        json: bool,

        /// Health-gated upgrade: snapshot first, then run --test after upgrading, and
        /// automatically roll back to the snapshot if the test fails
        #[arg(long)]
        canary: bool,

        /// Health-check command run after a --canary upgrade (non-zero exit = roll back)
        #[arg(long)]
        test: Option<String>,
    },

    /// List all installed packages
    List {
        /// Filter results by a specific backend
        #[arg(short, long)]
        backend: Option<String>,

        /// Output the list in machine-readable JSON format
        #[arg(long)]
        json: bool,
    },

    /// Fetch detailed metadata and properties for a specific package
    Info {
        /// Name of the package
        package: String,
    },

    /// Imperatively install one or more packages
    Install {
        /// Package strings (e.g. "apt:curl", "cargo:exa")
        packages: Vec<String>,

        /// Output the resulting changes as JSON (requires --dry-run)
        #[arg(long)]
        json: bool,
    },

    /// Imperatively remove one or more packages
    Remove {
        /// Names of packages to purge
        packages: Vec<String>,

        /// Output the resulting changes as JSON (requires --dry-run)
        #[arg(long)]
        json: bool,
    },

    /// Manage source repositories (PPA, Taps, Buckets, etc.)
    Repo(RepoArgs),

    /// Perform system health, snapshot, and backend readiness check
    Doctor,

    /// Ingest manually installed packages into LiNix management
    Migrate,

    /// Move a package from one backend to another (e.g. apt -> snap)
    Teleport {
        /// Name of the package to move
        package: String,
        /// Name of the destination backend
        to: String,
    },

    /// Enter an ephemeral shell with specific packages loaded
    Shell {
        /// Packages to load into the ghost shell
        packages: Vec<String>,
    },

    /// Interactive snapshot gallery and system rollback
    Undo,

    /// Activate one or more profiles: add each to the active set and converge the system.
    /// Several profiles can be active at once — their package sets are unioned. Live; no reboot.
    Activate {
        /// Profile name(s) to activate
        #[arg(required = true)]
        profiles: Vec<String>,
    },

    /// Deactivate one or more profiles: drop each from the active set and converge, removing
    /// packages no longer required by any remaining active profile. Live; no reboot.
    Deactivate {
        /// Profile name(s) to deactivate
        #[arg(required = true)]
        profiles: Vec<String>,
    },

    /// Manage system profiles / identities (list, show, create, save, switch, active)
    Profile(ProfileArgs),

    // --- NEW FOR 3.6.0 ---
    /// Reusable package modules (@module syntax)
    Module(ModuleArgs),

    /// System snapshots and atomic rollbacks
    Snapshot(SnapshotArgs),

    /// Generations: list saved system states, pin them, or roll back to one
    Generation(GenerationArgs),

    /// Roll back to a saved generation by id: realizes its package set on the system
    /// (drive backends), and for a full rollback also restores its manifests. Scope with
    /// `--package` and/or the global `--backend` to roll back just part of the system.
    Rollback {
        /// Generation id to restore (see `linix generation list`)
        id: String,
        /// Only roll back this package (name or backend:name)
        #[arg(long)]
        package: Option<String>,
    },

    /// Manage package leases and expirations
    Lease(LeaseArgs),

    /// Native system-level task scheduling (systemd, launchd, task-scheduler)
    Schedule(ScheduleArgs),

    /// Inspect and scaffold the LiNix application configuration file
    Config(ConfigArgs),

    /// Scaffold the LiNix directory structure (groups, modules, data dirs) and a
    /// starter manifest, so a fresh machine is ready for `linix sync`
    Init {
        /// Reset the starter manifest even if one already exists
        #[arg(long)]
        force: bool,
    },

    /// Scan every managed package across all backends for known security
    /// vulnerabilities (via the OSV.dev database)
    Audit {
        /// Output the findings as JSON
        #[arg(long)]
        json: bool,
    },

    /// Emit a CycloneDX software bill of materials (SBOM) spanning every backend
    Sbom,

    /// Explain why a package is installed: its provenance and what depends on it
    Why {
        /// Package name (optionally `backend:name`)
        package: String,
    },

    /// Find which system snapshot first breaks a test command (system time-travel bisect).
    /// Restores snapshots and runs --test to converge on the change that introduced a
    /// regression. Filesystem-restore backends may require a reboot between steps.
    Bisect {
        /// Command whose success (exit 0) means "good" and failure means "broken"
        #[arg(long)]
        test: String,

        /// Skip the interactive confirmation before restoring snapshots
        #[arg(short, long)]
        yes: bool,
    },

    /// Replicate another machine's managed packages onto this one over SSH, translating
    /// backends per-OS where needed (e.g. apt:ripgrep -> brew:ripgrep on macOS)
    Clone {
        /// SSH destination running LiNix, e.g. user@host
        host: String,

        /// Preview the translated plan without installing
        #[arg(long)]
        dry_run: bool,
    },

    /// Compare a set of machines over SSH against your manifests and report drift
    Fleet(FleetArgs),

    /// Check the desired system state against your policy rules (policy.toml)
    Policy,

    /// Generate a shell completion script (bash, zsh, fish, powershell, elvish)
    Completions {
        /// Target shell
        shell: Shell,
    },
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Write a commented default config.toml (refuses to overwrite unless --force)
    Init {
        /// Overwrite an existing config file
        #[arg(long)]
        force: bool,
    },
    /// Print the resolved configuration file path
    Path,
    /// Print the active configuration and its source (file or built-in defaults)
    Show,
}

#[derive(Args, Debug)]
pub struct RepoArgs {
    #[command(subcommand)]
    pub command: RepoCommand,
}

#[derive(Subcommand, Debug)]
pub enum RepoCommand {
    /// Add a new source repository
    Add {
        name: String,
        url: String,
        #[arg(short, long)]
        backend: Option<String>,
    },
    /// Remove an existing source repository
    Remove {
        name: String,
        #[arg(short, long)]
        backend: Option<String>,
    },
    /// List all configured repositories for a backend
    List {
        #[arg(short, long)]
        backend: Option<String>,
    },
}

#[derive(Args, Debug)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub command: ProfileCommand,
}

#[derive(Subcommand, Debug)]
pub enum ProfileCommand {
    /// List all defined profiles (a ★ marks the currently-active ones)
    List,
    /// Show the resolved package set a profile expands to (after include/exclude/-pkg)
    Show { name: String },
    /// Scaffold a new, empty profile definition file
    Create { name: String },
    /// Save the current desired state as a new standalone profile
    Save { name: String },
    /// Exclusively switch to a profile (deactivate all others), then converge
    Switch { name: String },
    /// List only the currently-active profiles
    Active,
}

#[derive(Args, Debug)]
pub struct ModuleArgs {
    #[command(subcommand)]
    pub command: ModuleCommand,
}

#[derive(Subcommand, Debug)]
pub enum ModuleCommand {
    /// List all available reusable modules
    List,
    /// Display the contents of a specific module
    Show { name: String },
    /// Create a new module interactively
    Create { name: String },
}

#[derive(Args, Debug)]
pub struct SnapshotArgs {
    #[command(subcommand)]
    pub command: SnapshotCommand,
}

#[derive(Subcommand, Debug)]
pub enum SnapshotCommand {
    /// List all system-level snapshots
    List,
    /// Prune snapshots based on age and count limits defined in config
    Prune {
        /// Force removal without verification
        #[arg(long)]
        force: bool,
    },
}

#[derive(Args, Debug)]
pub struct GenerationArgs {
    #[command(subcommand)]
    pub command: GenerationCommand,
}

#[derive(Subcommand, Debug)]
pub enum GenerationCommand {
    /// List saved generations (newest first)
    List,
    /// Roll back to a generation: realize its package set (and, for a full rollback,
    /// restore its manifests). Scope with `--package` / the global `--backend`.
    Rollback {
        /// Generation id (see `list`)
        id: String,
        /// Only roll back this package (name or backend:name)
        #[arg(long)]
        package: Option<String>,
    },
    /// Pin a generation so retention never deletes it
    Pin {
        /// Generation id
        id: String,
    },
    /// Remove a generation's pin
    Unpin {
        /// Generation id
        id: String,
    },
}

#[derive(Args, Debug)]
pub struct LeaseArgs {
    #[command(subcommand)]
    pub command: LeaseCommand,
}

#[derive(Subcommand, Debug)]
pub enum LeaseCommand {
    /// List all packages with active leases and their expiration times
    List,
    /// Set or update the lease duration for a managed package
    Set {
        /// Package identifier (backend:name)
        package: String,
        /// Duration string (e.g. "30d", "2h", "15m")
        #[arg(short, long)]
        duration: String,
    },
}

#[derive(Args, Debug)]
pub struct ScheduleArgs {
    #[command(subcommand)]
    pub command: ScheduleCommand,
}

#[derive(Subcommand, Debug)]
pub enum ScheduleCommand {
    /// Add a new background task to the system scheduler
    Add {
        /// Unique name for the scheduled task
        name: String,
        /// Cron-style execution string (e.g. "0 2 * * *")
        #[arg(long)]
        cron: String,
        /// Command to execute within LiNix (e.g. "upgrade --profile dev")
        #[arg(long)]
        command: String,
        /// Notification channel (desktop, email, or none)
        #[arg(long)]
        notification: Option<String>,
    },
    /// List all tasks currently registered in the native scheduler
    List,
    /// Remove a task from the native scheduler
    Remove { name: String },
}

#[derive(Args, Debug)]
pub struct FleetArgs {
    /// SSH destinations (user@host ...). If omitted, falls back to config `fleet_hosts`.
    pub hosts: Vec<String>,

    /// After reporting drift, run `linix sync` on each machine to reconcile it
    #[arg(long)]
    pub sync: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    Elvish,
    Nushell,
}

impl Shell {
    /// Map to a `clap_complete` built-in generator. Returns `None` for shells whose
    /// generator lives in a dedicated crate (NuShell → `clap_complete_nushell`);
    /// the completions command handles those separately.
    pub fn builtin(self) -> Option<clap_complete::Shell> {
        match self {
            Shell::Bash => Some(clap_complete::Shell::Bash),
            Shell::Zsh => Some(clap_complete::Shell::Zsh),
            Shell::Fish => Some(clap_complete::Shell::Fish),
            Shell::PowerShell => Some(clap_complete::Shell::PowerShell),
            Shell::Elvish => Some(clap_complete::Shell::Elvish),
            Shell::Nushell => None,
        }
    }
}
