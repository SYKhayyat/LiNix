use crate::App;
use crate::core::{Result, Error};
use crate::app::sandbox::{Sandbox, SandboxConfig};
use crate::app::bridge::DependencyBridge;
use std::process::Command;
use tracing::{info, debug, error};

/// Handles the execution of commands within specialized LiNix environments.
/// Orchestrates sandboxing, ephemeral provisioning, and dependency bridging.
pub struct Runner<'a> {
    app: &'a App,
}

impl<'a> Runner<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Executes a command, ensuring that the required environment is provisioned.
    /// Fulfills Roadmap Points 6, 16, 17, and 19.
    pub async fn run(&self, packages: &[String], command: &str, args: &[String]) -> Result<()> {
        info!("Runner: Provisioning environment for '{}'...", command);

        let mut sandbox_requested = false;
        let mut resolved_specs = Vec::new();

        // 1. Resolve and check all required packages
        for pkg_str in packages {
            let specs = self.app.resolve_spec(pkg_str).await?;
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
            let backend_caps = self.app.registry.get(&spec.backend)
                .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;
            
            let is_present = if let Some(q) = backend_caps.as_queryable() {
                q.info(&spec.name).await?.is_some()
            } else {
                false
            };

            if !is_present {
                if let Some(installer) = backend_caps.as_installable() {
                    info!("Runner: Provisioning missing dependency: {}:{}", spec.backend, spec.name);
                    installer.install(&[spec.clone()], true).await?;
                }
            }
        }

        // 3. Prepare Execution
        let status = if sandbox_requested {
            // Point 17: Execute within a Bubblewrap sandbox
            let config = SandboxConfig {
                allow_network: true,
                allow_home: true,
                allow_write: true,
                custom_mounts: vec![],
                custom_read_only_mounts: vec![],
                environment: vec![],
            };
            Sandbox::run(command, args, &config)?
        } else {
            // Standard execution
            debug!("Runner: Spawning standard process: {} {:?}", command, args);
            Command::new(command)
                .args(args)
                .status()
                .map_err(|e| Error::CommandFailed(format!("Failed to spawn {}: {}", command, e)))?
        };

        // 4. Point 16: Failure Diagnosis (Dependency Bridging)
        if !status.success() {
            error!("Runner: Command '{}' failed with status: {}", command, status);
            
            // Re-run with captured stderr to provide a diagnosis if it was a build/linking error
            let diag_output = Command::new(command)
                .args(args)
                .output()
                .ok();

            if let Some(out) = diag_output {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let bridge = DependencyBridge::new();
                bridge.handle_failure(&stderr, &self.app.config.default_backend.clone().unwrap_or_else(|| "apt".into()), self.app, false).await?;
            }

            return Err(Error::CommandFailed(format!("Execution of '{}' failed.", command)));
        }

        Ok(())
    }

    /// Point 6: Specialized entry point for Binary Shims.
    /// Detects the target command from the shim's filename and executes it.
    pub async fn exec_shim(&self, shim_name: &str, args: &[String]) -> Result<()> {
        debug!("Runner: Shim detected. Mapping '{}' to environment...", shim_name);
        
        // Use the shim name as the package name to provision
        let packages = vec![shim_name.to_string()];
        
        // Execute the command (which shares the same name as the shim)
        self.run(&packages, shim_name, args).await
    }
}