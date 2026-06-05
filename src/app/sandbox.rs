use crate::core::{Result, Error};
use std::process::Command;
use std::path::PathBuf;
use tracing::warn;

/// Configuration for the declarative sandbox environment.
/// Fulfills Roadmap Point 17.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub allow_network: bool,
    pub allow_home: bool,
    pub allow_write: bool,
    pub custom_mounts: Vec<(String, String)>, // (Source, Target)
    pub custom_read_only_mounts: Vec<(String, String)>,
    pub environment: Vec<(String, String)>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            allow_network: false,
            allow_home: false,
            allow_write: false,
            custom_mounts: Vec::new(),
            custom_read_only_mounts: Vec::new(),
            environment: Vec::new(),
        }
    }
}

/// A wrapper for sandboxing that works across platforms.
pub struct Sandbox;

impl Sandbox {
    /// Checks if sandboxing is supported on the current platform.
    pub fn is_supported() -> bool {
        if cfg!(target_os = "linux") {
            Self::bwrap_available()
        } else if cfg!(target_os = "macos") {
            Self::sandbox_exec_available()
        } else {
            false
        }
    }
    
    fn bwrap_available() -> bool {
        which::which("bwrap").is_ok()
    }
    
    fn sandbox_exec_available() -> bool {
        which::which("sandbox-exec").is_ok()
    }
    
    /// Generates a macOS sandbox profile based on configuration.
    #[cfg(target_os = "macos")]
    fn generate_macos_profile(config: &SandboxConfig) -> String {
        let mut profile = String::from("(version 1)\n(deny default)\n");
        profile.push_str("(allow sysctl-read)\n(allow signal (target self))\n");
        profile.push_str("(allow file-read* (subpath \"/usr\"))\n");
        profile.push_str("(allow file-read* (subpath \"/bin\"))\n");
        profile.push_str("(allow file-read* (subpath \"/lib\"))\n");
        
        if config.allow_network {
            profile.push_str("(allow network*)\n");
        }
        
        if config.allow_home {
            if let Some(home) = dirs::home_dir() {
                let path = home.to_string_lossy();
                profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", path));
                if config.allow_write {
                    profile.push_str(&format!("(allow file-write* (subpath \"{}\"))\n", path));
                }
            }
        }
        profile
    }

    /// Wraps a command in a platform-appropriate sandbox.
    pub fn wrap(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<Command> {
        #[cfg(target_os = "linux")]
        {
            return Self::wrap_linux(cmd, args, config);
        }
        
        #[cfg(target_os = "macos")]
        {
            return Self::wrap_macos(cmd, args, config);
        }
        
        #[cfg(target_os = "windows")]
        {
            return Self::wrap_windows(cmd, args, config);
        }
        
        #[allow(unreachable_code)]
        Err(Error::UnsupportedPlatform(format!("Sandboxing not supported on {}", std::env::consts::OS)))
    }
    
    #[cfg(target_os = "linux")]
    fn wrap_linux(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<Command> {
        if !Self::bwrap_available() {
            return Err(Error::UnsupportedPlatform("bubblewrap (bwrap) not found. Install it for sandboxing.".into()));
        }
        
        let mut bwrap = Command::new("bwrap");
        bwrap.arg("--unshare-all");
        
        if config.allow_network { bwrap.arg("--share-net"); }
        
        let ro_paths = ["/usr", "/bin", "/lib", "/lib64", "/etc/alternatives"];
        for path in ro_paths {
            if std::path::Path::new(path).exists() {
                bwrap.arg("--ro-bind").arg(path).arg(path);
            }
        }
        
        bwrap.arg("--dev").arg("/dev").arg("--proc").arg("/proc").arg("--tmpfs").arg("/tmp");
        
        if config.allow_home {
            if let Some(home) = dirs::home_dir() {
                if config.allow_write {
                    bwrap.arg("--bind").arg(&home).arg(&home);
                } else {
                    bwrap.arg("--ro-bind").arg(&home).arg(&home);
                }
            }
        }

        for (src, target) in &config.custom_read_only_mounts {
            if std::path::Path::new(src).exists() {
                bwrap.arg("--ro-bind").arg(src).arg(target);
            }
        }
        
        for (src, target) in &config.custom_mounts {
            if std::path::Path::new(src).exists() {
                bwrap.arg("--bind").arg(src).arg(target);
            }
        }

        for (key, value) in &config.environment {
            bwrap.arg("--setenv").arg(key).arg(value);
        }

        bwrap.arg("--").arg(cmd).args(args);
        Ok(bwrap)
    }
    
    #[cfg(target_os = "macos")]
    fn wrap_macos(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<Command> {
        let profile = Self::generate_macos_profile(config);
        let mut sandbox_cmd = Command::new("sandbox-exec");
        sandbox_cmd.arg("-p").arg(profile);
        
        for (key, value) in &config.environment {
            sandbox_cmd.env(key, value);
        }
        
        sandbox_cmd.arg(cmd).args(args);
        Ok(sandbox_cmd)
    }
    
    #[cfg(target_os = "windows")]
    fn wrap_windows(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<Command> {
        let mut command = Command::new(cmd);
        command.args(args);
        
        for (key, value) in &config.environment {
            command.env(key, value);
        }
        
        if !config.allow_home {
            command.env("USERPROFILE", "C:\\Users\\Public");
        }
        
        warn!("Windows Sandbox: Only basic environment isolation is supported.");
        Ok(command)
    }

    /// Executes a command in a one-off sandbox and returns the status.
    pub fn run(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<std::process::ExitStatus> {
        let mut sandboxed_cmd = Self::wrap(cmd, args, config)?;
        sandboxed_cmd.status().map_err(Error::from)
    }
}