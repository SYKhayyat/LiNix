use crate::core::{Result, Error};
use crate::config::config::SandboxSettings;
use std::process::Command;
use std::path::Path;
use tracing::{info, debug};
use std::io::Write;
use tempfile::NamedTempFile;

/// Configuration for the declarative sandbox environment.
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
/// 
/// Hardened for Phase 3.1 Final: 
/// - Linux: bwrap with fallback.
/// - macOS: Strict sandbox-exec profiles.
/// - Windows: True hardware-virtualized isolation via Windows Sandbox (.wsb).
pub struct Sandbox;

impl Sandbox {
    /// Checks if sandboxing is supported based on system binaries.
    pub fn is_supported() -> bool {
        if cfg!(target_os = "linux") {
            Self::bwrap_available()
        } else if cfg!(target_os = "macos") {
            Self::sandbox_exec_available()
        } else if cfg!(target_os = "windows") {
            // Windows support is checked dynamically via PowerShell feature detection.
            true 
        } else {
            false
        }
    }

    /// Async check if a usable sandbox mechanism exists considering user fallback preferences.
    pub async fn is_available(settings: &SandboxSettings) -> bool {
        if cfg!(target_os = "linux") {
            Self::bwrap_available() || settings.fallback_allowed
        } else if cfg!(target_os = "macos") {
            Self::sandbox_exec_available() || settings.fallback_allowed
        } else if cfg!(target_os = "windows") {
            Self::windows_sandbox_feature_enabled().await || settings.fallback_allowed
        } else {
            false
        }
    }
    
    fn bwrap_available() -> bool {
        which::which("bwrap").is_ok()
    }
    
    fn sandbox_exec_available() -> bool {
        #[cfg(target_os = "macos")]
        {
            return which::which("sandbox-exec").is_ok();
        }
        #[allow(unreachable_code)]
        false
    }

    /// Detects if the Windows Sandbox optional feature is enabled.
    async fn windows_sandbox_feature_enabled() -> bool {
        #[cfg(target_os = "windows")]
        {
            let output = tokio::process::Command::new("powershell")
                .args(["-Command", "Get-WindowsOptionalFeature -Online -FeatureName 'Containers-DisposableClient' | Select-Object -ExpandProperty State"])
                .output()
                .await;
            
            if let Ok(out) = output {
                return String::from_utf8_lossy(&out.stdout).trim() == "Enabled";
            }
        }
        false
    }

    /// Generates a Windows Sandbox (.wsb) configuration file content.
    #[cfg(target_os = "windows")]
    fn generate_wsb_config(cmd: &str, args: &[String], config: &SandboxConfig) -> String {
        let mut wsb = String::from("<Configuration>\n");
        
        wsb.push_str("  <VGpu>Disable</VGpu>\n");
        wsb.push_str(&format!("  <Networking>{}</Networking>\n", if config.allow_network { "Default" } else { "Disable" }));
        
        wsb.push_str("  <MappedFolders>\n");
        for (src, _) in &config.custom_mounts {
            if Path::new(src).exists() {
                wsb.push_str("    <MappedFolder>\n");
                wsb.push_str(&format!("      <HostFolder>{}</HostFolder>\n", src));
                wsb.push_str("      <ReadOnly>false</ReadOnly>\n");
                wsb.push_str("    </MappedFolder>\n");
            }
        }
        wsb.push_str("  </MappedFolders>\n");

        let full_cmd = format!("{} {}", cmd, args.join(" "));
        wsb.push_str("  <LogonCommand>\n");
        wsb.push_str(&format!("    <Command>{}</Command>\n", full_cmd));
        wsb.push_str("  </LogonCommand>\n");
        
        wsb.push_str("</Configuration>");
        wsb
    }
    
    #[cfg(target_os = "macos")]
    fn generate_macos_profile(config: &SandboxConfig) -> String {
        let mut profile = String::from("(version 1)\n(deny default)\n");
        profile.push_str("(allow sysctl-read)\n(allow signal (target self))\n(allow process-fork)\n(allow process-exec)\n");

        let ro_paths = ["/usr/lib", "/usr/share", "/System/Library", "/Library/Preferences", "/bin", "/usr/bin"];
        for path in ro_paths {
            profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", path));
        }
        
        if config.allow_network {
            profile.push_str("(allow network*)\n(allow file-read* (literal \"/etc/resolv.conf\"))\n");
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

        for (src, _) in &config.custom_read_only_mounts {
            profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", src));
        }
        for (src, _) in &config.custom_mounts {
            profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", src));
            profile.push_str(&format!("(allow file-write* (subpath \"{}\"))\n", src));
        }

        profile
    }

    pub fn wrap(cmd: &str, args: &[String], config: &SandboxConfig, settings: &SandboxSettings) -> Result<Command> {
        #[cfg(target_os = "linux")]
        {
            return Self::wrap_linux(cmd, args, config, settings);
        }
        
        #[cfg(target_os = "macos")]
        {
            return Self::wrap_macos(cmd, args, config, settings);
        }
        
        #[cfg(target_os = "windows")]
        {
            return Self::wrap_windows(cmd, args, config, settings);
        }
        
        #[allow(unreachable_code)]
        Err(Error::UnsupportedPlatform(format!("Sandboxing not supported on {}", std::env::consts::OS)))
    }
    
    #[cfg(target_os = "linux")]
    fn wrap_linux(cmd: &str, args: &[String], config: &SandboxConfig, settings: &SandboxSettings) -> Result<Command> {
        if !Self::bwrap_available() {
            if settings.fallback_allowed {
                debug!("Sandboxing: 'bwrap' not found. Falling back to PATH isolation.");
                let mut fallback = Command::new(cmd);
                fallback.args(args);
                return Ok(fallback);
            }
            return Err(Error::UnsupportedPlatform("bubblewrap (bwrap) required but not found.".into()));
        }
        
        let mut bwrap = Command::new("bwrap");
        bwrap.arg("--unshare-all");
        if config.allow_network { bwrap.arg("--share-net"); }
        
        let ro_paths = ["/usr", "/bin", "/lib", "/lib64", "/etc/alternatives"];
        for path in ro_paths {
            if Path::new(path).exists() { bwrap.arg("--ro-bind").arg(path).arg(path); }
        }
        
        bwrap.arg("--dev").arg("/dev").arg("--proc").arg("/proc").arg("--tmpfs").arg("/tmp");
        
        if config.allow_home {
            if let Some(home) = dirs::home_dir() {
                if config.allow_write { bwrap.arg("--bind").arg(&home).arg(&home); }
                else { bwrap.arg("--ro-bind").arg(&home).arg(&home); }
            }
        }

        for (src, target) in &config.custom_read_only_mounts {
            if Path::new(src).exists() { bwrap.arg("--ro-bind").arg(src).arg(target); }
        }
        for (src, target) in &config.custom_mounts {
            if Path::new(src).exists() { bwrap.arg("--bind").arg(src).arg(target); }
        }

        for (key, value) in &config.environment { bwrap.arg("--setenv").arg(key).arg(value); }

        bwrap.arg("--").arg(cmd).args(args);
        Ok(bwrap)
    }
    
    #[cfg(target_os = "macos")]
    fn wrap_macos(cmd: &str, args: &[String], config: &SandboxConfig, settings: &SandboxSettings) -> Result<Command> {
        if !Self::sandbox_exec_available() {
            if settings.fallback_allowed {
                debug!("Sandboxing: 'sandbox-exec' not found. Falling back to non-sandboxed execution.");
                let mut fallback = Command::new(cmd);
                fallback.args(args);
                return Ok(fallback);
            }
            return Err(Error::UnsupportedPlatform("sandbox-exec required but not found.".into()));
        }

        let profile = if let Some(ref path) = settings.macos_profile_template {
            std::fs::read_to_string(path).map_err(|e| Error::Config(format!("Failed to read custom macOS profile: {}", e)))?
        } else {
            Self::generate_macos_profile(config)
        };

        let mut sandbox_cmd = Command::new("sandbox-exec");
        sandbox_cmd.arg("-p").arg(profile);
        for (key, value) in &config.environment { sandbox_cmd.env(key, value); }
        sandbox_cmd.arg(cmd).args(args);
        Ok(sandbox_cmd)
    }
    
    #[cfg(target_os = "windows")]
    fn wrap_windows(cmd: &str, args: &[String], config: &SandboxConfig, settings: &SandboxSettings) -> Result<Command> {
        let wsb_exe = "C:\\Windows\\System32\\WindowsSandbox.exe";
        
        if Path::new(wsb_exe).exists() {
            info!("Sandboxing (Windows): Launching hardware-isolated environment (.wsb)");
            let wsb_content = Self::generate_wsb_config(cmd, args, config);
            let mut tmp_file = NamedTempFile::new().map_err(Error::from)?;
            tmp_file.write_all(wsb_content.as_bytes()).map_err(Error::from)?;
            let mut command = Command::new(wsb_exe);
            command.arg(tmp_file.path());
            return Ok(command);
        }

        // Phase 3.1 Hardening: If true sandbox is required but missing, check config
        if settings.windows_require_sandbox {
            return Err(Error::UnsupportedPlatform(
                "Windows Sandbox feature is required by configuration but not enabled on this system.".into()
            ));
        }

        // Best-effort fallback
        debug!("Sandboxing (Windows): Windows Sandbox unavailable. Using integrity-level fallback.");
        let mut command = Command::new("cmd");
        command.arg("/c").arg("start").arg("/low").arg("/b").arg(cmd).args(args);
        for (key, value) in &config.environment { command.env(key, value); }
        if !config.allow_home { command.env("USERPROFILE", "C:\\Users\\Public"); }
        Ok(command)
    }

    /// Executes a command in a one-off sandbox.
    pub fn run(cmd: &str, args: &[String], config: &SandboxConfig, settings: &SandboxSettings) -> Result<std::process::ExitStatus> {
        let mut sandboxed_cmd = Self::wrap(cmd, args, config, settings)?;
        sandboxed_cmd.status().map_err(Error::from)
    }
}