use std::process::Stdio;
use tokio::process::Command;

/// Check if a command exists in PATH
pub async fn command_exists(cmd: &str) -> bool {
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

/// Get command version
pub async fn get_command_version(cmd: &str) -> Option<String> {
    let output = Command::new(cmd).arg("--version").output().await.ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Run a simple command and return stdout
pub async fn run_simple(cmd: &str, args: &[&str]) -> crate::core::Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(crate::core::Error::CommandFailed(format!(
            "{} failed: {}",
            cmd,
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

/// Split a command string into command and arguments
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
    async fn test_command_exists() {
        #[cfg(unix)]
        {
            assert!(command_exists("ls").await);
            assert!(!command_exists("nonexistent_command_xyz123").await);
        }

        #[cfg(windows)]
        {
            assert!(command_exists("cmd").await);
        }
    }

    #[test]
    fn test_split_command() {
        let (cmd, args) = split_command("apt install -y package").unwrap();
        assert_eq!(cmd, "apt");
        assert_eq!(args, vec!["install", "-y", "package"]);

        assert!(split_command("").is_none());
    }
}
