use crate::core::{Result, Error, PackageSpec, Validator};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::app::sandbox::{Sandbox, SandboxConfig};
use crate::app::sync::resolver::StateResolver;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::collections::{VecDeque, HashSet};
use tracing::{info, debug, warn};

/// Orchestrates ephemeral environments (The Ghost Shell).
/// 
/// Hardened for Phase 4.1: Decoupled from the global App object. Now receives 
/// specific dependencies via injection, allowing the shell logic to be tested 
/// independently of the application state.
pub struct GhostShell {
    registry: Arc<BackendRegistry>,
    config: Arc<Config>,
}

impl GhostShell {
    /// Creates a new GhostShell with explicit dependency injection.
    pub fn new(registry: Arc<BackendRegistry>, config: Arc<Config>) -> Self {
        Self { registry, config }
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

    /// Spawns a sub-shell with the provided packages available.
    pub async fn enter(&self, packages: &[String]) -> Result<()> {
        info!("GhostShell: Provisioning isolated project namespace...");

        let mut store_paths = Vec::new();

        // 1. Resolve Specs and Locate Binaries
        for pkg_str in packages {
            let specs = self.resolve_spec(pkg_str).await?;
            for spec in specs {
                if let Some(path) = self.locate_package_root(&spec).await? {
                    debug!("GhostShell: Located {} at {:?}", spec.name, path);
                    store_paths.push((path.to_string_lossy().to_string(), spec.name.clone()));
                } else {
                    info!("GhostShell: Provisioning missing component: {}...", spec.name);
                    let backend = self.registry.get(&spec.backend)
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

        // 2. Determine Shell Binary
        let shell = env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(windows) { "cmd.exe".to_string() } else { "/bin/bash".to_string() }
        });

        // 3. Check Sandbox Availability (Phase 3.2)
        let settings = &self.config.sandbox;
        let can_sandbox = Sandbox::is_available(settings).await;

        if can_sandbox {
            info!("GhostShell: Launching hardened sandbox for session.");
            
            let mut mounts = Vec::new();
            for (path, name) in &store_paths {
                let target = format!("/opt/linix/packages/{}", name);
                mounts.push((path.clone(), target));
            }

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

            let shell_cmd = shell.clone();
            let settings_clone = settings.clone();

            tokio::task::spawn_blocking(move || {
                let mut bwrap = Sandbox::wrap(&shell_cmd, &[], &config, &settings_clone)?;
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
            // Sandboxing is unavailable. Check if fallback is permitted.
            if settings.fallback_allowed {
                warn!("GhostShell: Sandbox unavailable. Falling back to PATH-only isolation.");
                self.spawn_fallback_shell(&shell, &store_paths).await?;
            } else {
                return Err(Error::UnsupportedPlatform(
                    "Sandboxing is unavailable on this host and fallback_allowed is false.".into()
                ));
            }
        }

        Ok(())
    }

    /// Project-local directive: Enters shell if 'linix.txt' is present.
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
        let backend = self.registry.get(&spec.backend)
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

    /// Fallback logic that sets PATH for the child process without OS-level sandboxing.
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