use clap::{Parser, Subcommand, ValueEnum, Args};
use std::path::PathBuf;

/// LiNix - Universal Mission-Critical Package Manager
/// High-performance, DAG-based orchestration for 33+ backends.
#[derive(Parser, Debug)]
#[command(name = "linix", version = "3.5.0", about = "Universal Mission-Critical Package Manager")]
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
        /// Force strict version matching
        #[arg(long)] 
        locked: bool 
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

    /// Roadmap 3.3: Identify all packages installed on the OS but not managed by LiNix
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

    /// Upgrade all managed packages to their latest versions (auto-snapshot)
    Upgrade,

    /// List all installed packages
    List { 
        /// Filter results by a specific backend
        #[arg(short, long)] 
        backend: Option<String> 
    },

    /// Fetch detailed metadata and properties for a specific package
    Info { 
        /// Name of the package
        package: String 
    },

    /// Imperatively install one or more packages
    Install { 
        /// Package strings (e.g. "apt:curl", "cargo:exa")
        packages: Vec<String> 
    },

    /// Imperatively remove one or more packages
    Remove { 
        /// Names of packages to purge
        packages: Vec<String> 
    },

    /// Manage source repositories (PPA, Taps, Buckets, etc.)
    Repo(RepoArgs),

    /// Perform system health, snapshot, and backend readiness check
    Doctor,

    // --- NEW VARIANTS FOR VERSION 3.5.0 ---

    /// Point 3: Ingest manually installed packages into LiNix management
    Migrate,

    /// Point 5: Move a package from one backend to another (e.g. apt -> snap)
    Teleport {
        /// Name of the package to move
        package: String,
        /// Name of the destination backend
        to: String,
    },

    /// Point 19: Enter an ephemeral shell with specific packages loaded
    Shell {
        /// Packages to load into the ghost shell
        packages: Vec<String>,
    },

    /// Point 12: Interactive snapshot gallery and system rollback
    Undo,

    /// Point 18: Swap between different system configurations (identities)
    Profile {
        /// Name of the profile to switch to
        name: String,
    },
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