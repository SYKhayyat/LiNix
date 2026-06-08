use crate::core::{Result, Error, PackageSpec, Validator, StateRegistry};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::app::sandbox::{Sandbox, SandboxConfig};
use crate::app::bridge::DependencyBridge;
use crate::app::sync::resolver::StateResolver;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{info, debug, error, warn};
use std::sync::Arc;
use std::collections::{VecDeque, HashSet};

/// Handles the execution of commands within specialized LiNix environments.
/// Orchestrates sandboxing, ephemeral provisioning, and dependency bridging.
/// 
/// Hardened for Phase 4.1: Decoupled from the global App object.
/// Hardened for Phase 5.1: Correctly routes state to the Dependency Bridge for remediation.
pub struct Runner {
    registry: Arc<BackendRegistry>,
    state: Arc<Mutex<StateRegistry>>,
    config: Arc<Config>,
    bridge: Arc<DependencyBridge>,
}

impl Runner {
    /// Creates a new Runner with explicit dependency injection.
    pub fn new(
        registry: Arc<BackendRegistry>,
        state: Arc<Mutex<StateRegistry>>,
        config: Arc<Config>,
        bridge: Arc<DependencyBridge>,
    ) -> Self {
        Self {
            registry,
            state,
            config,
            bridge,
        }
    }

    /// Internal helper to resolve package specs without depending on the App object.
    async fn resolve_spec(&self, spec_str: &str) -> Result<Vec<PackageSpec>> {
        let mut resolved = Vec::new();
        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();

        let resolver = StateResolver::new(&self.config, self.registry.clone());
        queue.push_back(resolver.parse_and_probe_spec(spec_str).await?);

        while let Some(spec) = queue.pop_front() {
            let key = format!("{}:{}", spec.backend, spec.name);
            if !seen.insert(key) { continue; }

            Validator::validate_package_name(&spec.name)?;
            for req in &spec.requires {
                queue.push_back(resolver.parse_and_probe_spec(req).await?);
            }
            resolved.push(spec);
        }
        Ok(resolved)
    }

    /// Executes a command, ensuring that the required environment is provisioned.
    /// Fulfills Roadmap Points 6, 16, 17, and 19.
    pub async fn run(&self, packages: &[String], command: &str, args: &[String]) -> Result<()> {
        info!("Runner: Provisioning environment for '{}'...", command);

        let mut sandbox_requested = false;
        let mut resolved_specs = Vec::new();

        // 1. Resolve and check all required packages
        for pkg_str in packages {
            let specs = self.resolve_spec(pkg_str).await?;
            for spec in specs {
                // Check if any package in the environment requires sandboxing
                if spec.options.get("sandbox") == Some(&"true".to_string()) {
                    sandbox_requested = true;
                }
                resolved_specs.push(spec);
            }
        }

        // 2. Ensure packages are available
        for spec in &resolved_specs {
            let backend_caps = self.registry.get(&spec.backend)
                .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;
            
            let is_present = if let Some(q) = backend_caps.as_queryable() {
                q.info(&spec.name).await?.is_some()
            } else {
                false
            };

            if !is_present {
                if let Some(installer) = backend_caps.as_installable() {
                    info!("Runner: Provisioning missing dependency: {}:{}", spec.backend, spec.name);
                    let sudo = backend_caps.needs_root();
                    installer.install(&[spec.clone()], sudo).await?;
                }
            }
        }

        // 3. Prepare Execution with Sandbox Check
        let settings = &self.config.sandbox;
        let is_sandbox_available = Sandbox::is_available(settings).await;

        let status = if sandbox_requested {
            if is_sandbox_available {
                info!("Runner: Spawning command in hardened sandbox: {}", command);
                let config = SandboxConfig {
                    allow_network: true,
                    allow_home: true,
                    allow_write: true,
                    custom_mounts: vec![],
                    custom_read_only_mounts: vec![],
                    environment: vec![],
                };
                
                let cmd_str = command.to_string();
                let args_vec = args.to_vec();
                let settings_clone = settings.clone();

                tokio::task::spawn_blocking(move || {
                    Sandbox::run(&cmd_str, &args_vec, &config, &settings_clone)
                }).await.map_err(|e| Error::Other(e.to_string()))??
            } else if settings.fallback_allowed {
                warn!("Runner: Sandbox requested but unavailable. Falling back to standard execution.");
                self.execute_standard(command, args).await?
            } else {
                return Err(Error::UnsupportedPlatform(
                    "Sandboxing is requested, but unavailable and fallback is disabled.".into()
                ));
            }
        } else {
            self.execute_standard(command, args).await?
        };

        // 4. Point 16: Failure Diagnosis (Dependency Bridging)
        if !status.success() {
            error!("Runner: Command '{}' failed with status: {}", command, status);
            
            // Capture stderr for bridging
            let diag_output = tokio::process::Command::new(command)
                .args(args)
                .output()
                .await
                .ok();

            if let Some(out) = diag_output {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let default_backend = self.config.default_backend.clone().unwrap_or_else(|| "apt".into());
                
                // Optimized failure handling with explicit state injection (Phase 5.1)
                let _ = self.bridge.handle_failure(
                    &stderr, 
                    &default_backend, 
                    self.registry.clone(),
                    self.state.clone(),
                    &self.config,
                    false
                ).await;
            }

            return Err(Error::CommandFailed(format!("Execution of '{}' failed.", command)));
        }

        Ok(())
    }

    async fn execute_standard(&self, command: &str, args: &[String]) -> Result<std::process::ExitStatus> {
        debug!("Runner: Spawning standard process: {} {:?}", command, args);
        let mut child = Command::new(command);
        child.args(args);
        
        child.spawn()
            .map_err(|e| Error::CommandFailed(format!("Failed to spawn {}: {}", command, e)))?
            .wait()
            .await
            .map_err(|e| Error::CommandFailed(format!("Error waiting for {}: {}", command, e)))
    }

    pub async fn exec_shim(&self, shim_name: &str, args: &[String]) -> Result<()> {
        debug!("Runner: Shim detected. Mapping '{}' to environment...", shim_name);
        let packages = vec![shim_name.to_string()];
        self.run(&packages, shim_name, args).await
    }
}