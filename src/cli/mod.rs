pub mod args;

/// Re-export all primary CLI structures for binary consumption.
/// 
/// Resolves E0432: Ensuring 3.6.0 subcommands are visible to the crate root.
pub use args::{
    Cli, 
    Commands, 
    Shell, 
    RepoArgs, 
    RepoCommand,
    // New for v3.6.0
    ModuleArgs,
    ModuleCommand,
    SnapshotArgs,
    SnapshotCommand,
    LeaseArgs,
    LeaseCommand,
    ScheduleArgs,
    ScheduleCommand,
};

use clap::Command;

/// Generates shell completion scripts for the LiNix CLI.
/// 
/// Supported Shells:
/// - Bash
/// - Zsh
/// - Fish
/// - PowerShell
/// - Elvish
/// 
/// Usage: `linix completions <shell>`
pub fn generate_completions(shell: clap_complete::Shell, cmd: &mut Command) {
    clap_complete::generate(
        shell,
        cmd,
        "linix",
        &mut std::io::stdout(),
    );
}