use crate::config::config::SandboxSettings;
use crate::core::{Error, Result};
#[allow(unused_imports)] // `Path` is unused on macOS but used on linux/windows
use std::path::Path;
use std::process::Command;
// `info`, `Write` and `NamedTempFile` are used only by the Windows/macOS sandbox paths.
#[allow(unused_imports)]
use std::io::Write;
#[allow(unused_imports)]
use tempfile::NamedTempFile;
#[allow(unused_imports)]
use tracing::{debug, info};

/// Configuration for the declarative sandbox environment.
#[derive(Debug, Clone, Default)]
pub struct SandboxConfig {
    pub allow_network: bool,
    pub allow_home: bool,
    pub allow_write: bool,
    pub custom_mounts: Vec<(String, String)>, // (Source, Target)
    pub custom_read_only_mounts: Vec<(String, String)>,
    pub environment: Vec<(String, String)>,
}

/// Whether a command is actually confined, carried beside the command rather than inferred
/// from the settings that asked for it.
///
/// A caller holding a [`Command`] cannot tell the two apart: an unconfined fallback and a real
/// `bwrap` invocation have the same type. Every way this has gone wrong is a caller reporting a
/// boundary it was never handed one of, so the answer travels with the command and a caller that
/// wants to claim confinement has to hold the variant that grants it.
#[derive(Debug, Clone)]
#[must_use]
pub enum Confinement {
    /// A mechanism is in force, named so a caller can say which one.
    By(&'static str),
    /// No mechanism is in force. Carries the reason, so a caller cannot report the absence
    /// without being able to say why it is absent.
    None { because: String },
}

impl Confinement {
    /// The sentence a user is owed when a command they asked to confine is about to run loose.
    pub fn unconfined_warning(&self) -> Option<String> {
        match self {
            Self::By(_) => None,
            Self::None { because } => Some(format!(
                "confinement was requested but this command will run unconfined: {because}"
            )),
        }
    }
}

pub struct Sandbox;

impl Sandbox {
    /// The one place that answers "is a mechanism in force, and if not may this proceed".
    ///
    /// `run`, `shell` and `wrap` each used to answer it separately, which is how the `warn!` in
    /// `run` became unreachable: its condition was already folded into the predicate in front of
    /// it. The decision is made once here and carried, so the three cannot disagree and a
    /// mechanism that is missing cannot be spelled the same as one that is present.
    ///
    /// `require_bwrap` and `windows_require_sandbox` outrank `fallback_allowed`: they exist to
    /// refuse exactly the run `fallback_allowed` would permit, so a host that has neither
    /// mechanism nor permission to skip it gets an error rather than a bare command.
    pub async fn decide(settings: &SandboxSettings) -> Result<Confinement> {
        let (mechanism, present, required, knob) = if cfg!(target_os = "linux") {
            (
                "bubblewrap",
                Self::bwrap_available(),
                settings.require_bwrap,
                "sandbox.require_bwrap",
            )
        } else if cfg!(target_os = "macos") {
            (
                "sandbox-exec",
                Self::sandbox_exec_available(),
                false,
                "sandbox.fallback_allowed",
            )
        } else if cfg!(target_os = "windows") {
            (
                "Windows Sandbox",
                Self::windows_sandbox_feature_enabled().await,
                settings.windows_require_sandbox,
                "sandbox.windows_require_sandbox",
            )
        } else {
            (
                std::env::consts::OS,
                false,
                false,
                "sandbox.fallback_allowed",
            )
        };

        if present {
            return Ok(Confinement::By(mechanism));
        }
        if required {
            return Err(Error::UnsupportedPlatform(format!(
                "`{knob}` is set and `{mechanism}` is not functional on this host."
            )));
        }
        if !settings.fallback_allowed {
            return Err(Error::UnsupportedPlatform(format!(
                "Sandboxing is required by policy but `{mechanism}` is not functional on this host."
            )));
        }
        Ok(Confinement::None {
            because: format!("`{mechanism}` is not available on this host"),
        })
    }

    fn bwrap_available() -> bool {
        crate::core::launch::program_exists("bwrap")
    }

    fn sandbox_exec_available() -> bool {
        #[cfg(target_os = "macos")]
        {
            return crate::core::launch::program_exists("sandbox-exec");
        }
        #[allow(unreachable_code)]
        false
    }

    /// Where the Windows Sandbox host binary lives. Named once because `decide` and
    /// `wrap_windows` asking two different questions about one mechanism is how the verdict and
    /// the command they build came apart.
    #[cfg(target_os = "windows")]
    const WSB_EXE: &'static str = "C:\\Windows\\System32\\WindowsSandbox.exe";

    /// Detects if the Windows Sandbox optional feature is enabled *and* its host binary is on
    /// disk. Both, because the feature can be reported enabled while servicing has not yet laid
    /// the binary down, and `wrap_windows` needs the binary.
    async fn windows_sandbox_feature_enabled() -> bool {
        #[cfg(target_os = "windows")]
        {
            if !Path::new(Self::WSB_EXE).exists() {
                return false;
            }
            let mut command = tokio::process::Command::new("powershell");
            command.args(["-NoProfile", "-NonInteractive", "-Command", "Get-WindowsOptionalFeature -Online -FeatureName 'Containers-DisposableClient' | Select-Object -ExpandProperty State"]);
            // Supervised: `Get-WindowsOptionalFeature` talks to the servicing stack, which is
            // the component most likely on a Windows box to answer in its own time or not at all.
            let output =
                crate::core::supervise::supervised_output(command, "powershell", false).await;

            if let Ok(out) = output {
                return crate::utils::text::sanitize(&String::from_utf8_lossy(&out.stdout))
                    == "Enabled";
            }
        }
        false
    }

    /// Generates a Windows Sandbox (.wsb) configuration file content.
    #[cfg(target_os = "windows")]
    fn generate_wsb_config(cmd: &str, args: &[String], config: &SandboxConfig) -> String {
        let mut wsb = String::from("<Configuration>\n");

        wsb.push_str("  <VGpu>Disable</VGpu>\n");
        wsb.push_str(&format!(
            "  <Networking>{}</Networking>\n",
            if config.allow_network {
                "Default"
            } else {
                "Disable"
            }
        ));

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

        let ro_paths = [
            "/usr/lib",
            "/usr/share",
            "/System/Library",
            "/Library/Preferences",
            "/bin",
            "/usr/bin",
        ];
        for path in ro_paths {
            profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", path));
        }

        if config.allow_network {
            profile
                .push_str("(allow network*)\n(allow file-read* (literal \"/etc/resolv.conf\"))\n");
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

    /// Build the command `decided` describes.
    ///
    /// The verdict is passed in rather than recomputed: recomputing it here is what let the
    /// decision differ between the caller that reports to the user and the code that builds the
    /// process. An unconfined verdict yields the bare command and nothing else — in particular it
    /// does not claim an isolation it is not building.
    pub fn wrap(
        cmd: &str,
        args: &[String],
        config: &SandboxConfig,
        settings: &SandboxSettings,
        decided: &Confinement,
    ) -> Result<Command> {
        let _ = (config, settings);
        if let Confinement::None { .. } = decided {
            // Windows keeps its low-integrity launch here rather than in `By`: it does lower the
            // token, so deleting it would remove a real reduction, and it is not a sandbox, so
            // naming it one would be the claim this type exists to stop.
            #[cfg(target_os = "windows")]
            {
                return Ok(Self::low_integrity_windows(cmd, args, config));
            }
            #[allow(unreachable_code)]
            {
                let mut bare = Command::new(cmd);
                bare.args(args);
                return Ok(bare);
            }
        }

        #[cfg(target_os = "linux")]
        {
            return Self::wrap_linux(cmd, args, config);
        }

        #[cfg(target_os = "macos")]
        {
            return Self::wrap_macos(cmd, args, config, settings);
        }

        #[cfg(target_os = "windows")]
        {
            return Self::wrap_windows(cmd, args, config);
        }

        #[allow(unreachable_code)]
        Err(Error::UnsupportedPlatform(format!(
            "Sandboxing not supported on {}",
            std::env::consts::OS
        )))
    }

    #[cfg(target_os = "linux")]
    fn wrap_linux(cmd: &str, args: &[String], config: &SandboxConfig) -> Result<Command> {
        // Reached only for a `Confinement::By` verdict. A mechanism that has gone missing between
        // the decision and here is an error rather than a fallback: the caller has already told
        // the user this command is confined.
        if !Self::bwrap_available() {
            return Err(Error::UnsupportedPlatform(
                "bubblewrap (bwrap) was available when this run was planned and is not now.".into(),
            ));
        }

        let mut bwrap = Command::new("bwrap");
        bwrap.arg("--unshare-all");
        if config.allow_network {
            bwrap.arg("--share-net");
        }

        let ro_paths = ["/usr", "/bin", "/lib", "/lib64", "/etc/alternatives"];
        for path in ro_paths {
            if Path::new(path).exists() {
                bwrap.arg("--ro-bind").arg(path).arg(path);
            }
        }

        bwrap
            .arg("--dev")
            .arg("/dev")
            .arg("--proc")
            .arg("/proc")
            .arg("--tmpfs")
            .arg("/tmp");

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
            if Path::new(src).exists() {
                bwrap.arg("--ro-bind").arg(src).arg(target);
            }
        }
        for (src, target) in &config.custom_mounts {
            if Path::new(src).exists() {
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
    fn wrap_macos(
        cmd: &str,
        args: &[String],
        config: &SandboxConfig,
        settings: &SandboxSettings,
    ) -> Result<Command> {
        // As `wrap_linux`: reached only for a `Confinement::By` verdict, so a mechanism that has
        // gone missing since is an error rather than a silent unconfined run.
        if !Self::sandbox_exec_available() {
            return Err(Error::UnsupportedPlatform(
                "sandbox-exec was available when this run was planned and is not now.".into(),
            ));
        }

        let profile = if let Some(ref path) = settings.macos_profile_template {
            std::fs::read_to_string(path)
                .map_err(|e| Error::Config(format!("Failed to read custom macOS profile: {}", e)))?
        } else {
            Self::generate_macos_profile(config)
        };

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
        // As `wrap_linux`: reached only for a `Confinement::By` verdict.
        if !Path::new(Self::WSB_EXE).exists() {
            return Err(Error::UnsupportedPlatform(
                "Windows Sandbox was available when this run was planned and is not now.".into(),
            ));
        }

        info!("Sandboxing (Windows): Launching hardware-isolated environment (.wsb)");
        let wsb_content = Self::generate_wsb_config(cmd, args, config);
        let mut tmp_file = NamedTempFile::new().map_err(Error::from)?;
        tmp_file
            .write_all(wsb_content.as_bytes())
            .map_err(Error::from)?;
        let mut command = Command::new(Self::WSB_EXE);
        command.arg(tmp_file.path());
        Ok(command)
    }

    /// Windows' unconfined path: a lower integrity level, which is a real reduction and is not
    /// a sandbox. Reached only for a `Confinement::None` verdict, whose `because` says so.
    #[cfg(target_os = "windows")]
    fn low_integrity_windows(cmd: &str, args: &[String], config: &SandboxConfig) -> Command {
        let mut command = Command::new("cmd");
        command
            .arg("/c")
            .arg("start")
            .arg("/low")
            .arg("/b")
            .arg(cmd)
            .args(args);
        for (key, value) in &config.environment {
            command.env(key, value);
        }
        if !config.allow_home {
            command.env("USERPROFILE", "C:\\Users\\Public");
        }
        command
    }

    /// Executes a command under the confinement `decided` describes.
    pub fn run(
        cmd: &str,
        args: &[String],
        config: &SandboxConfig,
        settings: &SandboxSettings,
        decided: &Confinement,
    ) -> Result<std::process::ExitStatus> {
        let mut sandboxed_cmd = Self::wrap(cmd, args, config, settings, decided)?;
        sandboxed_cmd.status().map_err(Error::from)
    }
}
