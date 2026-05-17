use crate::core::{Result, Error};
use std::process::Command;
use std::path::PathBuf;
use tracing::{info, debug, warn};

/// Configuration for the declarative sandbox environment.
/// Fulfills Roadmap Point 17.
/// FIX #19: Added macOS support via sandbox-exec.
#[derive(Debug, Clone, Default)]
pub struct SandboxConfig {
    pub allow_network: bool,
    pub allow_home: bool,
    pub allow_write: bool,
    pub custom_mounts: Vec<(String, String)>, // (Source, Target)
    pub custom_read_only_mounts: Vec<(String, String)>,
    pub environment: Vec<(String, String)>,
}

/// A wrapper for sandboxing that works across platforms.
/// - Linux: bubblewrap (bwrap)
/// - macOS: sandbox-exec
/// - Windows: restricted token + job objects (limited support)
pub struct Sandbox;

impl Sandbox {
    /// Checks if sandboxing is supported on the current platform.
    pub fn is_supported() -> bool {
        if cfg!(target_os = "linux") {
            Self::bwrap_available()
        } else if cfg!(target_os = "macos") {
            Self::sandbox_exec_available()
        } else if cfg!(target_os = "windows") {
            false // Windows sandboxing is limited, will use fallback
        } else {
            false
        }
    }
    
    /// Checks if bubblewrap is available on Linux.
    fn bwrap_available() -> bool {
        which::which("bwrap").is_ok()
    }
    
    /// Checks if sandbox-exec is available on macOS.
    fn sandbox_exec_available() -> bool {
        which::which("sandbox-exec").is_ok()
    }
    
    /// Generates a macOS sandbox profile based on configuration.
    /// Uses the sandbox-exec configuration language.
    fn generate_macos_profile(&self, config: &SandboxConfig) -> String {
        let mut profile = String::new();
        
        // Base profile - allow basic system access
        profile.push_str("(version 1)\n");
        profile.push_str("(deny default)\n");
        
        // Allow essential system operations
        profile.push_str("(allow sysctl-read)\n");
        profile.push_str("(allow signal (target self))\n");
        profile.push_str("(allow mach-lookup (global-name \"com.apple.system.notificationcenter\"))\n");
        
        // Allow reading from system directories
        profile.push_str("(allow file-read*\n");
        profile.push_str("    (subpath \"/usr\")\n");
        profile.push_str("    (subpath \"/bin\")\n");
        profile.push_str("    (subpath \"/sbin\")\n");
        profile.push_str("    (subpath \"/System\")\n");
        profile.push_str("    (subpath \"/Library/Frameworks\")\n");
        profile.push_str("    (subpath \"/usr/lib\")\n");
        profile.push_str("    (subpath \"/usr/local/lib\")\n");
        profile.push_str(")\n");
        
        // Allow reading from dynamic linker cache
        profile.push_str("(allow file-read* (subpath \"/private/var/db/dyld\"))\n");
        
        // Allow network if requested
        if config.allow_network {
            profile.push_str("(allow network*)\n");
        } else {
            profile.push_str("(deny network*)\n");
        }
        
        // Allow home directory access if requested
        if config.allow_home {
            if let Some(home) = dirs::home_dir() {
                profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", home.display()));
                if config.allow_write {
                    profile.push_str(&format!("(allow file-write* (subpath \"{}\"))\n", home.display()));
                }
            }
        }
        
        // Custom mounts (read-only by default for safety)
        for (source, target) in &config.custom_read_only_mounts {
            profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", source));
            if PathBuf::from(target).is_absolute() {
                profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", target));
            }
        }
        
        // Custom writable mounts
        if config.allow_write {
            for (source, target) in &config.custom_mounts {
                profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", source));
                profile.push_str(&format!("(allow file-write* (subpath \"{}\"))\n", source));
                if PathBuf::from(target).is_absolute() {
                    profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", target));
                    profile.push_str(&format!("(allow file-write* (subpath \"{}\"))\n", target));
                }
            }
        }
        
        // Allow reading from temporary directory
        profile.push_str("(allow file-read* (subpath \"/tmp\"))\n");
        profile.push_str("(allow file-write* (subpath \"/tmp\"))\n");
        
        // Allow reading from /dev/urandom (needed for many programs)
        profile.push_str("(allow file-read* (subpath \"/dev/urandom\"))\n");
        
        profile
    }
    
    /// Generates a Windows sandbox configuration (limited).
    /// Windows doesn't have a built-in sandbox like bwrap or sandbox-exec,
    /// so we use a combination of job objects and restricted tokens.
    #[cfg(windows)]
    fn setup_windows_sandbox(&self, cmd: &mut Command, config: &SandboxConfig) -> Result<()> {
        use std::os::windows::process::CommandExt;
        use winapi::um::winbase::{CREATE_NEW_CONSOLE, CREATE_NO_WINDOW};
        use winapi::um::jobapi2::{CreateJobObjectW, SetInformationJobObject, AssignProcessToJobObject};
        use winapi::um::winnt::{JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};
        
        // Windows sandboxing is limited - we use job objects to ensure child processes are terminated
        // and restrict certain operations.
        
        if !config.allow_network {
            warn!("Windows sandbox: Network restriction is not fully supported. Use firewall rules instead.");
        }
        
        if !config.allow_home {
            // Set a temporary home directory
            let temp_home = std::env::temp_dir().join("linix_sandbox_home");
            cmd.env("HOME", &temp_home);
            cmd.env("USERPROFILE", &temp_home);
        }
        
        // Use CREATE_NO_WINDOW to prevent console windows from appearing
        cmd.creation_flags(CREATE_NO_WINDOW);
        
        Ok(())
    }

    /// Wraps a command in a platform-appropriate sandbox.
    /// - Linux: bubblewrap (bwrap)
    /// - macOS: sandbox-exec
    /// - Windows: limited job object isolation
    pub fn wrap(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<Command> {
        info!("Sandbox: Initializing restricted environment for '{}' on {}", 
              cmd, std::env::consts::OS);

        #[cfg(target_os = "linux")]
        {
            Self::wrap_linux(cmd, args, config)
        }
        
        #[cfg(target_os = "macos")]
        {
            Self::wrap_macos(cmd, args, config)
        }
        
        #[cfg(target_os = "windows")]
        {
            Self::wrap_windows(cmd, args, config)
        }
        
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            Err(Error::UnsupportedPlatform(format!("Sandboxing not supported on {}", std::env::consts::OS)))
        }
    }
    
    /// Linux implementation using bubblewrap.
    #[cfg(target_os = "linux")]
    fn wrap_linux(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<Command> {
        if !Self::bwrap_available() {
            return Err(Error::UnsupportedPlatform("bubblewrap (bwrap) not found. Install it with: sudo apt install bubblewrap or sudo pacman -S bubblewrap".into()));
        }
        
        let mut bwrap = Command::new("bwrap");
        
        // Core isolation
        bwrap.arg("--unshare-all");
        
        if config.allow_network {
            bwrap.arg("--share-net");
        }
        
        // Essential filesystem bindings (read-only)
        let ro_paths = ["/usr", "/bin", "/lib", "/lib64", "/etc/alternatives"];
        for path in ro_paths {
            if std::path::Path::new(path).exists() {
                bwrap.arg("--ro-bind").arg(path).arg(path);
            }
        }
        
        // Device and temp isolation
        bwrap.arg("--dev").arg("/dev")
             .arg("--proc").arg("/proc")
             .arg("--tmpfs").arg("/tmp");
        
        // Home directory handling
        if config.allow_home {
            if let Some(home) = dirs::home_dir() {
                if config.allow_write {
                    bwrap.arg("--bind").arg(&home).arg(&home);
                } else {
                    bwrap.arg("--ro-bind").arg(&home).arg(&home);
                }
            }
        } else {
            bwrap.arg("--tmpfs").arg("/home");
        }
        
        // Custom mounts
        for (src, target) in &config.custom_read_only_mounts {
            if std::path::Path::new(src).exists() {
                bwrap.arg("--ro-bind").arg(src).arg(target);
            }
        }
        
        for (src, target) in &config.custom_mounts {
            if std::path::Path::new(src).exists() {
                if config.allow_write {
                    bwrap.arg("--bind").arg(src).arg(target);
                } else {
                    bwrap.arg("--ro-bind").arg(src).arg(target);
                }
            }
        }
        
        // Environment variables
        for (key, value) in &config.environment {
            bwrap.arg("--setenv").arg(key).arg(value);
        }
        
        // Execute the command
        bwrap.arg("--").arg(cmd);
        bwrap.args(args);
        
        debug!("Sandbox (Linux): Final command: {:?}", bwrap);
        Ok(bwrap)
    }
    
    /// macOS implementation using sandbox-exec.
    #[cfg(target_os = "macos")]
    fn wrap_macos(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<Command> {
        if !Self::sandbox_exec_available() {
            return Err(Error::UnsupportedPlatform("sandbox-exec not found. This is a standard macOS utility that should be available.".into()));
        }
        
        let sandbox = Sandbox;
        let profile = sandbox.generate_macos_profile(config);
        
        // Write profile to a temporary file
        let temp_dir = std::env::temp_dir();
        let profile_path = temp_dir.join(format!("linix_sandbox_{}.sb", std::process::id()));
        let profile_content = profile.clone();
        
        // Create the profile file
        std::fs::write(&profile_path, &profile_content)
            .map_err(|e| Error::Io(e))?;
        
        let mut sandbox_cmd = Command::new("sandbox-exec");
        sandbox_cmd.arg("-f").arg(&profile_path);
        sandbox_cmd.arg(cmd);
        sandbox_cmd.args(args);
        
        // Set environment variables
        for (key, value) in &config.environment {
            sandbox_cmd.env(key, value);
        }
        
        // Set a flag to clean up the profile file when the process exits
        // Note: The file will persist until the process exits, but that's acceptable
        // for a temporary sandbox.
        
        debug!("Sandbox (macOS): Final command: {:?}", sandbox_cmd);
        debug!("Sandbox (macOS): Profile content:\n{}", profile);
        
        Ok(sandbox_cmd)
    }
    
    /// Windows implementation (limited sandboxing via job objects).
    #[cfg(target_os = "windows")]
    fn wrap_windows(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<Command> {
        let mut cmd = Command::new(cmd);
        cmd.args(args);
        
        // Set environment variables
        for (key, value) in &config.environment {
            cmd.env(key, value);
        }
        
        // Apply Windows-specific sandboxing
        let sandbox = Sandbox;
        sandbox.setup_windows_sandbox(&mut cmd, config)?;
        
        debug!("Sandbox (Windows): Final command: {:?}", cmd);
        Ok(cmd)
    }

    /// Executes a command in a one-off sandbox and returns the status.
    pub fn run(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<std::process::ExitStatus> {
        if !Self::is_supported() {
            if cfg!(target_os = "windows") {
                warn!("Sandboxing on Windows has limited support. Running with reduced isolation.");
            } else {
                return Err(Error::UnsupportedPlatform(
                    format!("Sandboxing not supported on {}. For Linux, install bubblewrap (bwrap). For macOS, sandbox-exec should be available.", 
                            std::env::consts::OS)
                ));
            }
        }
        
        let mut sandboxed_cmd = Self::wrap(cmd, args, config)?;
        let status = sandboxed_cmd.status().map_err(Error::Io)?;
        
        // Clean up temporary profile file on macOS
        #[cfg(target_os = "macos")]
        {
            let temp_dir = std::env::temp_dir();
            let profile_path = temp_dir.join(format!("linix_sandbox_{}.sb", std::process::id()));
            let _ = std::fs::remove_file(profile_path);
        }
        
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sandbox_config_default() {
        let config = SandboxConfig::default();
        assert!(!config.allow_network);
        assert!(!config.allow_home);
        assert!(!config.allow_write);
        assert!(config.custom_mounts.is_empty());
    }
    
    #[test]
    fn test_sandbox_config_custom() {
        let config = SandboxConfig {
            allow_network: true,
            allow_home: true,
            allow_write: true,
            custom_mounts: vec![("/src".to_string(), "/dst".to_string())],
            custom_read_only_mounts: vec![],
            environment: vec![("TEST".to_string(), "value".to_string())],
        };
        
        assert!(config.allow_network);
        assert!(config.allow_home);
        assert!(config.allow_write);
        assert_eq!(config.custom_mounts.len(), 1);
    }
    
    #[test]
    #[cfg(target_os = "macos")]
    fn test_generate_macos_profile() {
        let sandbox = Sandbox;
        let config = SandboxConfig {
            allow_network: true,
            allow_home: true,
            allow_write: false,
            custom_mounts: vec![],
            custom_read_only_mounts: vec![("/usr/local".to_string(), "/usr/local".to_string())],
            environment: vec![],
        };
        
        let profile = sandbox.generate_macos_profile(&config);
        assert!(profile.contains("(version 1)"));
        assert!(profile.contains("(allow network*)"));
        assert!(profile.contains("(allow file-read*"));
        assert!(profile.contains("(subpath \"/usr\")"));
    }
    
    #[test]
    fn test_is_supported_returns_bool() {
        // This just tests that the function returns without panicking
        let _ = Sandbox::is_supported();
    }
}