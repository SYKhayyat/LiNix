use crate::App;
use crate::core::{Result, Error, PackageSpec};
use crate::app::sandbox::{Sandbox, SandboxConfig};
use std::env;
use std::path::{Path, PathBuf};
use tracing::{info, debug, warn};

/// Orchestrates ephemeral environments (The Ghost Shell).
/// Hardened for Phase 3.2: Async I/O and non-blocking process execution.
pub struct GhostShell<'a> {
    app: &'a App,
}

impl<'a> GhostShell<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Spawns a sub-shell with the provided packages available.
    pub async fn enter(&self, packages: &[String]) -> Result<()> {
        info!("GhostShell: Provisioning isolated project namespace...");

        let mut store_paths = Vec::new();

        // 1. Resolve Specs and Locate Binaries
        for pkg_str in packages {
            let specs = self.app.resolve_spec(pkg_str).await?;
            for spec in specs {
                if let Some(path) = self.locate_package_root(&spec).await? {
                    debug!("GhostShell: Located {} at {:?}", spec.name, path);
                    store_paths.push((path.to_string_lossy().to_string(), spec.name.clone()));
                } else {
                    info!("GhostShell: Provisioning missing component: {}...", spec.name);
                    let backend = self.app.registry.get(&spec.backend)
                        .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;
                    
                    if let Some(installer) = backend.as_installable() {
                        let sudo = backend.needs_root();
                        installer.install(&[spec.clone()], sudo).await?;
                        if let Some(path) = self.locate_package_root(&spec).await? {
                            store_paths.push((path.to_string_lossy().to_string(), spec.name.clone()));
                        }
                    }
                }
            }
        }

        // 2. Build Sandbox Configuration
        let mut mounts = Vec::new();
        for (path, name) in &store_paths {
            let target = format!("/opt/linix/packages/{}", name);
            mounts.push((path.clone(), target));
        }

        let shell = env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(windows) { "cmd.exe".to_string() } else { "/bin/bash".to_string() }
        });
        
        // 3. Namespace Provisioning Execution
        if cfg!(target_os = "linux") && Sandbox::is_supported() {
            info!("GhostShell: Launching Bubblewrap container for session.");
            
            let config = SandboxConfig {
                allow_network: true,
                allow_home: true,
                allow_write: true,
                custom_mounts: mounts.clone(),
                custom_read_only_mounts: vec![],
                environment: vec![],
            };

            let mut internal_path = String::from("/usr/local/bin:/usr/bin:/bin");
            for (_, target) in mounts {
                internal_path = format!("{}:{}:{}/bin", internal_path, target, target);
            }

            // Sandbox logic involves building a Command and running it.
            // Since Sandbox currently uses std::process, we wrap the execution in spawn_blocking.
            let shell_cmd = shell.clone();
            tokio::task::spawn_blocking(move || {
                let mut bwrap = Sandbox::wrap(&shell_cmd, &[], &config)?;
                bwrap.env("PATH", internal_path)
                     .env("LINIX_GHOST", "true")
                     .env("PROMPT_COMMAND", "echo -n '(linix-ghost) '");

                let status = bwrap.status().map_err(Error::from)?;
                if !status.success() {
                    return Err(Error::CommandFailed(format!("Ghost shell exited with status: {}", status)));
                }
                Ok(())
            }).await.map_err(|e| Error::Other(e.to_string()))??;

        } else {
            warn!("GhostShell: Platform fallback to PATH mutation.");
            self.spawn_fallback_shell(&shell, &store_paths).await?;
        }

        Ok(())
    }

    /// Project-local directive.
    pub async fn auto_shell(&self) -> Result<()> {
        let local_config = Path::new("linix.txt");
        if tokio::fs::try_exists(local_config).await.unwrap_or(false) {
            info!("GhostShell: Found 'linix.txt'. Initializing environment...");
            
            let content = tokio::fs::read_to_string(local_config).await.map_err(Error::from)?;
            let packages: Vec<String> = content.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            
            if !packages.is_empty() {
                return self.enter(&packages).await;
            }
        }
        Ok(())
    }

    /// Internal helper to find the physical root of a package.
    async fn locate_package_root(&self, spec: &PackageSpec) -> Result<Option<PathBuf>> {
        let backend = self.app.registry.get(&spec.backend)
            .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;

        if let Some(queryable) = backend.as_queryable() {
            if let Ok(Some(pkg)) = queryable.info(&spec.name).await {
                let keys = ["local_path", "store_path", "install_path", "path"];
                for key in keys {
                    if let Some(val) = pkg.properties.get(key) {
                        let path = PathBuf::from(val);
                        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
                            return Ok(Some(path));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Proper fallback logic that sets PATH for the child process only.
    async fn spawn_fallback_shell(&self, shell: &str, store_paths: &[(String, String)]) -> Result<()> {
        let current_path = env::var_os("PATH").unwrap_or_default();
        let mut new_path_parts = Vec::new();

        for (path, _) in store_paths {
            new_path_parts.push(path.clone());
            let bin_sub = Path::new(path).join("bin");
            if tokio::fs::try_exists(&bin_sub).await.unwrap_or(false) {
                new_path_parts.push(bin_sub.to_string_lossy().to_string());
            }
        }

        let current_path_str = current_path.to_string_lossy();
        new_path_parts.push(current_path_str.to_string());

        let new_path = env::join_paths(&new_path_parts)
            .map_err(|e| Error::Other(format!("Failed to build PATH: {}", e)))?;

        // Spawning interactive shells is better handled with tokio::process if we need async control,
        // but since we usually wait for the shell to finish, status() is appropriate.
        // We use tokio::process::Command to avoid blocking the thread.
        let mut cmd = tokio::process::Command::new(shell);
        cmd.env("PATH", &new_path)
           .env("LINIX_GHOST", "fallback");

        let status = cmd.status().await.map_err(Error::from)?;

        if !status.success() {
            return Err(Error::CommandFailed(format!("Shell exited with: {}", status)));
        }
        
        Ok(())
    }
}