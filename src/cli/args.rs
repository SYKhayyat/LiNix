// C:\Users\Administrator\Videos\Nexus\linix\src\cli\args.rs
use clap::{Parser, Subcommand, ValueEnum, Args};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "linix")]
#[command(author = "LiNix Contributors")]
#[command(version = "3.0.0")]
#[command(about = "Universal package manager with multi-backend support")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(short = 'n', long, global = true)]
    pub dry_run: bool,
    #[arg(short, long, global = true)]
    pub yes: bool,
    #[arg(short, long, global = true)]
    pub backend: Option<String>,
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,
    #[arg(short, long, global = true)]
    pub groups_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub remove_bloatware: bool,
    #[arg(long, global = true, default_value = "true")]
    pub progress: bool,
    #[arg(short, long, global = true)]
    pub verbose: bool,
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Sync {
        #[arg(long)]
        locked: bool,
    },
    Clean,
    Unmanaged,
    Orphans,
    Search { query: String },
    Update,
    Upgrade,
    List { #[arg(short, long)] backend: Option<String> },
    Info { package: String },
    Install { packages: Vec<String> },
    Remove { packages: Vec<String> },
    Backends,
    Completions { #[arg(value_enum)] shell: Shell },
    Repo(RepoArgs),
    Doctor,
    Rollback {
        /// Timestamp or ID of the snapshot to rollback to. If omitted, lists available snapshots.
        snapshot: Option<String>,
    },
}

#[derive(Args, Debug)]
pub struct RepoArgs {
    #[clap(subcommand)]
    pub command: RepoCommand,
}

#[derive(Subcommand, Debug)]
pub enum RepoCommand {
    Add {
        name: String,
        url: String,
        #[arg(short, long)]
        backend: Option<String>,
    },
    Remove {
        name: String,
        #[arg(short, long)]
        backend: Option<String>,
    },
    List {
        #[arg(short, long)]
        backend: Option<String>,
    },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash, Zsh, Fish, PowerShell, Elvish,
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