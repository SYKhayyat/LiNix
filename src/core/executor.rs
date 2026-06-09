use crate::core::{Error, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use fs2::FileExt;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs::File;
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

/// A mock Output for dry-run and testing modes.
/// Fulfills Phase 9.4: Provides inspection methods for testing.
#[derive(Debug, Clone, Default)]
pub struct DryRunOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl DryRunOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the captured stdout as a UTF-8 String.
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    /// Returns the captured stderr as a UTF-8 String.
    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }
}

/// Conversion to make DryRunOutput behave like a real OS process output.
impl From<DryRunOutput> for StdOutput {
    fn from(dry: DryRunOutput) -> Self {
        let status = if cfg!(windows) {
            StdCommand::new("cmd").args(["/C", "exit", "0"]).status().expect("Failed to create dummy exit status")
        } else {
            StdCommand::new("true").status().expect("Failed to create dummy exit status")
        };
        StdOutput { status, stdout: dry.stdout, stderr: dry.stderr }
    }
}

/// Defines a pluggable layer for low-level command execution and system queries.
#[async_trait]
pub trait ExecutionLayer: Send + Sync {
    /// Executes a command asynchronously.
    async fn execute(&self, cmd: &str, args: &[String], env: &HashMap<String, String>) -> Result<StdOutput>;
    
    /// Checks if a command exists in the system PATH.
    fn check_command(&self, cmd: &str) -> bool;

    /// Phase 1.1 Correction: Abstract symlinking to allow VFS recording and mock testing.
    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()>;
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

    fn check_command(&self, cmd: &str) -> bool {
        let check_bin = if cfg!(windows) { "where" } else { "which" };
        StdCommand::new(check_bin)
            .arg(cmd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        #[cfg(unix)]
        {
            tokio::fs::symlink(src, dst).await.map_err(|e| Error::Io(e.to_string()))
        }
        #[cfg(windows)]
        {
            if src.is_dir() {
                tokio::fs::symlink_dir(src, dst).await.map_err(|e| Error::Io(e.to_string()))
            } else {
                tokio::fs::symlink_file(src, dst).await.map_err(|e| Error::Io(e.to_string()))
            }
        }
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

    fn check_command(&self, _cmd: &str) -> bool {
        true
    }

    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        let val = format!("SYMLINK:{}", src.display());
        VFS.insert(dst.to_path_buf(), val);
        Ok(())
    }
}

/// A mock execution layer for testing without real system commands.
pub struct MockExecutor {
    pub responses: DashMap<String, Result<StdOutput>>,
    pub command_existence: DashMap<String, bool>,
}

impl MockExecutor {
    pub fn new() -> Self {
        Self { 
            responses: DashMap::new(),
            command_existence: DashMap::new(),
        }
    }

    pub fn set_response(&self, cmd_pattern: &str, response: Result<StdOutput>) {
        self.responses.insert(cmd_pattern.to_string(), response);
    }

    pub fn set_command_exists(&self, cmd: &str, exists: bool) {
        self.command_existence.insert(cmd.to_string(), exists);
    }
}

#[async_trait]
impl ExecutionLayer for MockExecutor {
    async fn execute(&self, cmd: &str, args: &[String], _env: &HashMap<String, String>) -> Result<StdOutput> {
        let full_cmd = format!("{} {}", cmd, args.join(" "));
        if let Some(res) = self.responses.get(&full_cmd) {
            return res.clone();
        }
        Ok(DryRunOutput::new().into())
    }

    fn check_command(&self, cmd: &str) -> bool {
        self.command_existence.get(cmd).map(|r| *r.value()).unwrap_or(true)
    }

    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        let val = format!("SYMLINK:{}", src.display());
        VFS.insert(dst.to_path_buf(), val);
        Ok(())
    }
}

/// The primary coordinator for all external command calls and filesystem IO.
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

    pub fn with_layer(dry_run: bool, verbose: bool, layer: Arc<dyn ExecutionLayer>) -> Self {
        Self { dry_run, verbose, inner: layer }
    }

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
        std::fs::read_to_string(path).map_err(Error::from)
    }

    pub async fn write_atomic(&self, path: &Path, content: &str) -> Result<()> {
        if self.dry_run {
            VFS.insert(path.to_path_buf(), content.to_string());
            return Ok(());
        }
        let dir = path.parent().ok_or_else(|| Error::Other("Invalid path: target has no parent directory".into()))?;
        tokio::fs::create_dir_all(dir).await.map_err(Error::from)?;
        
        let mut temp_file = tokio::task::spawn_blocking({
            let dir = dir.to_path_buf();
            move || NamedTempFile::new_in(dir)
        }).await.map_err(|e| Error::Other(e.to_string()))?.map_err(Error::from)?;
        
        temp_file.write_all(content.as_bytes()).map_err(Error::from)?;
        temp_file.persist(path).map_err(Error::from)?;
        Ok(())
    }

    /// Abstracted symlink for cross-platform and virtual filesystem support.
    pub async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        self.inner.symlink(src, dst).await
    }

    pub fn get_vfs_diff(&self) -> Vec<(PathBuf, String)> {
        VFS.iter().map(|item| (item.key().clone(), item.value().clone())).collect()
    }

    pub async fn command_exists(&self, cmd: &str) -> bool {
        self.inner.check_command(cmd)
    }

    pub fn command_exists_sync(&self, cmd: &str) -> bool {
        self.inner.check_command(cmd)
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