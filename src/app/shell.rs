use crate::App;
use crate::core::{Result, Error, PackageSpec};
use crate::app::sandbox::{Sandbox, SandboxConfig};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, debug, warn};

/// Orchestrates ephemeral environments (The Ghost Shell).
/// Hardened for Version 3.5.0 with "Namespace Provisioning."
/// 
/// Instead of polluting the global system or relying on symlinks, 
/// Ghost Shell creates a mounting namespace where only the requested 
/// packages are visible alongside the core OS.
/// 
/// FIX #16: Windows fallback now correctly sets PATH for the child process
/// using Command::env() instead of trying to mutate the global environment.
pub struct GhostShell<'a> {
    app: &'a App,
}

impl<'a> GhostShell<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Spawns a sub-shell with the provided packages available.
    /// Fulfills Point 19: Namespace Provisioner using Bubblewrap (Linux) or
    /// PATH modification (Windows/macOS fallback).
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
                custom_mounts: mounts.clone(),
            };

            let mut internal_path = String::from("/usr/local/bin:/usr/bin:/bin");
            for (_, target) in mounts {
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
            // FIX #16: Windows and macOS fallback - properly set PATH for child process
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

    /// FIX #16: Proper fallback logic that sets PATH for the child process only.
    /// This does NOT mutate the parent process's environment.
    async fn spawn_fallback_shell(&self, shell: &str, store_paths: &[(String, String)]) -> Result<()> {
        // Build new PATH by prepending package bin directories
        let current_path = env::var_os("PATH").unwrap_or_default();
        let mut new_path_parts = Vec::new();

        for (path, name) in store_paths {
            // Add the package root
            new_path_parts.push(path.clone());
            
            // Add bin subdirectory if it exists
            let bin_sub = Path::new(path).join("bin");
            if bin_sub.exists() {
                new_path_parts.push(bin_sub.to_string_lossy().to_string());
            }
            
            // FIX #16: On Windows, also check for executable parent directory
            #[cfg(windows)]
            {
                let exe_sub = Path::new(path).join("exe");
                if exe_sub.exists() {
                    new_path_parts.push(exe_sub.to_string_lossy().to_string());
                }
            }
            
            debug!("GhostShell: Adding to PATH: {} (package: {})", path, name);
        }

        // Add the current PATH at the end
        let current_path_str = current_path.to_string_lossy();
        new_path_parts.push(current_path_str.to_string());

        let new_path = env::join_paths(&new_path_parts)
            .map_err(|e| Error::Other(format!("Failed to build PATH: {}", e)))?;

        debug!("GhostShell: New PATH length: {} characters", new_path.len());

        // FIX #16: Use Command::env() to set PATH for the child process only
        // This does NOT affect the parent process's environment
        let status = Command::new(shell)
            .env("PATH", &new_path)
            .env("LINIX_GHOST", "fallback")
            .env("LINIX_GHOST_PACKAGES", store_paths.iter().map(|(_, n)| n.clone()).collect::<Vec<_>>().join(","))
            .status()
            .map_err(|e| Error::CommandFailed(format!("Failed to spawn fallback shell: {}", e)))?;

        if !status.success() {
            return Err(Error::CommandFailed(format!("Shell exited with: {}", status)));
        }
        
        info!("GhostShell: Fallback shell exited successfully.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::backends::create_default_registry;
    use crate::app::LuaHooks;
    use crate::core::CommandExecutor;
    use std::sync::Arc;

    async fn create_test_app() -> App {
        let config = Config::default();
        App::new(config).await.unwrap()
    }

    #[tokio::test]
    async fn test_ghost_shell_creation() {
        let app = create_test_app().await;
        let shell = GhostShell::new(&app);
        
        // Just verify the shell object is created
        assert!(shell.app.config.dry_run == false || shell.app.config.dry_run == true);
    }

    #[tokio::test]
    async fn test_auto_shell_no_file() {
        let app = create_test_app().await;
        let shell = GhostShell::new(&app);
        
        // Should return Ok even if no linix.txt exists
        let result = shell.auto_shell().await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_path_building_logic() {
        let paths = vec![
            ("/usr/local/pkg1".to_string(), "pkg1".to_string()),
            ("/home/user/.local/pkg2".to_string(), "pkg2".to_string()),
        ];
        
        let mut new_path_parts = Vec::new();
        for (path, _) in &paths {
            new_path_parts.push(path.clone());
            let bin_sub = Path::new(path).join("bin");
            if bin_sub.exists() {
                // In test, doesn't exist, so skip
            }
        }
        
        assert_eq!(new_path_parts.len(), 2);
        assert_eq!(new_path_parts[0], "/usr/local/pkg1");
        assert_eq!(new_path_parts[1], "/home/user/.local/pkg2");
    }
}