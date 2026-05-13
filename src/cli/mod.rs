pub mod args;

pub use args::{Cli, Commands, Shell, RepoArgs, RepoCommand};

/// Generates shell completion scripts for the LiNix CLI.
/// Supports Bash, Zsh, Fish, PowerShell, and Elvish.
pub fn generate_completions(shell: clap_complete::Shell, cmd: &mut clap::Command) {
    clap_complete::generate(
        shell,
        cmd,
        "linix",
        &mut std::io::stdout(),
    );
}