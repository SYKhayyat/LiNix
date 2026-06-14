use crate::core::{Result, Error, PackageSpec, StateRegistry};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::app::sandbox::{Sandbox, SandboxConfig};
use crate::app::sync::{SyncEngine, StateResolver, ScopedFilter, ChangePlanner};
use crate::app::diagnostics::FailureDiagnosticEngine; // Modernized: DI Import
use crate::app::{LuaHooks, MetricsCollector};
use crate::utils::progress::ProgressReporter;
use crate::core::{SnapshotManager, Journal};

use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tracing::{info, debug, warn, instrument}; // Modernized: Removed unused 'trace'
use uuid::Uuid;

/// Orchestrates ephemeral environments (The Ghost Shell).
/// 
/// The Ghost Shell allows users to "try" packages or modules without 
/// permanently polluting their system state. It achieves this by:
/// 1. Installing dependencies as 'transient' (Feature 6).
/// 2. Spawning a sandboxed or PATH-isolated sub-shell.
/// 3. Automatically uninstalling all session-specific packages on exit.
/// 
/// Modernized v3.6.0: Utilizes Dependency Injection for diagnostics and 
/// follows the exhaustive 10-argument SyncEngine model.
pub struct GhostShell {
    /// Registry of all package manager backends.
    registry: Arc<BackendRegistry>,
    /// Shared mutable access to the mission-critical system state.
    state: Arc<Mutex<StateRegistry>>,
    /// Global application configuration.
    config: Arc<Config>,
    /// Low-level command and filesystem executor.
    executor: crate::core::CommandExecutor,
    /// Transaction telemetry collector.
    metrics: MetricsCollector,
    /// Interactive progress reporter.
    progress: Arc<dyn ProgressReporter>,
    /// Scripting event hooks.
    hooks: Arc<LuaHooks>,
    /// System-level snapshot orchestrator.
    snapshot_manager: Arc<SnapshotManager>,
    /// Write-Ahead Log for crash recovery.
    journal: Arc<Mutex<Journal>>,
    /// Modernized v3.6.0: Injected diagnostic engine.
    diagnostics: Arc<FailureDiagnosticEngine>,
}

impl GhostShell {
    /// Initializes a new GhostShell with exhaustive kernel dependencies.
    pub fn new(
        registry: Arc<BackendRegistry>,
        state: Arc<Mutex<StateRegistry>>,
        config: Arc<Config>,
        executor: crate::core::CommandExecutor,
        metrics: MetricsCollector,
        progress: Arc<dyn ProgressReporter>,
        hooks: Arc<LuaHooks>,
        snapshot_manager: Arc<SnapshotManager>,
        journal: Arc<Mutex<Journal>>,
        diagnostics: Arc<FailureDiagnosticEngine>, // Added 10th DI component
    ) -> Self {
        Self { 
            registry, 
            state, 
            config, 
            executor, 
            metrics, 
            progress, 
            hooks, 
            snapshot_manager, 
            journal,
            diagnostics,
        }
    }

    /// Primary entry point for entering an ephemeral environment.
    #[instrument(skip(self, packages))]
    pub async fn enter(&self, packages: &[String]) -> Result<()> {
        let session_id = format!("shell-{}", Uuid::new_v4().simple());
        info!("GhostShell: Initializing isolated session '{}'...", session_id);

        // 1. Session Activation
        {
            let mut state_guard = self.state.lock().await;
            state_guard.active_session_id = Some(session_id.clone());
        }

        // 2. Provisioning Phase
        info!("GhostShell: Provisioning ephemeral components...");
        self.provision_transient_env(packages, &session_id).await?;

        // 3. Environment Discovery Phase
        let mut store_paths = Vec::new();
        for pkg_req in packages {
            let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;
            if let Ok(spec) = resolver.parse_and_probe_spec(pkg_req).await {
                if let Some(path) = self.locate_package_root(&spec).await? {
                    debug!("GhostShell: Discovered root for {}: {:?}", spec.name, path);
                    store_paths.push((path.to_string_lossy().to_string(), spec.name.clone()));
                }
            }
        }

        // 4. Sub-process Execution Phase
        let shell_bin = env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(windows) { "cmd.exe".to_string() } else { "/bin/bash".to_string() }
        });

        let can_sandbox = Sandbox::is_available(&self.config.sandbox).await;

        if can_sandbox {
            info!("GhostShell: Launching hardened sandboxed sub-shell.");
            self.launch_sandboxed_shell(&shell_bin, &session_id, &store_paths).await?;
        } else if self.config.sandbox.fallback_allowed {
            warn!("GhostShell: Sandboxing unavailable. Falling back to PATH isolation.");
            self.spawn_fallback_shell(&shell_bin, &session_id, &store_paths).await?;
        } else {
            return Err(Error::UnsupportedPlatform(
                "Requested sandboxed Ghost Shell, but sandboxing is unavailable.".into()
            ));
        }

        // 5. Atomic Purge Phase (Feature 6 Cleanup)
        info!("GhostShell: Shell session closed. Commencing automatic cleanup...");
        self.cleanup_transient_env(&session_id).await?;

        // 6. Session Termination
        {
            let mut state_guard = self.state.lock().await;
            state_guard.active_session_id = None;
            let _ = state_guard.save();
        }

        info!("GhostShell: Ephemeral environment purged. System restored.");
        Ok(())
    }

    /// Spawns the sub-shell using OS-native sandboxing.
    async fn launch_sandboxed_shell(&self, shell: &str, session_id: &str, store_paths: &[(String, String)]) -> Result<()> {
        let mut mounts = Vec::new();
        let mut internal_path = String::from("/usr/local/bin:/usr/bin:/bin");

        for (path, name) in store_paths {
            let target = format!("/opt/linix/packages/{}", name);
            mounts.push((path.clone(), target.clone()));
            internal_path = format!("{}:{}/bin", internal_path, target);
        }

        let sandbox_cfg = SandboxConfig {
            allow_network: true,
            allow_home: true,
            allow_write: true,
            custom_mounts: mounts,
            ..Default::default()
        };

        let shell_owned = shell.to_string();
        let session_owned = session_id.to_string();
        let settings_clone = self.config.sandbox.clone();

        tokio::task::spawn_blocking(move || {
            let mut bwrap = Sandbox::wrap(&shell_owned, &[], &sandbox_cfg, &settings_clone)?;
            
            bwrap.env("PATH", internal_path)
                 .env("LINIX_GHOST", "true")
                 .env("LINIX_SESSION_ID", session_owned)
                 .env("PROMPT_COMMAND", "echo -n '(linix-ghost) '");

            let mut handle = bwrap.spawn().map_err(|e| Error::CommandFailed(format!("Sandbox spawn error: {}", e)))?;
            let _ = handle.wait().map_err(|e| Error::CommandFailed(e.to_string()))?;
            
            Ok::<(), Error>(())
        }).await.map_err(|e| Error::Other(format!("Sub-shell task failure: {}", e)))??;

        Ok(())
    }

    /// Spawns the sub-shell with host PATH modification.
    async fn spawn_fallback_shell(&self, shell: &str, session_id: &str, store_paths: &[(String, String)]) -> Result<()> {
        let current_path = env::var_os("PATH").unwrap_or_default();
        let mut new_path_parts = Vec::new();

        for (path, _) in store_paths {
            new_path_parts.push(PathBuf::from(path.clone()));
            let bin_sub = Path::new(path).join("bin");
            if tokio::fs::try_exists(&bin_sub).await.unwrap_or(false) {
                new_path_parts.push(bin_sub);
            }
        }

        for p in env::split_paths(&current_path) { new_path_parts.push(p); }

        let new_path_env = env::join_paths(new_path_parts)
            .map_err(|e| Error::Other(format!("Failed to build PATH: {}", e)))?;

        let mut child = tokio::process::Command::new(shell);
        child.env("PATH", new_path_env)
             .env("LINIX_GHOST", "true")
             .env("LINIX_SESSION_ID", session_id)
             .stdin(std::process::Stdio::inherit())
             .stdout(std::process::Stdio::inherit())
             .stderr(std::process::Stdio::inherit());

        let mut handle = child.spawn().map_err(|e| Error::CommandFailed(format!("Shell spawn error: {}", e)))?;
        let _ = handle.wait().await?;
        
        Ok(())
    }

    /// Internal helper to locate the data root of a package.
    async fn locate_package_root(&self, spec: &PackageSpec) -> Result<Option<PathBuf>> {
        let backend = self.registry.get(&spec.backend)
            .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;

        if let Some(queryable) = backend.as_queryable() {
            if let Ok(Some(pkg)) = queryable.info(&spec.name).await {
                for key in ["local_path", "install_path", "path", "store_path"] {
                    if let Some(val) = pkg.properties.get(key) {
                        let p = PathBuf::from(val);
                        if tokio::fs::try_exists(&p).await.unwrap_or(false) {
                            return Ok(Some(p));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Orchestrates the installation of packages for the shell.
    async fn provision_transient_env(&self, requests: &[String], _session_id: &str) -> Result<()> {
        let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;
        
        let mut transient_desired = HashMap::new();
        for req in requests {
            if let Ok(spec) = resolver.parse_and_probe_spec(req).await {
                transient_desired.entry(spec.backend.clone())
                    .or_insert_with(Vec::new)
                    .push(spec);
            }
        }

        let state_guard = self.state.lock().await;
        let planner = ChangePlanner::new(self.registry.clone(), &state_guard, &self.config);
        
        let changes = planner.plan(&transient_desired, ScopedFilter::None).await?;

        if !changes.is_empty() {
            let engine = self.create_sync_engine().await;
            engine.sync(changes).await?;
        }

        Ok(())
    }

    /// Logic for Feature 6 cleanup. Removes all packages tied to the session ID.
    async fn cleanup_transient_env(&self, session_id: &str) -> Result<()> {
        let to_remove = {
            let state = self.state.lock().await;
            state.get_transient_packages(session_id)
        };

        if to_remove.is_empty() { return Ok(()); }

        let mut graph = petgraph::stable_graph::StableDiGraph::new();
        for (backend, name) in to_remove {
            graph.add_node(crate::core::GraphAction::Remove { name, backend });
        }

        let changes = crate::app::sync::SyncChanges {
            graph,
            install_map: HashMap::new(),
            removal_tracker: std::collections::HashSet::new(),
        };

        let engine = self.create_sync_engine().await;
        engine.sync(changes).await?;

        Ok(())
    }

    /// Feature: Automatically enter a shell if 'linix.txt' manifest is found.
    pub async fn auto_shell(&self) -> Result<()> {
        let local_config = Path::new("linix.txt");
        if tokio::fs::try_exists(local_config).await.unwrap_or(false) {
            info!("GhostShell: Found local configuration 'linix.txt'.");
            let content = tokio::fs::read_to_string(local_config).await?;
            let pkgs: Vec<String> = content.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            
            // Resolves E0425: Fixed variable naming mismatch
            if !pkgs.is_empty() {
                return self.enter(&pkgs).await;
            }
        }
        Ok(())
    }

    /// Factory for context-aware sync engine.
    /// Resolves E0061: Passes diagnostics as the 10th argument.
    async fn create_sync_engine(&self) -> SyncEngine<'_> {
        SyncEngine::new(
            &self.config,
            self.registry.clone(),
            self.executor.duplicate(),
            self.metrics.clone(),
            self.progress.clone(),
            self.hooks.clone(),
            self.snapshot_manager.clone(),
            self.journal.clone(),
            self.state.clone(),
            self.diagnostics.clone(), // Correctly providing the 10th argument
        ).await
    }
}