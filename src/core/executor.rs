use crate::core::{Error, Result};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, info};

/// Executes system commands with dry-run and sudo support
#[derive(Clone)]
pub struct CommandExecutor {
    dry_run: bool,
    verbose: bool,
}

impl CommandExecutor {
    /// Create a new command executor
    pub fn new(dry_run: bool, verbose: bool) -> Self {
        Self { dry_run, verbose }
    }

    /// Check if we're running as root
    #[cfg(unix)]
    pub fn is_root() -> bool {
        unsafe { libc::geteuid() == 0 }
    }

    #[cfg(not(unix))]
    pub fn is_root() -> bool {
        // On Windows, we assume admin rights if needed
        false
    }

    /// Execute a command with optional sudo
    pub async fn run(
        &self,
        cmd: &str,
        args: &[&str],
        use_sudo: bool,
    ) -> Result<std::process::Output> {
        self.run_with_env(cmd, args, use_sudo, &HashMap::new())
            .await
    }

    /// Execute a command with environment variables
    pub async fn run_with_env(
        &self,
        cmd: &str,
        args: &[&str],
        use_sudo: bool,
        env: &HashMap<String, String>,
    ) -> Result<std::process::Output> {
        let (final_cmd, final_args) = self.prepare_command(cmd, args, use_sudo);

        if self.dry_run {
            info!(
                "[DRY-RUN] Would execute: {} {}",
                final_cmd,
                final_args.join(" ")
            );
            // Return a fake successful output
            return Ok(std::process::Output {
                status: std::process::ExitStatus::default(),
                stdout: Vec::new(),
                stderr: Vec::new(),
            });
        }

        if self.verbose {
            debug!("Executing: {} {}", final_cmd, final_args.join(" "));
        }

        let mut command = Command::new(&final_cmd);
        command
            .args(&final_args)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = command
            .output()
            .await
            .map_err(|e| Error::CommandFailed(format!("Failed to execute {}: {}", final_cmd, e)))?;

        if self.verbose {
            if !output.stdout.is_empty() {
                debug!("stdout: {}", String::from_utf8_lossy(&output.stdout));
            }
            if !output.stderr.is_empty() {
                debug!("stderr: {}", String::from_utf8_lossy(&output.stderr));
            }
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::CommandFailed(format!(
                "{} failed with status {}: {}",
                final_cmd, output.status, stderr
            )));
        }

        Ok(output)
    }

    /// Execute a command and return combined output as string
    pub async fn run_output(&self, cmd: &str, args: &[&str], use_sudo: bool) -> Result<String> {
        let output = self.run(cmd, args, use_sudo).await?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Execute a command and return combined stdout+stderr
    pub async fn run_combined_output(
        &self,
        cmd: &str,
        args: &[&str],
        use_sudo: bool,
    ) -> Result<String> {
        let output = self.run(cmd, args, use_sudo).await?;
        let mut result = String::from_utf8_lossy(&output.stdout).to_string();
        result.push_str(&String::from_utf8_lossy(&output.stderr));
        Ok(result)
    }

    /// Prepare command with sudo if needed
    fn prepare_command(&self, cmd: &str, args: &[&str], use_sudo: bool) -> (String, Vec<String>) {
        #[cfg(unix)]
        {
            if use_sudo && !Self::is_root() {
                let mut sudo_args = vec![cmd.to_string()];
                sudo_args.extend(args.iter().map(|s| s.to_string()));
                return ("sudo".to_string(), sudo_args);
            }
        }

        #[cfg(not(unix))]
        {
            let _ = use_sudo; // Ignore on Windows
        }

        (
            cmd.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        )
    }

    /// Check if a command exists in PATH
    pub async fn command_exists(&self, cmd: &str) -> bool {
        #[cfg(unix)]
        {
            Command::new("which")
                .arg(cmd)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|status| status.success())
                .unwrap_or(false)
        }

        #[cfg(windows)]
        {
            Command::new("where")
                .arg(cmd)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|status| status.success())
                .unwrap_or(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_command_exists() {
        let executor = CommandExecutor::new(false, false);

        // These commands should exist on any Unix-like system
        #[cfg(unix)]
        {
            assert!(executor.command_exists("ls").await);
            assert!(!executor.command_exists("nonexistent_command_xyz").await);
        }

        #[cfg(windows)]
        {
            assert!(executor.command_exists("cmd").await);
        }
    }

    #[tokio::test]
    async fn test_dry_run() {
        let executor = CommandExecutor::new(true, false);
        let result = executor.run("echo", &["test"], false).await;
        assert!(result.is_ok());
    }
}
