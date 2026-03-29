use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "linix")]
#[command(author = "LiNix Contributors")]
#[command(version = "3.0.0")]
#[command(about = "Universal package manager with multi-backend support")]
#[command(long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Show what would happen without making changes
    #[arg(short = 'n', long, global = true)]
    pub dry_run: bool,

    /// Skip confirmation prompts
    #[arg(short, long, global = true)]
    pub yes: bool,

    /// Operate only on a specific backend
    #[arg(short, long, global = true)]
    pub backend: Option<String>,

    /// Config file path
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Groups directory
    #[arg(short, long, global = true)]
    pub groups_dir: Option<PathBuf>,

    /// Remove bloatware during sync
    #[arg(long, global = true)]
    pub remove_bloatware: bool,

    /// Show progress indicators
    #[arg(long, global = true, default_value = "true")]
    pub progress: bool,

    /// Verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Output in JSON format
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Sync packages according to configuration
    Sync,

    /// Remove packages not in configuration
    Clean,

    /// Show packages not in configuration
    Unmanaged,

    /// Clean orphaned dependencies
    Orphans,

    /// Search for packages across all backends
    Search {
        /// Search query
        query: String,
    },

    /// Update package databases
    Update,

    /// Upgrade all packages
    Upgrade,

    /// List installed packages
    List {
        /// Filter by backend
        #[arg(short, long)]
        backend: Option<String>,
    },

    /// Show package information
    Info {
        /// Package name
        package: String,
    },

    /// Install packages
    Install {
        /// Packages to install
        packages: Vec<String>,
    },

    /// Remove packages
    Remove {
        /// Packages to remove
        packages: Vec<String>,
    },

    /// Show available backends
    Backends,

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

impl Default for Commands {
    fn default() -> Self {
        Commands::Sync
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    #[value(name = "powershell")]
    PowerShell,
    Elvish,
}

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