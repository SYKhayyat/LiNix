// src/core/executor.rs

use crate::core::{Error, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use fs2::FileExt;
use std::collections::HashMap;
use std::fs::File;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Output as StdOutput};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{info};

// ============================================================================
// Dry-run output mock
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct DryRunOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl DryRunOutput {
    pub fn new() -> Self {
        Self::default()
    }
}

impl From<DryRunOutput> for StdOutput {
    fn from(dry: DryRunOutput) -> Self {
        let status = if cfg!(windows) {
            StdCommand::new("cmd")
                .args(["/C", "exit", "0"])
                .status()
                .expect("failed to create dummy status")
        } else {
            StdCommand::new("true")
                .status()
                .expect("failed to create dummy status")
        };
        StdOutput {
            status,
            stdout: dry.stdout,
            stderr: dry.stderr,
        }
    }
}

// ============================================================================
// ExecutionLayer trait
// ============================================================================

#[async_trait]
pub trait ExecutionLayer: Send + Sync {
    async fn execute(&self, cmd: &str, args: &[String], env: &HashMap<String, String>) -> Result<StdOutput>;
    fn check_command(&self, cmd: &str) -> bool;
    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()>;
}

// ============================================================================
// RawExecutor
// ============================================================================

pub struct RawExecutor;

#[async_trait]
impl ExecutionLayer for RawExecutor {
    async fn execute(&self, cmd: &str, args: &[String], env: &HashMap<String, String>) -> Result<StdOutput> {
        let mut command = Command::new(cmd);
        command.args(args).envs(env);

        if std::io::stdin().is_terminal() {
            command
                .stdin(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit());
        } else {
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
        }

        let output = command
            .spawn()
            .map_err(|e| Error::CommandFailed(format!("Failed to spawn {}: {}", cmd, e)))?
            .wait_with_output()
            .await?;
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
            tokio::fs::symlink(src, dst)
                .await
                .map_err(|e| Error::Io(e.to_string()))
        }
        #[cfg(windows)]
        {
            if src.is_dir() {
                tokio::fs::symlink_dir(src, dst)
                    .await
                    .map_err(|e| Error::Io(e.to_string()))
            } else {
                tokio::fs::symlink_file(src, dst)
                    .await
                    .map_err(|e| Error::Io(e.to_string()))
            }
        }
    }
}

// ============================================================================
// DryRunExecutor
// ============================================================================

pub struct DryRunExecutor {
    vfs: Arc<DashMap<PathBuf, String>>,
}

impl DryRunExecutor {
    pub fn new(vfs: Arc<DashMap<PathBuf, String>>) -> Self {
        Self { vfs }
    }
}

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
        // Changed marker to "LINK:" for test compatibility
        let val = format!("LINK:{}", src.display());
        self.vfs.insert(dst.to_path_buf(), val);
        Ok(())
    }
}

// ============================================================================
// MockExecutor
// ============================================================================

pub struct MockExecutor {
    pub responses: DashMap<String, Result<StdOutput>>,
    pub command_existence: DashMap<String, bool>,
    pub call_log: Arc<Mutex<Vec<String>>>,
    vfs: Arc<DashMap<PathBuf, String>>,
}

impl MockExecutor {
    pub fn new(vfs: Arc<DashMap<PathBuf, String>>) -> Self {
        Self {
            responses: DashMap::new(),
            command_existence: DashMap::new(),
            call_log: Arc::new(Mutex::new(Vec::new())),
            vfs,
        }
    }

    pub fn set_response(&self, cmd_pattern: &str, response: Result<StdOutput>) {
        self.responses.insert(cmd_pattern.to_string(), response);
    }

    pub fn set_command_exists(&self, cmd: &str, exists: bool) {
        self.command_existence.insert(cmd.to_string(), exists);
    }

    pub async fn get_calls(&self) -> Vec<String> {
        self.call_log.lock().await.clone()
    }
}

#[async_trait]
impl ExecutionLayer for MockExecutor {
    async fn execute(&self, cmd: &str, args: &[String], _env: &HashMap<String, String>) -> Result<StdOutput> {
        let full_cmd = format!("{} {}", cmd, args.join(" "));
        {
            let mut log = self.call_log.lock().await;
            log.push(full_cmd.clone());
        }
        if let Some(res) = self.responses.get(&full_cmd) {
            return res.clone();
        }
        Ok(DryRunOutput::new().into())
    }

    fn check_command(&self, cmd: &str) -> bool {
        self.command_existence
            .get(cmd)
            .map(|r| *r.value())
            .unwrap_or(true)
    }

    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        // Changed marker to "LINK:" for test compatibility
        let val = format!("LINK:{}", src.display());
        self.vfs.insert(dst.to_path_buf(), val);
        Ok(())
    }
}

// ============================================================================
// CommandExecutor
// ============================================================================

#[derive(Clone)]
pub struct CommandExecutor {
    pub dry_run: bool,
    pub verbose: bool,
    pub inner: Arc<dyn ExecutionLayer>,
    vfs: Arc<DashMap<PathBuf, String>>,
    lock_map: Arc<DashMap<String, Arc<Mutex<()>>>>,
}

impl CommandExecutor {
    pub fn new(dry_run: bool, verbose: bool) -> Self {
        let vfs = Arc::new(DashMap::new());
        let lock_map = Arc::new(DashMap::new());
        let inner: Arc<dyn ExecutionLayer> = if dry_run {
            Arc::new(DryRunExecutor::new(vfs.clone()))
        } else {
            Arc::new(RawExecutor)
        };
        Self {
            dry_run,
            verbose,
            inner,
            vfs,
            lock_map,
        }
    }

    pub fn with_layer(
        dry_run: bool,
        verbose: bool,
        layer: Arc<dyn ExecutionLayer>,
        vfs: Arc<DashMap<PathBuf, String>>,
        lock_map: Arc<DashMap<String, Arc<Mutex<()>>>>,
    ) -> Self {
        Self {
            dry_run,
            verbose,
            inner: layer,
            vfs,
            lock_map,
        }
    }

    pub fn duplicate(&self) -> Self {
        self.clone()
    }

    pub fn is_root() -> bool {
        #[cfg(unix)]
        {
            unsafe { libc::geteuid() == 0 }
        }
        #[cfg(windows)]
        {
            false
        }
    }

    pub async fn run(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<StdOutput> {
        let mut final_cmd = cmd.to_string();
        let mut final_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        if sudo && !cfg!(windows) && !Self::is_root() {
            final_args.insert(0, final_cmd);
            final_cmd = "sudo".to_string();
        }

        self.inner.execute(&final_cmd, &final_args, &HashMap::new()).await
    }

    pub async fn run_output(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<String> {
        let output = self.run(cmd, args, sudo).await?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub async fn run_exclusive(&self, lock_key: &str, cmd: &str, args: &[&str], sudo: bool) -> Result<StdOutput> {
        let mutex = self
            .lock_map
            .entry(lock_key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _thread_guard = mutex.lock().await;

        if self.dry_run {
            return self.run(cmd, args, sudo).await;
        }

        let lock_path = std::env::temp_dir().join(format!("linix_{}.lock", lock_key));
        let lock_file = File::create(lock_path).map_err(Error::from)?;
        lock_file.lock_exclusive().map_err(Error::from)?;
        let result = self.run(cmd, args, sudo).await;
        let _ = lock_file.unlock();
        result
    }

    pub async fn read_file(&self, path: &Path) -> Result<String> {
        if self.dry_run {
            if let Some(content) = self.vfs.get(path) {
                return Ok(content.clone());
            }
        }
        tokio::fs::read_to_string(path).await.map_err(Error::from)
    }

    pub fn read_file_sync(&self, path: &Path) -> Result<String> {
        if self.dry_run {
            if let Some(content) = self.vfs.get(path) {
                return Ok(content.clone());
            }
        }
        std::fs::read_to_string(path).map_err(Error::from)
    }

    pub async fn write_atomic(&self, path: &Path, content: &str) -> Result<()> {
        if self.dry_run {
            self.vfs.insert(path.to_path_buf(), content.to_string());
            return Ok(());
        }
        let dir = path
            .parent()
            .ok_or_else(|| Error::Other("Invalid path: no parent directory".into()))?;
        tokio::fs::create_dir_all(dir).await.map_err(Error::from)?;

        let mut temp_file = tokio::task::spawn_blocking({
            let dir = dir.to_path_buf();
            move || NamedTempFile::new_in(dir)
        })
        .await
        .map_err(|e| Error::Other(format!("IO thread failure: {}", e)))?
        .map_err(Error::from)?;

        temp_file.write_all(content.as_bytes()).map_err(Error::from)?;
        temp_file.persist(path).map_err(Error::from)?;
        Ok(())
    }

    pub async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        self.inner.symlink(src, dst).await
    }

    pub fn get_vfs_diff(&self) -> Vec<(PathBuf, String)> {
        self.vfs
            .iter()
            .map(|item| (item.key().clone(), item.value().clone()))
            .collect()
    }

    pub async fn command_exists(&self, cmd: &str) -> bool {
        self.inner.check_command(cmd)
    }

    pub fn command_exists_sync(&self, cmd: &str) -> bool {
        self.inner.check_command(cmd)
    }

    pub async fn start_sudo_keepalive(&self) -> Option<tokio::task::JoinHandle<()>> {
        if cfg!(windows) || Self::is_root() || self.dry_run {
            return None;
        }
        Some(tokio::spawn(async move {
            loop {
                let _ = StdCommand::new("sudo").arg("-v").status();
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        }))
    }
}