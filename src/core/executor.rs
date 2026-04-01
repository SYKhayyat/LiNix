use crate::core::{Error, Result};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;
use tokio::sync::Mutex;
use fs2::FileExt; 
use std::fs::File;
use once_cell::sync::Lazy;
use tracing::{debug, info, warn, error};
use std::time::Duration;
use std::path::{Path, PathBuf};
use std::io::IsTerminal;


/// Internal mutex for multi-threaded access within a single LiNix instance.
static INTERNAL_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Clone)]
pub struct CommandExecutor {
    pub dry_run: bool,
    pub verbose: bool,
}

impl CommandExecutor {
    pub fn new(dry_run: bool, verbose: bool) -> Self {
        Self { dry_run, verbose }
    }

    /// Helper to detect root/admin privileges.
    pub fn is_root() -> bool {
    #[cfg(unix)] {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(windows)] {
        // REAL LOGIC: Check if the current process has Admin privileges
        use std::ptr;
        use winapi::um::processthreadsapi::{GetCurrentProcess, OpenProcessToken};
        use winapi::um::securitybaseapi::GetTokenInformation;
        use winapi::um::winnt::{TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};

        unsafe {
            let mut token = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) != 0 {
                let mut elevation = TOKEN_ELEVATION::default();
                let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
                if GetTokenInformation(token, TokenElevation, &mut elevation as *mut _ as *mut _, size, &mut size) != 0 {
                    return elevation.TokenIsElevated != 0;
                }
            }
            false
        }
    }
}

    /// Spawns a background task to keep the sudo session alive.
    /// This prevents long-running syncs from hanging on a hidden password prompt.
    pub async fn start_sudo_keepalive(&self) -> Option<tokio::task::JoinHandle<()>> {
        if cfg!(windows) || self.dry_run || Self::is_root() { return None; }
        
        info!("Requesting administrative authorization for the session...");
        let status = Command::new("sudo").arg("-v").status().await;
        if let Ok(s) = status {
            if !s.success() { 
                warn!("Sudo authorization failed. Some operations may fail or hang.");
                return None; 
            }
        }

        Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(50)).await;
                // Periodic non-interactive update of sudo timestamp (-n = non-interactive)
                let _ = Command::new("sudo").args(["-n", "-v"]).status().await;
            }
        }))
    }

    /// Actively waits for external system locks (apt/pacman/dnf) to be released.
    /// Employs exponential backoff for up to 4 minutes.
    async fn wait_for_system_locks(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            let lock_files = vec![
                "/var/lib/dpkg/lock-frontend",
                "/var/lib/apt/lists/lock",
                "/var/lib/pacman/db.lck",
                "/var/run/yum.pid",
            ];

            let mut attempts = 0;
            let mut delay = Duration::from_secs(2);
            
            while attempts < 7 { 
                let mut locked = false;
                for lock in &lock_files {
                    if Path::new(lock).exists() {
                        locked = true;
                        break;
                    }
                }

                if !locked { return Ok(()); }

                warn!("System package database is locked by another process. Retrying in {:?} (Attempt {}/7)...", delay, attempts + 1);
                tokio::time::sleep(delay).await;
                attempts += 1;
                delay *= 2; 
            }
            return Err(Error::Other("Timeout: System lock is held. Ensure no other updates are running.".into()));
        }
        Ok(())
    }

    /// Runs a command with an inter-process file lock AND system lock checks.
    pub async fn run_exclusive(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<std::process::Output> {
        let _internal_guard = INTERNAL_LOCK.lock().await;
        
        // LiNix Inter-process Lock (prevents two LiNix instances from colliding)
        let lock_path = std::env::temp_dir().join("linix_global.lock");
        let lock_file = File::create(&lock_path)?;
        lock_file.lock_exclusive().map_err(|e| Error::Other(format!("Failed to acquire inter-process lock: {}", e)))?;

        // Wait for OS-level locks (apt/pacman)
        self.wait_for_system_locks().await?;

        // Verify sudo is still valid before entering background execution
        if sudo && !cfg!(windows) && !Self::is_root() {
            let check = Command::new("sudo").args(["-n", "true"]).status().await;
            if !check.map(|s| s.success()).unwrap_or(false) {
                let _ = lock_file.unlock();
                return Err(Error::Permission("Sudo timeout. Please run 'sudo -v' in your terminal.".into()));
            }
        }

        let result = self.run(cmd, args, sudo).await;
        let _ = lock_file.unlock();
        result
    }

    /// Basic execution.
    pub async fn run(&self, cmd: &str, args: &[&str], use_sudo: bool) -> Result<std::process::Output> {
        self.run_with_env(cmd, args, use_sudo, &HashMap::new()).await
    }

    /// Full execution with environment injection and interactive capability.
    pub async fn run_with_env(
        &self,
        cmd: &str,
        args: &[&str],
        use_sudo: bool,
        env: &HashMap<String, String>,
    ) -> Result<std::process::Output> {
        // 1. Prepare command based on platform and sudo requirement
        let (mut final_cmd, mut final_args) = if use_sudo && !cfg!(windows) && !Self::is_root() {
            let mut sudo_args = vec![cmd.to_string()];
            sudo_args.extend(args.iter().map(|s| s.to_string()));
            ("sudo".to_string(), sudo_args)
        } else {
            (cmd.to_string(), args.iter().map(|s| s.to_string()).collect())
        };

        // 2. Handle Dry Run
        if self.dry_run {
            info!("[DRY-RUN] Would execute: {} {}", final_cmd, final_args.join(" "));
            return Ok(std::process::Output {
                status: unsafe { std::mem::zeroed() }, // Successful exit code 0
                stdout: Vec::new(),
                stderr: Vec::new(),
            });
        }

        // 3. Logic Fix for Ghost Interactivity: Inject non-interactive flags automatically
        if (cmd == "apt-get" || cmd == "apt") && !final_args.contains(&"-y".to_string()) {
            final_args.push("-y".into());
            final_args.push("-o".into());
            final_args.push("Dpkg::Options::=--force-confold".into());
        }

        if self.verbose {
            debug!("Executing: {} {}", final_cmd, final_args.join(" "));
        }

        // 4. Fix for Automation Death (TTY Check):
        // If stdin is a terminal, we inherit it so the user can answer prompts.
        // If it's a script/robot, we provide Null to prevent the process from hanging forever.
        let stdin_mode = if std::io::stdin().is_terminal() {
    Stdio::inherit()
} else {
    Stdio::null()
};

        // 5. Build and Spawn the command
        let mut command = Command::new(&final_cmd);
        command
            .args(&final_args)
            .envs(env)
            .stdin(stdin_mode)
            .stdout(Stdio::inherit()) // Inherit allows user to see progress bars from backends
            .stderr(Stdio::inherit());

        let mut child = command.spawn()
            .map_err(|e| Error::CommandFailed(format!("Failed to spawn process {}: {}", final_cmd, e)))?;

        // 6. Wait for the process to finish
        let status = child.wait().await?;

        if !status.success() {
            return Err(Error::CommandFailed(format!(
                "Command '{}' failed with exit code: {}",
                final_cmd, status
            )));
        }

        // We return an empty Output struct because we inherited stdout/stderr
        // If a backend needs to PARSE output, it should call 'run_output' instead.
        Ok(std::process::Output { status, stdout: vec![], stderr: vec![] })
    }

    /// Clean output collection (no interactive inheritance).
    pub async fn run_output(&self, cmd: &str, args: &[&str], use_sudo: bool) -> Result<String> {
        let (final_cmd, final_args) = self.prepare_command(cmd, args, use_sudo);
        
        let mut command = Command::new(&final_cmd);
        command.args(&final_args);
        
        let output = command.output().await
            .map_err(|e| Error::CommandFailed(format!("Failed to execute {}: {}", final_cmd, e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::CommandFailed(format!("{} output error: {}", final_cmd, stderr.trim())));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Combined stdout/stderr for logging.
    pub async fn run_combined_output(&self, cmd: &str, args: &[&str], use_sudo: bool) -> Result<String> {
        let (final_cmd, final_args) = self.prepare_command(cmd, args, use_sudo);
        let output = Command::new(final_cmd).args(final_args).output().await?;
        let mut result = String::from_utf8_lossy(&output.stdout).to_string();
        result.push_str(&String::from_utf8_lossy(&output.stderr));
        Ok(result.trim().to_string())
    }

    fn prepare_command(&self, cmd: &str, args: &[&str], use_sudo: bool) -> (String, Vec<String>) {
        if use_sudo && !cfg!(windows) && !Self::is_root() {
            let mut sudo_args = vec![cmd.to_string()];
            sudo_args.extend(args.iter().map(|s| s.to_string()));
            return ("sudo".to_string(), sudo_args);
        }
        (cmd.to_string(), args.iter().map(|s| s.to_string()).collect())
    }

    /// Binary detection including non-standard fallbacks.
    pub async fn command_exists(&self, cmd: &str) -> bool {
        let check_cmd = if cfg!(windows) { "where" } else { "which" };
        let status = Command::new(check_cmd).arg(cmd).stdout(Stdio::null()).stderr(Stdio::null()).status().await;
        
        if let Ok(s) = status {
            if s.success() { return true; }
        }

        let home = dirs::home_dir().unwrap_or_default();
        let fallbacks = vec![
            home.join(".local").join("bin"),
            home.join(".cargo").join("bin"),
            home.join(".npm-global").join("bin"),
            home.join("go").join("bin"),
            home.join(".local").join("share").join("mise").join("shims"),
        ];

        for path in fallbacks {
            if path.join(cmd).exists() || path.join(format!("{}.exe", cmd)).exists() {
                return true;
            }
        }
        false
    }
	pub fn refresh_paths(&self, new_path: std::path::PathBuf) {
    if let Some(path) = std::env::var_os("PATH") {
        let mut paths = std::env::split_paths(&path).collect::<Vec<_>>();
        if !paths.contains(&new_path) {
            paths.insert(0, new_path);
            let merged = std::env::join_paths(paths).unwrap();
            std::env::set_var("PATH", merged);
        }
    }
}
}