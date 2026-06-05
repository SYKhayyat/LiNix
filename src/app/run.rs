use crate::App;
use crate::core::{Result, Error};
use crate::app::sandbox::{Sandbox, SandboxConfig};
use crate::app::bridge::DependencyBridge;
use tokio::process::Command;
use tracing::{info, debug, error};

/// Handles the execution of commands within specialized LiNix environments.
/// Orchestrates sandboxing, ephemeral provisioning, and dependency bridging.
/// 
/// Hardened for Phase 2.2: Respects backend root requirements and utilizes
/// asynchronous process spawning to prevent runtime freezes.
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
            // app.resolve_spec is async
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
                    // Phase 2.2: Pass sudo requirement from backend capability
                    let sudo = backend_caps.needs_root();
                    installer.install(&[spec.clone()], sudo).await?;
                }
            }
        }

        // 3. Prepare Execution
        let status = if sandbox_requested {
            // Point 17: Execute within a Bubblewrap/MacOS sandbox
            let config = SandboxConfig {
                allow_network: true,
                allow_home: true,
                allow_write: true,
                custom_mounts: vec![],
                custom_read_only_mounts: vec![],
                environment: vec![],
            };
            
            // Sandbox::run is historically synchronous or uses std::process
            // We wrap it in spawn_blocking to keep the async executor free
            let cmd_str = command.to_string();
            let args_vec = args.to_vec();
            tokio::task::spawn_blocking(move || {
                Sandbox::run(&cmd_str, &args_vec, &config)
            }).await.map_err(|e| Error::Other(e.to_string()))??
        } else {
            // Standard asynchronous execution
            debug!("Runner: Spawning standard process: {} {:?}", command, args);
            let mut child = Command::new(command);
            child.args(args);
            
            child.spawn()
                .map_err(|e| Error::CommandFailed(format!("Failed to spawn {}: {}", command, e)))?
                .wait()
                .await
                .map_err(|e| Error::CommandFailed(format!("Error waiting for {}: {}", command, e)))?
        };

        // 4. Point 16: Failure Diagnosis (Dependency Bridging)
        if !status.success() {
            error!("Runner: Command '{}' failed with status: {}", command, status);
            
            // Re-run with captured stderr to provide a diagnosis
            let diag_output = Command::new(command)
                .args(args)
                .output()
                .await
                .ok();

            if let Some(out) = diag_output {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let bridge = DependencyBridge::new();
                let default_backend = self.app.config.default_backend.clone().unwrap_or_else(|| "apt".into());
                bridge.handle_failure(&stderr, &default_backend, self.app, false).await?;
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