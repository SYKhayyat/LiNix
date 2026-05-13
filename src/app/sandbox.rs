use crate::core::{Result, Error};
use std::process::Command;
use tracing::{info, debug};

/// Configuration for the declarative sandbox environment.
/// Fulfills Roadmap Point 17.
#[derive(Debug, Clone, Default)]
pub struct SandboxConfig {
    pub allow_network: bool,
    pub allow_home: bool,
    pub custom_mounts: Vec<(String, String)>, // (Source, Target)
}

/// A wrapper for 'bubblewrap' (bwrap) to provide secure execution environments.
/// Ensures that sandboxed applications cannot access sensitive system files 
/// or persist data outside of allowed zones.
pub struct Sandbox;

impl Sandbox {
    /// Wraps a command in a bubblewrap container.
    /// 
    /// Default security posture:
    /// - New IPC namespace
    /// - New PID namespace
    /// - New UTS namespace (hostname)
    /// - Read-only bind of /usr, /bin, /lib, /lib64
    /// - Private /tmp and /dev
    pub fn wrap(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<Command> {
        info!("Sandbox: Initializing restricted environment for '{}'", cmd);

        let mut bwrap = Command::new("bwrap");

        // 1. Core Isolation Namespaces
        bwrap.arg("--unshare-all"); // Unshare all namespaces including net, user, ipc, pid
        
        if config.allow_network {
            // Re-share the network if explicitly requested
            bwrap.arg("--share-net");
        }

        // 2. Essential Filesystem Bindings (Read-Only)
        // We mount the bare essentials needed to run standard binaries.
        let ro_paths = ["/usr", "/bin", "/lib", "/lib64", "/etc/alternatives"];
        for path in ro_paths {
            if std::path::Path::new(path).exists() {
                bwrap.arg("--ro-bind").arg(path).arg(path);
            }
        }

        // 3. Device and Temp Isolation
        bwrap.arg("--dev").arg("/dev")
             .arg("--proc").arg("/proc")
             .arg("--tmpfs").arg("/tmp");

        // 4. Identity and Home Handling
        if config.allow_home {
            if let Some(home) = dirs::home_dir() {
                bwrap.arg("--bind").arg(&home).arg(&home);
            }
        } else {
            // Provide a fake minimal home if access is denied
            bwrap.arg("--dir").arg("/home/linix")
                 .arg("--setenv").arg("HOME", "/home/linix");
        }

        // 5. Custom Mounts
        for (src, target) in &config.custom_mounts {
            bwrap.arg("--bind").arg(src).arg(target);
        }

        // 6. Pass the actual command and arguments
        bwrap.arg("--").arg(cmd);
        bwrap.args(args);

        debug!("Sandbox: Final command: {:?}", bwrap);
        Ok(bwrap)
    }

    /// Checks if the bubblewrap binary is available on the system.
    pub fn is_supported() -> bool {
        which::which("bwrap").is_ok()
    }

    /// Executes a command in a one-off sandbox and returns the status.
    pub fn run(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<std::process::ExitStatus> {
        if !Self::is_supported() {
            return Err(Error::UnsupportedPlatform("bubblewrap (bwrap) not found. Sandboxing requires bwrap on Linux.".into()));
        }

        let mut sandboxed_cmd = Self::wrap(cmd, args, config)?;
        let status = sandboxed_cmd.status().map_err(Error::Io)?;
        
        Ok(status)
    }
}