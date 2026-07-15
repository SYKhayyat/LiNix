pub mod args;

/// Re-export all primary CLI structures for binary consumption.
///
/// Resolves E0432: Ensuring 3.6.0 subcommands are visible to the crate root.
pub use args::{
    Cli,
    Commands,
    ConfigArgs,
    ConfigCommand,
    FleetArgs,
    GenerationArgs,
    GenerationCommand,
    GitArgs,
    GitCommand,
    HooksArgs,
    HooksCommand,
    LeaseArgs,
    LeaseCommand,
    ManagedArgs,
    ManagedCommand,
    // New for v3.6.0
    ModuleArgs,
    ModuleCommand,
    ProfileArgs,
    ProfileCommand,
    RepoArgs,
    RepoCommand,
    ScheduleArgs,
    ScheduleCommand,
    ServiceArgs,
    ServiceCommand,
    Shell,
    SnapshotArgs,
    SnapshotCommand,
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
/// - NuShell
///
/// Usage: `linix completions <shell>`
pub fn generate_completions(shell: args::Shell, cmd: &mut Command) {
    let mut out = std::io::stdout();
    match shell.builtin() {
        Some(builtin) => clap_complete::generate(builtin, cmd, "linix", &mut out),
        // NuShell's generator lives in its own crate but plugs into the same API.
        None => clap_complete::generate(clap_complete_nushell::Nushell, cmd, "linix", &mut out),
    }
}
