use std::process::Stdio;
use tokio::process::Command;
use crate::core::{Result, Error};

/// Asynchronously checks if a command exists in the system PATH.
/// Uses 'which' on Unix-like systems and 'where' on Windows.
pub async fn command_exists(cmd: &str) -> bool {
    let check_bin = if cfg!(windows) { "where" } else { "which" };
    
    match Command::new(check_bin)
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

/// Attempts to retrieve the version of a command by executing it with '--version'.
pub async fn get_command_version(cmd: &str) -> Option<String> {
    let output = Command::new(cmd)
        .arg("--version")
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let version_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if version_str.is_empty() {
            None
        } else {
            Some(version_str)
        }
    } else {
        None
    }
}

/// Executes a simple command and returns its standard output as a String.
/// This utility is intended for simple queries; for system-modifying operations, 
/// use the CommandExecutor.
pub async fn run_simple(cmd: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::CommandFailed(format!(
            "Simple command '{}' failed: {}",
            cmd,
            stderr.trim()
        )))
    }
}

/// Splits a raw command string into a command binary and its arguments.
/// Respects whitespace and handles simple splitting logic.
pub fn split_command(cmd_str: &str) -> Option<(String, Vec<String>)> {
    let parts: Vec<&str> = cmd_str.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let cmd = parts[0].to_string();
    let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();

    Some((cmd, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_command_presence() {
        #[cfg(unix)]
        {
            assert!(command_exists("sh").await);
            assert!(!command_exists("nonexistent_binary_12345").await);
        }

        #[cfg(windows)]
        {
            assert!(command_exists("cmd").await);
        }
    }

    #[test]
    fn test_command_splitter() {
        let result = split_command("apt install -y vim");
        assert!(result.is_some());
        let (cmd, args) = result.unwrap();
        assert_eq!(cmd, "apt");
        assert_eq!(args, vec!["install", "-y", "vim"]);
    }
}