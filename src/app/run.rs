use crate::app::sandbox::{Sandbox, SandboxConfig};
use crate::app::sync::resolver::StateResolver;
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Error, PackageSpec, Result, Validator};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use tokio::process::Command;
use tracing::{debug, error, info, instrument, warn};

/// Handles the execution of commands within specialized LiNix environments.
///
/// The Runner is responsible for the "On-Demand" lifecycle:
/// 1. Resolution: Determining which packages/modules are needed.
/// 2. Provisioning: Installing missing components in the background.
/// 3. Orchestration: Spawning the command with appropriate sandboxing or isolation.
///
/// Modernized for v3.6.0: Removed unused state and bridge dependencies to
/// streamline the kernel execution path.
pub struct Runner {
    /// Registry for capability discovery (Installable/Queryable).
    registry: Arc<BackendRegistry>,
    /// Global configuration for sandbox and priority settings.
    config: Arc<Config>,
}

impl Runner {
    /// Creates a new Runner with explicit dependency injection.
    ///
    /// Resolves E0061 and dead_code warnings: Now takes exactly 2 arguments.
    pub fn new(registry: Arc<BackendRegistry>, config: Arc<Config>) -> Self {
        Self { registry, config }
    }

    /// Internal helper to resolve package specs using the async StateResolver.
    ///
    /// This method ensures that even in 'run' mode, recursive meta-dependencies
    /// and modules are correctly expanded before execution.
    async fn resolve_spec(&self, spec_str: &str) -> Result<Vec<PackageSpec>> {
        let mut resolved = Vec::new();
        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();

        // Initialize the 3.6.0 Resolver (Non-locked mode for ad-hoc execution)
        let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;

        // Probe and parse the initial request
        queue.push_back(resolver.parse_and_probe_spec(spec_str).await?);

        while let Some(spec) = queue.pop_front() {
            let key = format!("{}:{}", spec.backend, spec.name);
            if !seen.insert(key) {
                continue;
            }

            // Mission-Critical Security: Validate name integrity
            Validator::validate_package_name_for(&spec.name, &spec.backend)?;

            // Expand meta-dependencies (requires=... tags)
            for req in &spec.requires {
                queue.push_back(resolver.parse_and_probe_spec(req).await?);
            }
            resolved.push(spec);
        }
        Ok(resolved)
    }

    /// Primary execution driver: Ensures environment is ready and spawns process.
    ///
    /// # Arguments
    /// * `packages` - List of specs to provision (e.g. ["apt:curl", "@module:aws"]).
    /// * `command` - The binary to execute.
    /// * `args` - Command-line arguments for the target binary.
    #[instrument(skip(self, packages, args))]
    pub async fn run(&self, packages: &[String], command: &str, args: &[String]) -> Result<()> {
        info!(
            "Runner: Provisioning environment for command '{}'...",
            command
        );

        let mut sandbox_requested = false;
        let mut resolved_specs = Vec::new();

        // 1. RESOLUTION PHASE
        for pkg_str in packages {
            let specs = self.resolve_spec(pkg_str).await?;
            for spec in specs {
                // If any package in the closure requires a sandbox, the whole env is sandboxed
                if spec.options.get("sandbox") == Some(&"true".to_string()) {
                    sandbox_requested = true;
                }
                resolved_specs.push(spec);
            }
        }

        // 2. PROVISIONING PHASE
        for spec in &resolved_specs {
            let backend_caps = self
                .registry
                .get(&spec.backend)
                .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;

            // Panic-Free Trait Checking
            let is_present = if let Some(queryable) = backend_caps.as_queryable() {
                queryable.info(&spec.name).await?.is_some()
            } else {
                debug!(
                    "Runner: Backend '{}' not queryable, assuming missing.",
                    spec.backend
                );
                false
            };

            if !is_present {
                if let Some(installer) = backend_caps.as_installable() {
                    info!(
                        "Runner: Auto-provisioning missing component: {}:{}",
                        spec.backend, spec.name
                    );
                    let sudo = backend_caps.sudo_for_write();
                    installer.install(std::slice::from_ref(spec), sudo).await?;
                } else {
                    return Err(Error::Transaction(format!(
                        "Component {}:{} is required but the backend does not support installation.", 
                        spec.backend, spec.name
                    )));
                }
            }
        }

        // 3. EXECUTION PHASE
        let settings = &self.config.sandbox;
        let can_sandbox = Sandbox::is_available(settings).await;

        let status = if sandbox_requested {
            if can_sandbox {
                info!("Runner: Spawning command in hardware-isolated sandbox.");
                let sandbox_cfg = SandboxConfig {
                    allow_network: true,
                    allow_home: true,
                    allow_write: true,
                    ..Default::default()
                };

                let cmd_str = command.to_string();
                let args_vec = args.to_vec();
                let settings_clone = settings.clone();

                // Offload blocking sandbox spawn to a dedicated thread
                tokio::task::spawn_blocking(move || {
                    Sandbox::run(&cmd_str, &args_vec, &sandbox_cfg, &settings_clone)
                })
                .await
                .map_err(|e| Error::Other(format!("Sandbox thread failure: {}", e)))??
            } else if settings.fallback_allowed {
                warn!("Runner: Sandbox requested but unavailable. Falling back to host execution.");
                self.execute_standard(command, args).await?
            } else {
                return Err(Error::UnsupportedPlatform(
                    "Sandboxing is required by policy but not functional on this host.".into(),
                ));
            }
        } else {
            self.execute_standard(command, args).await?
        };

        // 4. POST-EXECUTION
        if !status.success() {
            let code = status.code().unwrap_or(-1);
            error!(
                "Runner: Environment command failed with exit code {}.",
                code
            );
            return Err(Error::CommandFailed(format!(
                "Sub-process exited with code {}",
                code
            )));
        }

        Ok(())
    }

    /// Spawns a standard OS process with inherited IO streams.
    async fn execute_standard(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<std::process::ExitStatus> {
        debug!("Runner: Spawning process: {} {:?}", command, args);

        let mut child = Command::new(command);
        child.args(args);

        // Inherit stdin/out/err for interactive tool compatibility
        child
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());

        let mut handle = child.spawn().map_err(|e| {
            Error::CommandFailed(format!("Failed to start binary {}: {}", command, e))
        })?;

        handle
            .wait()
            .await
            .map_err(|e| Error::CommandFailed(format!("Error during process wait: {}", e)))
    }

    /// High-performance entry point for Rust binary shims.
    pub async fn exec_shim(&self, shim_name: &str, args: &[String]) -> Result<()> {
        debug!("Runner: Shim redirection for identity '{}'...", shim_name);
        // Map the shim name to a single package request
        let packages = vec![shim_name.to_string()];
        self.run(&packages, shim_name, args).await
    }
}
