use std::process::Stdio;
use tokio::process::Command;

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

pub fn command_exists_sync(cmd: &str) -> bool {
    let check_bin = if cfg!(windows) { "where" } else { "which" };
    std::process::Command::new(check_bin)
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub async fn get_command_version(cmd: &str) -> Option<String> {
    let output = Command::new(cmd).arg("--version").output().await.ok()?;

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
