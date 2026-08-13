use crate::app::sandbox::{Sandbox, SandboxConfig};
use crate::app::sync::resolver::StateResolver;
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Error, PackageSpec, Result};
use std::sync::Arc;
use tokio::process::Command;
use tracing::{debug, error, info, instrument, warn};

pub struct Runner {
    registry: Arc<BackendRegistry>,
    config: Arc<Config>,
    /// `shall run` provisions what the command needs and does not remove it afterwards, so
    /// the install it performs is as real and as interruptible as any other. Calling it
    /// temporary describes the intent, not what the package manager is left holding.
    journal: Arc<tokio::sync::Mutex<crate::core::Journal>>,
}

impl Runner {
    pub fn new(
        registry: Arc<BackendRegistry>,
        config: Arc<Config>,
        journal: Arc<tokio::sync::Mutex<crate::core::Journal>>,
    ) -> Self {
        Self {
            registry,
            config,
            journal,
        }
    }

    async fn resolve_spec(&self, spec_str: &str) -> Result<Vec<PackageSpec>> {
        StateResolver::new(&self.config, self.registry.clone(), false)
            .await
            .resolve_spec(spec_str)
            .await
    }

    /// Install whatever the command needs and is missing.
    ///
    /// Split out of [`run`](Self::run) so the data lock can be scoped to it: this is the whole
    /// of what `run` writes, and everything after it is somebody else's program.
    async fn provision(&self, specs: &[PackageSpec]) -> Result<()> {
        for spec in specs {
            let backend_caps = self
                .registry
                .get(&spec.backend)
                .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;

            let is_present = if let Some(queryable) = backend_caps.as_queryable() {
                queryable.info(&spec.name).await?.is_some()
            } else {
                debug!(
                    "Backend '{}' not queryable, assuming missing.",
                    spec.backend
                );
                false
            };

            if is_present {
                continue;
            }
            let Some(installer) = backend_caps.as_installable() else {
                return Err(Error::Transaction(format!(
                    "Component {}:{} is required but the backend does not support installation.",
                    spec.backend, spec.name
                )));
            };
            info!(
                "Auto-provisioning missing component: {}:{}",
                spec.backend, spec.name
            );
            let sudo = backend_caps.sudo_for_write();
            crate::core::journalled(
                &self.journal,
                vec![crate::core::JournalAction::Install(spec.clone())],
                installer.install(std::slice::from_ref(spec), sudo),
            )
            .await?;
        }
        Ok(())
    }

    /// Primary execution driver: Ensures environment is ready and spawns process.
    ///
    #[instrument(skip(self, packages, args))]
    pub async fn run(&self, packages: &[String], command: &str, args: &[String]) -> Result<()> {
        info!("Provisioning environment for command '{}'...", command);

        let mut sandbox_requested = false;
        let mut resolved_specs = Vec::new();

        for pkg_str in packages {
            let specs = self.resolve_spec(pkg_str).await?;
            for spec in specs {
                if spec.options.one("sandbox") == Some("true") {
                    sandbox_requested = true;
                }
                resolved_specs.push(spec);
            }
        }

        // **The lock covers the provisioning, not the command** (`LockScope::Deferred`).
        // `shall run -p X -- some-command` installs what the command needs and then runs a
        // command Shall neither wrote nor bounds — a server, an editor, a shell one-liner that
        // waits on input. Held for the whole verb, the 120-second exclusive lock was held for
        // the length of somebody else's program. The scope below ends before it is spawned.
        {
            let _data_lock = crate::core::datalock::DataLock::for_one_step("run").await?;
            self.provision(&resolved_specs).await?;
        }

        let settings = &self.config.sandbox;

        let status = if sandbox_requested {
            // One decision, and the user hears about it before the command runs rather than at
            // `debug!` after. An unconfined verdict here is the whole of F10: `@sandbox` was
            // asked for and cannot be given, which is a sentence a person is owed.
            let decided = Sandbox::decide(settings).await?;
            if let Some(warning) = decided.unconfined_warning() {
                warn!("{warning}");
            } else {
                debug!("running command in sandbox");
            }

            let sandbox_cfg = SandboxConfig {
                allow_network: true,
                allow_home: true,
                allow_write: true,
                ..Default::default()
            };

            let cmd_str = command.to_string();
            let args_vec = args.to_vec();
            let settings_clone = settings.clone();

            tokio::task::spawn_blocking(move || {
                Sandbox::run(&cmd_str, &args_vec, &sandbox_cfg, &settings_clone, &decided)
            })
            .await
            .map_err(|e| Error::Other(format!("Sandbox thread failure: {}", e)))??
        } else {
            self.execute_standard(command, args).await?
        };

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            error!("Environment command failed with exit code {}.", code);
            return Err(Error::command_failed(format!(
                "Sub-process exited with code {}",
                code
            )));
        }

        Ok(())
    }

    async fn execute_standard(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<std::process::ExitStatus> {
        let command = real_program(command).await;
        debug!("Spawning process: {} {:?}", command, args);

        let mut child = Command::new(&command);
        child.args(args);

        // Inherit stdin/out/err for interactive tool compatibility
        child
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());

        // The terminal-handoff door: streams inherited because the user is looking at it, no idle
        // bound because a program waiting for them to type is not a hung one — but owned, so it
        // does not keep the terminal after Shall has gone.
        crate::core::supervise::supervised_status(child, &command).await
    }

    /// What a `shim:` line says to provision and run under this name, or the bare name when the
    /// line names no source.
    ///
    /// The declaration is the record. A shim is a copy of the shall binary and carries no data of
    /// its own, so `@source=` had nowhere to be *stored* — but it does not need storing: the
    /// config that declared the shim is the same config this process has already loaded, and it
    /// still says `shim:jq@source=cargo:jq`.
    pub async fn shim_spec(&self, shim_name: &str) -> Result<String> {
        use crate::config::grammar::Statement;

        let state = StateResolver::new(&self.config, self.registry.clone(), false)
            .await
            .resolve_model()
            .await?;
        let declared = state.dependents().find_map(|(stmt, _)| match stmt {
            Statement::Shim(name, opts) if name == shim_name => Some(opts.one("source")),
            _ => None,
        });
        Ok(match declared.flatten() {
            Some(source) => source.to_string(),
            None => shim_name.to_string(),
        })
    }

    pub async fn exec_shim(&self, shim_name: &str, args: &[String]) -> Result<()> {
        debug!("Shim redirection for identity '{}'...", shim_name);
        let packages = vec![self.shim_spec(shim_name).await?];
        self.run(&packages, shim_name, args).await
    }
}

/// `command` resolved through `PATH`, skipping any Shall shim on the way.
///
/// **A shim must never resolve to itself.** `bin_dir` is on `PATH` *ahead* of the real binary —
/// that is the entire mechanism — so spawning the shimmed name by bare name finds the shim
/// again, which re-enters Shall, which spawns the name again. One process per turn, for ever.
///
/// Identity is asked of the file, not of the directory: `web:`, `github:` and `appimage:` all
/// deploy real executables into that same `bin_dir`, and excluding the directory would make
/// `shall run` unable to find them.
async fn real_program(command: &str) -> String {
    real_program_on(command, std::env::var_os("PATH")).await
}

/// [`real_program`] against a given `PATH`, so the search can be tested without a test having to
/// edit the process environment every other test is reading.
async fn real_program_on(command: &str, path: Option<std::ffi::OsString>) -> String {
    if command.contains('/') || command.contains('\\') {
        return command.to_string();
    }
    let Some(path) = path else {
        return command.to_string();
    };
    for dir in std::env::split_paths(&path) {
        for candidate in program_candidates(&dir, command) {
            if !tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
                continue;
            }
            if crate::app::ShimManager::is_deployed_shim(&candidate).await {
                debug!("skipping the shim at {:?} — it is this binary", candidate);
                continue;
            }
            return candidate.to_string_lossy().into_owned();
        }
    }
    // Nothing on PATH but shims, or nothing at all: hand the bare name to the OS so the failure
    // is the one the user would have got by typing it.
    command.to_string()
}

/// The filenames one PATH entry could hold for `command`, in the order the OS would try them —
/// the same `.exe` rule `ShimManager::shim_path` writes by, because these two have to agree
/// about what a shim is called.
fn program_candidates(dir: &std::path::Path, command: &str) -> Vec<std::path::PathBuf> {
    let plain = dir.join(command);
    #[cfg(windows)]
    {
        if std::path::Path::new(command).extension().is_some() {
            vec![plain]
        } else {
            vec![dir.join(format!("{command}.exe")), plain]
        }
    }
    #[cfg(not(windows))]
    vec![plain]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ShimManager;
    use tempfile::tempdir;

    /// What `create_shim` writes this name as, which is what the search has to skip.
    fn deployed_name(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    }

    /// The shim spawning itself, closed.
    ///
    /// `bin_dir` sits on `PATH` ahead of the real binary — that is what makes a shim a shim — so
    /// resolving the shimmed name by bare name found the shim, which re-entered Shall, which
    /// resolved the name again. Nothing in the tree stopped it: no depth counter, no marker,
    /// no exclusion.
    #[tokio::test]
    async fn a_shim_on_path_is_never_what_the_runner_spawns() {
        let tmp = tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        let real_dir = tmp.path().join("usr-bin");
        tokio::fs::create_dir_all(&real_dir).await.unwrap();
        let real = real_dir.join(deployed_name("jq"));
        tokio::fs::write(&real, b"the real jq").await.unwrap();

        // The control: nothing is a shim yet, so the first PATH entry wins as it always has.
        let decoy = bin_dir.join(deployed_name("jq"));
        tokio::fs::create_dir_all(&bin_dir).await.unwrap();
        tokio::fs::write(&decoy, b"someone else's jq")
            .await
            .unwrap();
        let path = std::env::join_paths([&bin_dir, &real_dir]).unwrap();
        assert_eq!(
            real_program_on("jq", Some(path.clone())).await,
            decoy.to_string_lossy(),
            "a file Shall did not deploy is a normal binary and must still be found first"
        );

        // And now the same PATH with a real shim in front of the real binary.
        tokio::fs::remove_file(&decoy).await.unwrap();
        ShimManager::with_bin_dir(bin_dir.clone())
            .await
            .unwrap()
            .create_shim("jq")
            .await
            .unwrap();
        assert!(decoy.exists(), "the fixture needs a deployed shim to skip");
        assert_eq!(
            real_program_on("jq", Some(path)).await,
            real.to_string_lossy(),
            "the runner resolved the shimmed name back to the shim — that is the recursion"
        );
    }

    /// A `PATH` holding nothing but the shim hands the bare name to the OS, so the user gets the
    /// error they would have got by typing it rather than a silent re-entry.
    #[tokio::test]
    async fn a_path_with_only_the_shim_on_it_falls_back_to_the_bare_name() {
        let tmp = tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        ShimManager::with_bin_dir(bin_dir.clone())
            .await
            .unwrap()
            .create_shim("jq")
            .await
            .unwrap();
        let path = std::env::join_paths([&bin_dir]).unwrap();
        assert_eq!(real_program_on("jq", Some(path)).await, "jq");
    }

    /// A command that is already a path is not a `PATH` question. `shall run ./build.sh` names
    /// one file, and re-resolving it through directories would run a different one.
    #[tokio::test]
    async fn a_command_that_names_a_path_is_left_exactly_as_written() {
        for written in ["./build.sh", "/usr/bin/jq", r"C:\tools\jq.exe"] {
            assert_eq!(real_program_on(written, None).await, written);
        }
    }
}
