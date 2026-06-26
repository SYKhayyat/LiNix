use crate::core::{
    Error, GhostMetadata, GraphAction, PackageSpec, Result, 
    Transaction, StateRegistry, Journal
};
use crate::backends::BackendRegistry;
use crate::app::diagnostics::FailureDiagnosticEngine; // Modernized: DI Import
use crate::config::manifest::ManifestEngine;
use crate::utils::safe_data_dir;
use petgraph::stable_graph::StableDiGraph;
use std::collections::HashMap;
use std::path::{PathBuf, Path};
use std::sync::Arc;
pub use tokio::sync::Mutex;
use tracing::{info, debug, error, instrument, trace};

/// The cross-backend transition engine.
/// 
/// The Teleporter is responsible for moving a package's ownership from one 
/// backend to another (e.g., migrating 'curl' from 'apt' to 'snap').
/// 
/// Modernized v3.6.0: Utilizes Dependency Injection for diagnostics and 
/// follows the exhaustive 6-argument state registration model.
pub struct Teleporter {
    /// Registry for capability discovery.
    registry: Arc<BackendRegistry>,
    /// Write-Ahead Log for transaction safety.
    journal: Arc<Mutex<Journal>>,
    /// Mission-critical system state.
    state: Arc<Mutex<StateRegistry>>,
    /// Path to the ghost metadata storage.
    ghost_path: PathBuf,
    /// Engine for modifying declarative .txt files.
    manifest_engine: ManifestEngine,
    /// Modernized v3.6.0: Injected diagnostic engine.
    diagnostics: Arc<FailureDiagnosticEngine>,
}

impl Teleporter {
    /// Initializes a new Teleporter with explicit dependency injection.
    pub fn new(
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        state: Arc<Mutex<StateRegistry>>,
        diagnostics: Arc<FailureDiagnosticEngine>, // Added 4th DI component
        groups_dir: &Path,
    ) -> Self {
        let ghost_path = safe_data_dir().join("ghosts.json");
        Self {
            registry,
            journal,
            state,
            ghost_path,
            manifest_engine: ManifestEngine::new(groups_dir),
            diagnostics,
        }
    }

    /// Primary entry point: Transports a package to a new backend.
    /// 
    /// This method performs an atomic cross-backend closure:
    /// 1. Saves metadata as a "Ghost".
    /// 2. Executes a DAG (Remove Source -> Install Target).
    /// 3. Re-acquires ownership in the State Registry.
    #[instrument(skip(self))]
    pub async fn teleport(&self, package_name: &str, target_backend_name: &str) -> Result<()> {
        info!("Teleporter: Transitioning '{}' to backend '{}'...", package_name, target_backend_name);

        // --- 1. DISCOVERY ---
        let mut source_backend = None;
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(Some(pkg)) = queryable.info(package_name).await {
                    source_backend = Some((backend.clone(), pkg));
                    break;
                }
            }
        }

        // Pass just the package name — `Error::PackageNotFound` already formats the
        // "Package '<name>' was not found" message (passing a full sentence here
        // produced a double-wrapped error).
        let (src_backend, src_pkg) = source_backend
            .ok_or_else(|| Error::PackageNotFound(package_name.to_string()))?;

        let src_backend_name = src_backend.name();
        if src_backend_name == target_backend_name {
            info!("Teleporter: Package is already managed by '{}'.", target_backend_name);
            return Ok(());
        }

        // --- 2. ARCHIVAL ---
        self.save_ghost(package_name, src_backend_name, &src_pkg.properties, target_backend_name).await?;

        // --- 3. TRANSACTION ---
        let mut graph = StableDiGraph::new();
        
        let remove_node = graph.add_node(GraphAction::Remove {
            name: package_name.to_string(),
            backend: src_backend_name.to_string(),
        });
        
        let target_spec = PackageSpec {
            name: package_name.to_string(),
            backend: target_backend_name.to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        let install_node = graph.add_node(GraphAction::Install(target_spec.clone()));
        graph.add_edge(remove_node, install_node, ());

        info!("Teleporter: Executing atomic transition transaction...");
        
        // Resolves E0061: Passes diagnostics as the 4th argument
        let mut tx = Transaction::new(
            graph, 
            self.registry.clone(), 
            self.journal.clone(),
            self.diagnostics.clone()
        );
        
        let result = tx.execute().await;

        // --- 4. COMPLETION ---
        match result {
            Ok(_) => {
                debug!("Teleporter: Transaction successful. Aligning state registry.");
                
                {
                    let mut state = self.state.lock().await;
                    state.remove(src_backend_name, package_name);
                    
                    // Resolves E0061: Supplies all 6 arguments to modernized state.add
                    state.add(
                        target_backend_name, 
                        package_name, 
                        src_pkg.version.clone(), 
                        HashMap::new(), 
                        Some("teleport".into()), 
                        false // Migration is permanent
                    );
                    
                    let state_clone = state.clone();
                    tokio::task::spawn_blocking(move || state_clone.save())
                        .await
                        .map_err(|e| Error::Other(format!("State save join panic: {}", e)))??;
                }

                trace!("Teleporter: Aligning declarative manifests...");
                let _ = self.manifest_engine.delete_package(package_name).await;
                let new_spec_str = format!("{}:{}", target_backend_name, package_name);
                self.manifest_engine.add_to_local(&new_spec_str).await?;

                info!("Teleporter: Success. '{}' moved to {}.", package_name, target_backend_name);
                Ok(())
            }
            Err(e) => {
                error!("Teleporter: Transition FAILED: {}. Ghost metadata preserved.", e);
                Err(e)
            }
        }
    }

    /// Internal logic for ghost metadata archival.
    async fn save_ghost(&self, name: &str, backend: &str, props: &HashMap<String, String>, target: &str) -> Result<()> {
        let mut ghosts: HashMap<String, GhostMetadata> = if tokio::fs::try_exists(&self.ghost_path).await.unwrap_or(false) {
            let data = tokio::fs::read_to_string(&self.ghost_path).await.map_err(Error::from)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            HashMap::new()
        };
        
        ghosts.insert(name.to_string(), GhostMetadata {
            backend: backend.to_string(),
            options: HashMap::new(),
            properties: props.clone(),
            requires: vec![],
            removed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            teleported_to: Some(target.to_string()),
        });
        
        let data = serde_json::to_string_pretty(&ghosts).map_err(Error::from)?;
        let path = self.ghost_path.clone();
        
        tokio::task::spawn_blocking(move || {
            crate::utils::file::atomic_write(&path, &data)
        }).await.map_err(|e| Error::Other(e.to_string()))??;
        
        debug!("Teleporter: Snapshot metadata archived for '{}'.", name);
        Ok(())
    }

    /// Restores a package from a ghost record.
    pub async fn restore(&self, name: &str) -> Result<()> {
        if !tokio::fs::try_exists(&self.ghost_path).await.unwrap_or(false) {
            return Err(Error::PackageNotFound("No ghost records exist.".into()));
        }
        
        let data = tokio::fs::read_to_string(&self.ghost_path).await.map_err(Error::from)?;
        let mut ghosts: HashMap<String, GhostMetadata> = serde_json::from_str(&data).unwrap_or_default();
        
        let ghost = ghosts.get(name)
            .ok_or_else(|| Error::PackageNotFound(format!("No ghost record for '{}'.", name)))?
            .clone();
        
        info!("Teleporter: Restoring '{}' from archival ghost record.", name);
        
        let spec = PackageSpec {
            name: name.to_string(),
            backend: ghost.backend.clone(),
            options: ghost.options.clone(),
            requires: ghost.requires.clone(),
        };
        
        let b_cap = self.registry.get(&spec.backend)
            .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;
            
        if let Some(installer) = b_cap.as_installable() {
            installer.install(&[spec], b_cap.needs_root()).await?;
            
            let mut state = self.state.lock().await;
            
            // Resolves E0061: Supplies 6 arguments
            state.add(
                &ghost.backend, 
                name, 
                None, 
                ghost.options.clone(), 
                Some("restore".into()), 
                false
            );
            
            let state_clone = state.clone();
            tokio::task::spawn_blocking(move || state_clone.save())
                .await
                .map_err(|e| Error::Other(format!("State save panic: {}", e)))??;

            ghosts.remove(name);
            let updated = serde_json::to_string_pretty(&ghosts).map_err(Error::from)?;
            let path = self.ghost_path.clone();
            tokio::task::spawn_blocking(move || {
                crate::utils::file::atomic_write(&path, &updated)
            }).await.map_err(|e| Error::Other(e.to_string()))??;

            info!("Teleporter: Restoration complete for '{}'.", name);
            Ok(())
        } else {
            Err(Error::Transaction(format!("Backend '{}' is not installable.", ghost.backend)))
        }
    }
}