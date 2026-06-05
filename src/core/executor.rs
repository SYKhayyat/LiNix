use crate::core::{Error, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use fs2::FileExt;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Output as StdOutput};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::info;

/// The LockMap: A fine-grained, thread-safe locking mechanism for backend-level mutual exclusion.
static LOCK_MAP: Lazy<DashMap<String, Arc<Mutex<()>>>> = Lazy::new(DashMap::new);

/// The Virtual File System (VFS): Tracks file changes in memory during dry-runs.
static VFS: Lazy<DashMap<PathBuf, String>> = Lazy::new(DashMap::new);

/// A mock Output for dry-run mode.
#[derive(Debug)]
pub struct DryRunOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl DryRunOutput {
    pub fn new() -> Self {
        Self { stdout: Vec::new(), stderr: Vec::new() }
    }
}

/// Conversion to make DryRunOutput behave like a real OS process output.
impl From<DryRunOutput> for StdOutput {
    fn from(dry: DryRunOutput) -> Self {
        // Create a real exit status that represents success (0)
        let status = if cfg!(windows) {
            StdCommand::new("cmd").args(["/C", "exit", "0"]).status().unwrap()
        } else {
            StdCommand::new("true").status().unwrap()
        };
        StdOutput { status, stdout: dry.stdout, stderr: dry.stderr }
    }
}

/// Defines a pluggable layer for low-level command execution.
#[async_trait]
pub trait ExecutionLayer: Send + Sync {
    async fn execute(&self, cmd: &str, args: &[String], env: &HashMap<String, String>) -> Result<StdOutput>;
}

/// Actual OS process execution layer.
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

/// Dry-run execution layer that logs instead of executing.
pub struct DryRunExecutor;

#[async_trait]
impl ExecutionLayer for DryRunExecutor {
    async fn execute(&self, cmd: &str, args: &[String], _env: &HashMap<String, String>) -> Result<StdOutput> {
        info!("[DRY-RUN] Would execute: {} {}", cmd, args.join(" "));
        Ok(DryRunOutput::new().into())
    }
}

/// The primary coordinator for all external command calls and filesystem IO.
/// Derived Clone to allow sharing across backends and parallel tasks.
#[derive(Clone)]
pub struct CommandExecutor {
    pub dry_run: bool,
    pub verbose: bool,
    inner: Arc<dyn ExecutionLayer>,
}

impl std::fmt::Debug for CommandExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandExecutor")
            .field("dry_run", &self.dry_run)
            .field("verbose", &self.verbose)
            .finish()
    }
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

    /// Provides a safe way to create a copy of the executor.
    pub fn duplicate(&self) -> Self {
        self.clone()
    }

    pub fn is_root() -> bool {
        #[cfg(unix)] { unsafe { libc::geteuid() == 0 } }
        #[cfg(windows)] { false }
    }

    fn hardened_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("LC_ALL".into(), "C".into());
        env.insert("LANG".into(), "C".into());
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
        let lock_file = File::create(lock_path).map_err(Error::from)?;
        lock_file.lock_exclusive().map_err(Error::from)?;
        let result = self.run(cmd, args, sudo).await;
        let _ = lock_file.unlock();
        result
    }

    pub async fn read_file(&self, path: &Path) -> Result<String> {
        if self.dry_run {
            if let Some(content) = VFS.get(path) {
                return Ok(content.clone());
            }
        }
        tokio::fs::read_to_string(path).await.map_err(Error::from)
    }

    pub fn read_file_sync(&self, path: &Path) -> Result<String> {
        if self.dry_run {
            if let Some(content) = VFS.get(path) {
                return Ok(content.clone());
            }
        }
        fs::read_to_string(path).map_err(Error::from)
    }

    pub async fn write_atomic(&self, path: &Path, content: &str) -> Result<()> {
        if self.dry_run {
            VFS.insert(path.to_path_buf(), content.to_string());
            return Ok(());
        }
        let dir = path.parent().ok_or_else(|| Error::Other("Invalid path".into()))?;
        tokio::fs::create_dir_all(dir).await.map_err(Error::from)?;
        
        let mut temp_file = NamedTempFile::new_in(dir).map_err(Error::from)?;
        temp_file.write_all(content.as_bytes()).map_err(Error::from)?;
        temp_file.persist(path).map_err(Error::from)?;
        Ok(())
    }

    pub fn write_atomic_sync(&self, path: &Path, content: &str) -> Result<()> {
        if self.dry_run {
            VFS.insert(path.to_path_buf(), content.to_string());
            return Ok(());
        }
        let dir = path.parent().ok_or_else(|| Error::Other("Invalid path".into()))?;
        fs::create_dir_all(dir).map_err(Error::from)?;
        
        let mut temp_file = NamedTempFile::new_in(dir).map_err(Error::from)?;
        temp_file.write_all(content.as_bytes()).map_err(Error::from)?;
        temp_file.persist(path).map_err(Error::from)?;
        Ok(())
    }

    pub async fn command_exists(&self, cmd: &str) -> bool {
        let check_bin = if cfg!(windows) { "where" } else { "which" };
        match Command::new(check_bin).arg(cmd).status().await {
            Ok(status) => status.success(),
            Err(_) => false,
        }
    }

    pub fn command_exists_sync(&self, cmd: &str) -> bool {
        let check_bin = if cfg!(windows) { "where" } else { "which" };
        StdCommand::new(check_bin).arg(cmd).status().map(|s| s.success()).unwrap_or(false)
    }

    pub async fn start_sudo_keepalive(&self) -> Option<tokio::task::JoinHandle<()>> {
        if cfg!(windows) || Self::is_root() || self.dry_run { return None; }
        Some(tokio::spawn(async move {
            loop {
                let _ = Command::new("sudo").arg("-v").status().await;
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        }))
    }
}