use clap::{Parser, Subcommand, ValueEnum, Args};
use std::path::PathBuf;

/// LiNix - Universal Mission-Critical Package Manager
/// High-performance, DAG-based orchestration for 33+ backends.
/// Version 3.6.0: Consistency, Integrity, and Native Automation.
#[derive(Parser, Debug)]
#[command(name = "linix", version = "3.6.0", about = "Universal Mission-Critical Package Manager")]
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
        command: String 
    },

    /// Create a permanent high-performance Rust shim for a package
    Shim { 
        /// The name of the binary to create
        binary: String, 
        /// The source package spec (e.g. "cargo:ripgrep")
        #[arg(short, long)] 
        source: String 
    },

    /// Recover the system from an interrupted or crashed transaction (WAL)
    Heal,

    /// Perform a deep system cleanup (orphans, cache, temp files)
    Clean,

    /// Identify all packages installed on the OS but not managed by LiNix
    Unmanaged,

    /// List and remove orphaned dependencies across all backends
    Orphans,

    /// Parallel search across all searchable repositories
    Search { 
        /// Search query string
        query: String 
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
        package: String 
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

    /// Swap between different system configurations (identities)
    Profile {
        /// Name of the profile to switch to
        name: String,
    },

    // --- NEW FOR 3.6.0 ---

    /// Reusable package modules (@module syntax)
    Module(ModuleArgs),

    /// System snapshots and atomic rollbacks
    Snapshot(SnapshotArgs),

    /// Manage package leases and expirations
    Lease(LeaseArgs),

    /// Native system-level task scheduling (systemd, launchd, task-scheduler)
    Schedule(ScheduleArgs),
}

#[derive(Args, Debug)]
pub struct RepoArgs { 
    #[command(subcommand)]
    pub command: RepoCommand 
}

#[derive(Subcommand, Debug)]
pub enum RepoCommand {
    /// Add a new source repository
    Add { 
        name: String, 
        url: String, 
        #[arg(short, long)] 
        backend: Option<String> 
    },
    /// Remove an existing source repository
    Remove { 
        name: String, 
        #[arg(short, long)] 
        backend: Option<String> 
    },
    /// List all configured repositories for a backend
    List { 
        #[arg(short, long)] 
        backend: Option<String> 
    },
}

#[derive(Args, Debug)]
pub struct ModuleArgs {
    #[command(subcommand)]
    pub command: ModuleCommand
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
    pub command: SnapshotCommand
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
pub struct LeaseArgs {
    #[command(subcommand)]
    pub command: LeaseCommand
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
    pub command: ScheduleCommand
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

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum Shell { Bash, Zsh, Fish, PowerShell, Elvish }

impl From<Shell> for clap_complete::Shell {
    fn from(shell: Shell) -> Self {
        match shell {
            Shell::Bash => clap_complete::Shell::Bash,
            Shell::Zsh => clap_complete::Shell::Zsh,
            Shell::Fish => clap_complete::Shell::Fish,
            Shell::PowerShell => clap_complete::Shell::PowerShell,
            Shell::Elvish => clap_complete::Shell::Elvish,
        }
    }
}