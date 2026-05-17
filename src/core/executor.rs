use crate::core::{Error, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use fs2::FileExt;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, ExitStatus, Output as StdOutput};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// The LockMap: A fine-grained, thread-safe locking mechanism.
static LOCK_MAP: Lazy<DashMap<String, Arc<Mutex<()>>>> = Lazy::new(DashMap::new);

/// The Virtual File System (VFS): Tracks file changes in memory during dry-runs.
static VFS: Lazy<DashMap<PathBuf, String>> = Lazy::new(DashMap::new);

/// Represents a successful exit status for dry-run mode.
#[derive(Debug)]
struct DryRunExitStatus;

impl DryRunExitStatus {
    fn success(&self) -> bool { true }
    fn code(&self) -> Option<i32> { Some(0) }
}

/// A mock Output for dry-run mode.
#[derive(Debug)]
pub struct DryRunOutput {
    status: DryRunExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl DryRunOutput {
    pub fn new() -> Self {
        Self { status: DryRunExitStatus, stdout: Vec::new(), stderr: Vec::new() }
    }
    pub fn status(&self) -> &DryRunExitStatus { &self.status }
    pub fn stdout(&self) -> &[u8] { &self.stdout }
    pub fn stderr(&self) -> &[u8] { &self.stderr }
}

impl From<DryRunOutput> for StdOutput {
    fn from(dry: DryRunOutput) -> Self {
        let true_status = if cfg!(windows) {
            StdCommand::new("cmd").args(&["/C", "exit", "0"]).status()
                .unwrap_or_else(|_| StdOutput { status: ExitStatus::default(), stdout: vec![], stderr: vec![] }.status)
        } else {
            StdCommand::new("true").status()
                .unwrap_or_else(|_| StdOutput { status: ExitStatus::default(), stdout: vec![], stderr: vec![] }.status)
        };
        StdOutput { status: true_status, stdout: dry.stdout, stderr: dry.stderr }
    }
}

/// Defines a pluggable layer for low-level command execution.
#[async_trait]
pub trait ExecutionLayer: Send + Sync {
    async fn execute(&self, cmd: &str, args: &[String], env: &HashMap<String, String>) -> Result<StdOutput>;
}

/// Dry-run execution layer.
pub struct DryRunExecutor;

#[async_trait]
impl ExecutionLayer for DryRunExecutor {
    async fn execute(&self, cmd: &str, args: &[String], _env: &HashMap<String, String>) -> Result<StdOutput> {
        info!("[DRY-RUN] Would execute: {} {}", cmd, args.join(" "));
        Ok(DryRunOutput::new().into())
    }
}

/// The base implementation that performs actual OS process spawning.
pub struct RawExecutor;

#[async_trait]
impl ExecutionLayer for RawExecutor {
    async fn execute(&self, cmd: &str, args: &[String], env: &HashMap<String, String>) -> Result<StdOutput> {
        let mut command = Command::new(cmd);
        command.args(args).envs(env);

        if std::io::stdin().is_terminal() {
            command.stdin(std::process::Stdio::inherit())
                   .stdout(std::process::Stdio::inherit())
                   .stderr(std::process::Stdio::inherit());
        } else {
            command.stdin(std::process::Stdio::null())
                   .stdout(std::process::Stdio::piped())
                   .stderr(std::process::Stdio::piped());
        }

        let output = command.spawn()
            .map_err(|e| Error::CommandFailed(format!("Failed to spawn {}: {}", cmd, e)))?
            .wait_with_output().await?;
        Ok(output)
    }
}

/// The primary coordinator for all external command calls and filesystem IO.
/// FIX #15: No longer derives Clone - use Arc if shared ownership is needed.
#[derive(Debug)]
pub struct CommandExecutor {
    pub dry_run: bool,
    pub verbose: bool,
    inner: Arc<dyn ExecutionLayer>,
}

impl CommandExecutor {
    pub fn new(dry_run: bool, verbose: bool) -> Self {
        let inner: Arc<dyn ExecutionLayer> = if dry_run {
            Arc::new(DryRunExecutor)
        } else {
            Arc::new(RawExecutor)
        };
        Self { dry_run, verbose, inner }
    }
    
    /// Creates a new instance with the same settings (but new Arc).
    /// Use this instead of Clone.
    pub fn duplicate(&self) -> Self {
        Self::new(self.dry_run, self.verbose)
    }
    
    /// Wraps in Arc for shared ownership.
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    pub fn is_root() -> bool {
        #[cfg(unix)] { unsafe { libc::geteuid() == 0 } }
        #[cfg(windows)] { false }
    }

    fn hardened_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("LC_ALL".into(), "C".into());
        env.insert("LANG".into(), "C".into());
        env.insert("LANGUAGE".into(), "C".into());
        env.insert("DEBIAN_FRONTEND".into(), "noninteractive".into());
        env
    }

    pub async fn run(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<StdOutput> {
        let mut final_cmd = cmd.to_string();
        let mut final_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        if sudo && !cfg!(windows) && !Self::is_root() {
            final_args.insert(0, final_cmd);
            final_cmd = "sudo".to_string();
        }

        let output = self.inner.execute(&final_cmd, &final_args, &self.hardened_env()).await?;

        if !output.status.success() && !self.dry_run {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::CommandFailed(format!("Command '{}' failed: {}", cmd, stderr.trim())));
        }
        Ok(output)
    }

    pub async fn run_output(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<String> {
        let output = self.run(cmd, args, sudo).await?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub async fn run_exclusive(&self, lock_key: &str, cmd: &str, args: &[&str], sudo: bool) -> Result<StdOutput> {
        let mutex = LOCK_MAP.entry(lock_key.to_string()).or_insert_with(|| Arc::new(Mutex::new(()))).clone();
        let _thread_guard = mutex.lock().await;

        if self.dry_run { return self.run(cmd, args, sudo).await; }

        let lock_path = std::env::temp_dir().join(format!("linix_{}.lock", lock_key));
        let lock_file = File::create(&lock_path)?;
        lock_file.lock_exclusive()?;
        let result = self.run(cmd, args, sudo).await;
        let _ = lock_file.unlock();
        result
    }

    pub async fn read_file(&self, path: &Path) -> Result<String> {
        if self.dry_run {
            if let Some(content) = VFS.get(path) {
                debug!("VFS: Serving virtual content for {:?}", path);
                return Ok(content.clone());
            }
        }
        if !path.exists() {
            return Err(Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "File not found")));
        }
        tokio::fs::read_to_string(path).await.map_err(Error::Io)
    }

    pub fn read_file_sync(&self, path: &Path) -> Result<String> {
        if self.dry_run {
            if let Some(content) = VFS.get(path) {
                debug!("VFS: Serving virtual content for {:?}", path);
                return Ok(content.clone());
            }
        }
        if !path.exists() {
            return Err(Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "File not found")));
        }
        fs::read_to_string(path).map_err(Error::Io)
    }

    pub async fn write_atomic(&self, path: &Path, content: &str) -> Result<()> {
        if self.dry_run {
            info!("[DRY-RUN] VFS: Staging write to {:?}", path);
            VFS.insert(path.to_path_buf(), content.to_string());
            return Ok(());
        }
        let dir = path.parent().ok_or_else(|| Error::Other("Invalid path".into()))?;
        if !dir.exists() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let mut temp_file = NamedTempFile::new_in(dir)?;
        temp_file.write_all(content.as_bytes())?;
        temp_file.flush()?;
        temp_file.as_file().sync_all()?;
        temp_file.persist(path).map_err(|e| Error::Persist(e.to_string()))?;
        Ok(())
    }

    pub fn write_atomic_sync(&self, path: &Path, content: &str) -> Result<()> {
        if self.dry_run {
            info!("[DRY-RUN] VFS: Staging write to {:?}", path);
            VFS.insert(path.to_path_buf(), content.to_string());
            return Ok(());
        }
        let dir = path.parent().ok_or_else(|| Error::Other("Invalid path".into()))?;
        if !dir.exists() {
            fs::create_dir_all(dir)?;
        }
        let mut temp_file = NamedTempFile::new_in(dir)?;
        temp_file.write_all(content.as_bytes())?;
        temp_file.flush()?;
        temp_file.as_file().sync_all()?;
        temp_file.persist(path).map_err(|e| Error::Persist(e.to_string()))?;
        Ok(())
    }

    pub async fn create_dir_all(&self, path: &Path) -> Result<()> {
        if self.dry_run {
            info!("[DRY-RUN] Would create directory: {:?}", path);
            return Ok(());
        }
        tokio::fs::create_dir_all(path).await.map_err(Error::Io)
    }

    pub async fn remove_file(&self, path: &Path) -> Result<()> {
        if self.dry_run {
            info!("[DRY-RUN] Would remove file: {:?}", path);
            return Ok(());
        }
        tokio::fs::remove_file(path).await.map_err(Error::Io)
    }

    pub async fn remove_dir_all(&self, path: &Path) -> Result<()> {
        if self.dry_run {
            info!("[DRY-RUN] Would remove directory: {:?}", path);
            return Ok(());
        }
        tokio::fs::remove_dir_all(path).await.map_err(Error::Io)
    }

    pub async fn file_exists(&self, path: &Path) -> bool {
        if self.dry_run {
            return VFS.contains_key(path);
        }
        tokio::fs::metadata(path).await.is_ok()
    }

    pub fn get_vfs_diff(&self) -> Vec<(PathBuf, String)> {
        VFS.iter().map(|entry| (entry.key().clone(), entry.value().clone())).collect()
    }

    pub fn command_exists_sync(&self, cmd: &str) -> bool {
        let check_bin = if cfg!(windows) { "where" } else { "which" };
        std::process::Command::new(check_bin).arg(cmd).stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
    }

    pub async fn command_exists(&self, cmd: &str) -> bool {
        let check_bin = if cfg!(windows) { "where" } else { "which" };
        match Command::new(check_bin).arg(cmd).stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).status().await {
            Ok(status) => status.success(),
            Err(_) => false,
        }
    }

    pub async fn start_sudo_keepalive(&self) -> Option<tokio::task::JoinHandle<()>> {
        if cfg!(windows) || Self::is_root() || self.dry_run { return None; }
        Some(tokio::spawn(async move {
            loop {
                let _ = Command::new("sudo").arg("-v").stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null()).status().await;
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        }))
    }
}

// FIX #15: No Clone implementation - use Arc::clone or duplicate() instead

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_command_executor_creation() {
        let executor = CommandExecutor::new(true, false);
        assert!(executor.dry_run);
        assert!(!executor.verbose);
        
        let duplicate = executor.duplicate();
        assert_eq!(duplicate.dry_run, executor.dry_run);
    }
}