use crate::App;
use crate::core::{Result, Error, PackageSpec};
use crate::app::sandbox::{Sandbox, SandboxConfig};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, debug, warn};

/// Orchestrates ephemeral environments (The Ghost Shell).
/// Hardened for Version 3.4.0 with "Namespace Provisioning."
/// 
/// Instead of polluting the global system or relying on symlinks, 
/// Ghost Shell creates a mounting namespace where only the requested 
/// packages are visible alongside the core OS.
pub struct GhostShell<'a> {
    app: &'a App,
}

impl<'a> GhostShell<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Spawns a sub-shell with the provided packages available.
    /// Fulfills Point 19: Namespace Provisioner using Bubblewrap.
    pub async fn enter(&self, packages: &[String]) -> Result<()> {
        info!("GhostShell: Provisioning isolated project namespace...");

        let mut store_paths = Vec::new();
        let mut resolved_specs = Vec::new();

        // 1. Resolve Specs and Locate Binaries
        for pkg_str in packages {
            let specs = self.app.resolve_spec(pkg_str).await?;
            for spec in specs {
                if let Some(path) = self.locate_package_root(&spec).await? {
                    debug!("GhostShell: Located {} at {:?}", spec.name, path);
                    store_paths.push((path.to_string_lossy().to_string(), spec.name.clone()));
                } else {
                    // If not found, attempt a background ephemeral install
                    info!("GhostShell: Provisioning missing component: {}...", spec.name);
                    let backend = self.app.registry.get(&spec.backend)
                        .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;
                    if let Some(installer) = backend.as_installable() {
                        installer.install(&[spec.clone()], true).await?;
                        if let Some(path) = self.locate_package_root(&spec).await? {
                            store_paths.push((path.to_string_lossy().to_string(), spec.name.clone()));
                        }
                    }
                }
                resolved_specs.push(spec);
            }
        }

        // 2. Build Sandbox Configuration
        // We mount each package's root into the sandbox. 
        // We also ensure common paths like /usr/bin are preserved.
        let mut mounts = Vec::new();
        for (path, name) in &store_paths {
            // We mount the package into a specific /opt/linix/packages path inside the namespace
            let target = format!("/opt/linix/packages/{}", name);
            mounts.push((path.clone(), target));
        }

        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        
        // 3. Namespace Provisioning Execution
        if cfg!(target_os = "linux") && Sandbox::is_supported() {
            info!("GhostShell: Launching Bubblewrap container for session.");
            
            let config = SandboxConfig {
                allow_network: true,
                allow_home: true,
                custom_mounts: mounts.clone(),
            };

            // Calculate internal PATH for the container
            let mut internal_path = String::from("/usr/local/bin:/usr/bin:/bin");
            for (_, target) in mounts {
                // Heuristic: Add the /bin or root of each mounted package to the internal PATH
                internal_path = format!("{}:{}:{}/bin", internal_path, target, target);
            }

            let mut bwrap = Sandbox::wrap(&shell, &[], &config)?;
            bwrap.env("PATH", internal_path)
                 .env("LINIX_GHOST", "true")
                 .env("PROMPT_COMMAND", "echo -n '(linix-ghost) '");

            let status = bwrap.status().map_err(Error::Io)?;
            if !status.success() {
                return Err(Error::CommandFailed(format!("Ghost shell exited with status: {}", status)));
            }
        } else {
            // Fallback for non-Linux or systems without bwrap: Path Mutation
            warn!("GhostShell: Namespace isolation is unsupported on this platform. Falling back to PATH mutation.");
            self.spawn_fallback_shell(&shell, &store_paths).await?;
        }

        Ok(())
    }

    /// Point 20: Project-local directive.
    pub async fn auto_shell(&self) -> Result<()> {
        let local_config = Path::new("linix.txt");
        if local_config.exists() {
            info!("GhostShell: Found 'linix.txt'. Initializing environment...");
            let content = std::fs::read_to_string(local_config).map_err(Error::Io)?;
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
                // Check common property keys for the install path
                let keys = ["local_path", "store_path", "install_path", "path"];
                for key in keys {
                    if let Some(val) = pkg.properties.get(key) {
                        let path = PathBuf::from(val);
                        if path.exists() {
                            return Ok(Some(path));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Legacy fallback logic that only modifies PATH for the current process tree.
    async fn spawn_fallback_shell(&self, shell: &str, store_paths: &[(String, String)]) -> Result<()> {
        let mut current_path = env::var_os("PATH").unwrap_or_default();
        let mut new_path_str = String::new();

        for (path, _) in store_paths {
            new_path_str.push_str(path);
            let bin_sub = Path::new(path).join("bin");
            if bin_sub.exists() {
                new_path_str.push(':');
                new_path_str.push_str(&bin_sub.to_string_lossy());
            }
            new_path_str.push(':');
        }

        let mut final_path = std::ffi::OsString::from(new_path_str);
        final_path.push(current_path);

        let status = Command::new(shell)
            .env("PATH", final_path)
            .env("LINIX_GHOST", "fallback")
            .status()
            .map_err(|e| Error::CommandFailed(format!("Failed to spawn fallback shell: {}", e)))?;

        if !status.success() {
            return Err(Error::CommandFailed(format!("Shell exited with: {}", status)));
        }
        Ok(())
    }
}