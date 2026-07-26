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
}

impl Runner {
    pub fn new(registry: Arc<BackendRegistry>, config: Arc<Config>) -> Self {
        Self { registry, config }
    }

    async fn resolve_spec(&self, spec_str: &str) -> Result<Vec<PackageSpec>> {
        StateResolver::new(&self.config, self.registry.clone(), false)
            .await
            .resolve_spec(spec_str)
            .await
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
                if spec.options.get("sandbox") == Some(&"true".to_string()) {
                    sandbox_requested = true;
                }
                resolved_specs.push(spec);
            }
        }

        for spec in &resolved_specs {
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

            if !is_present {
                if let Some(installer) = backend_caps.as_installable() {
                    info!(
                        "Auto-provisioning missing component: {}:{}",
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

        let settings = &self.config.sandbox;
        let can_sandbox = Sandbox::is_available(settings).await;

        let status = if sandbox_requested {
            if can_sandbox {
                debug!("running command in sandbox");
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
                    Sandbox::run(&cmd_str, &args_vec, &sandbox_cfg, &settings_clone)
                })
                .await
                .map_err(|e| Error::Other(format!("Sandbox thread failure: {}", e)))??
            } else if settings.fallback_allowed {
                warn!("Sandbox requested but unavailable. Falling back to host execution.");
                self.execute_standard(command, args).await?
            } else {
                return Err(Error::UnsupportedPlatform(
                    "Sandboxing is required by policy but not functional on this host.".into(),
                ));
            }
        } else {
            self.execute_standard(command, args).await?
        };

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            error!("Environment command failed with exit code {}.", code);
            return Err(Error::CommandFailed(format!(
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
        debug!("Spawning process: {} {:?}", command, args);

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

    pub async fn exec_shim(&self, shim_name: &str, args: &[String]) -> Result<()> {
        debug!("Shim redirection for identity '{}'...", shim_name);
        let packages = vec![shim_name.to_string()];
        self.run(&packages, shim_name, args).await
    }
}
