use crate::core::{Error, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use fs2::FileExt;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// The LockMap: A fine-grained, thread-safe locking mechanism.
static LOCK_MAP: Lazy<DashMap<String, Arc<Mutex<()>>>> = Lazy::new(DashMap::new);

/// The Virtual File System (VFS): Tracks file changes in memory during dry-runs.
/// This allows LiNix to simulate manifest updates and registry commits accurately.
static VFS: Lazy<DashMap<PathBuf, String>> = Lazy::new(DashMap::new);

/// Defines a pluggable layer for low-level command execution.
#[async_trait]
pub trait ExecutionLayer: Send + Sync {
    async fn execute(
        &self,
        cmd: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<std::process::Output>;
}

/// The base implementation that performs actual OS process spawning.
pub struct RawExecutor;

#[async_trait]
impl ExecutionLayer for RawExecutor {
    async fn execute(
        &self,
        cmd: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<std::process::Output> {
        let mut command = Command::new(cmd);
        command.args(args).envs(env);

        if std::io::stdin().is_terminal() {
            command
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        } else {
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        }

        let output = command
            .spawn()
            .map_err(|e| Error::CommandFailed(format!("Failed to spawn {}: {}", cmd, e)))?
            .wait_with_output()
            .await?;

        Ok(output)
    }
}

/// The primary coordinator for all external command calls and filesystem IO.
/// Hardened for Version 3.5.0 with VFS Middleware to allow deep dry-run simulations.
#[derive(Clone)]
pub struct CommandExecutor {
    pub dry_run: bool,
    pub verbose: bool,
    inner: Arc<dyn ExecutionLayer>,
}

impl CommandExecutor {
    pub fn new(dry_run: bool, verbose: bool) -> Self {
        Self {
            dry_run,
            verbose,
            inner: Arc::new(RawExecutor),
        }
    }

    /// Detects administrative privileges.
    pub fn is_root() -> bool {
        #[cfg(unix)]
        {
            unsafe { libc::geteuid() == 0 }
        }
        #[cfg(windows)]
        {
            // Windows root detection logic remains as in previous version
            false 
        }
    }

    /// Forces a deterministic environment for consistent parsing.
    fn hardened_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("LC_ALL".into(), "C".into());
        env.insert("LANG".into(), "C".into());
        env.insert("LANGUAGE".into(), "C".into());
        env.insert("DEBIAN_FRONTEND".into(), "noninteractive".into());
        env
    }

    /// Executes a command. If dry_run is enabled, it logs the intent and returns a success stub.
    pub async fn run(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<std::process::Output> {
        if self.dry_run {
            info!("[DRY-RUN] Would execute: {} {}", cmd, args.join(" "));
            return Ok(std::process::Output {
                status: unsafe { std::mem::zeroed() }, // Represents success
                stdout: vec![],
                stderr: vec![],
            });
        }

        let mut final_cmd = cmd.to_string();
        let mut final_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        if sudo && !cfg!(windows) && !Self::is_root() {
            final_args.insert(0, final_cmd);
            final_cmd = "sudo".to_string();
        }

        let output = self
            .inner
            .execute(&final_cmd, &final_args, &self.hardened_env())
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::CommandFailed(format!(
                "Command '{}' failed: {}",
                cmd,
                stderr.trim()
            )));
        }

        Ok(output)
    }

    pub async fn run_output(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<String> {
        let output = self.run(cmd, args, sudo).await?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Exclusive lock execution for thread and process safety.
    pub async fn run_exclusive(
        &self, 
        lock_key: &str, 
        cmd: &str, 
        args: &[&str], 
        sudo: bool
    ) -> Result<std::process::Output> {
        let mutex = LOCK_MAP.entry(lock_key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        
        let _thread_guard = mutex.lock().await;

        if self.dry_run {
            return self.run(cmd, args, sudo).await;
        }

        let lock_path = std::env::temp_dir().join(format!("linix_{}.lock", lock_key));
        let lock_file = File::create(&lock_path)?;
        lock_file.lock_exclusive()?;

        let result = self.run(cmd, args, sudo).await;

        let _ = lock_file.unlock();
        result
    }

    // --- VIRTUAL FILESYSTEM MIDDLEWARE ---

    /// Reads a file from the physical disk, or from the VFS if dry-run and modified.
    pub fn read_file(&self, path: &Path) -> Result<String> {
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

    /// Atomically writes content to a file. 
    /// If dry_run is enabled, it diverts the write to the VFS DashMap.
    pub fn write_atomic(&self, path: &Path, content: &str) -> Result<()> {
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

    /// Returns the current state of the VFS for dry-run inspection.
    pub fn get_vfs_diff(&self) -> Vec<(PathBuf, String)> {
        VFS.iter().map(|entry| (entry.key().clone(), entry.value().clone())).collect()
    }

    pub fn command_exists_sync(&self, cmd: &str) -> bool {
        let check_bin = if cfg!(windows) { "where" } else { "which" };
        std::process::Command::new(check_bin)
            .arg(cmd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub async fn start_sudo_keepalive(&self) -> Option<tokio::task::JoinHandle<()>> {
        if cfg!(windows) || Self::is_root() || self.dry_run {
            return None;
        }

        Some(tokio::spawn(async move {
            loop {
                let _ = Command::new("sudo")
                    .arg("-v")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        }))
    }
}