use crate::App;
use crate::core::{Error, GhostMetadata, GraphAction, PackageSpec, Result, Transaction};
use crate::config::manifest::ManifestEngine;
use crate::utils::safe_data_dir;
use petgraph::stable_graph::StableDiGraph;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, debug};

/// The cross-backend transition engine.
/// Responsible for moving a package between different management backends.
pub struct Teleporter<'a> {
    app: &'a App,
    ghost_path: PathBuf,
    manifest_engine: ManifestEngine,
}

impl<'a> Teleporter<'a> {
    pub fn new(app: &'a App) -> Self {
        let ghost_path = safe_data_dir().join("ghosts.json");
        Self {
            app,
            ghost_path,
            manifest_engine: ManifestEngine::new(&app.config.groups_dir),
        }
    }

    /// Primary entry point: Transports a package to a new backend.
    pub async fn teleport(&self, package_name: &str, target_backend_name: &str) -> Result<()> {
        info!("Teleporter: Initiating transition of '{}' to backend '{}'...", package_name, target_backend_name);

        let mut source_backend = None;
        for backend in self.app.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(Some(pkg)) = queryable.info(package_name).await {
                    source_backend = Some((backend.clone(), pkg));
                    break;
                }
            }
        }

        let (src_backend, src_pkg) = source_backend
            .ok_or_else(|| Error::PackageNotFound(format!("Cannot teleport '{}': Not found in any active backend.", package_name)))?;

        let src_backend_name = src_backend.name();
        if src_backend_name == target_backend_name {
            info!("Teleporter: Package is already managed by '{}'. No action needed.", target_backend_name);
            return Ok(());
        }

        // 1. Archive the existing state metadata
        self.save_ghost(package_name, src_backend_name, &src_pkg.properties, target_backend_name).await?;

        // 2. Build the Atomic Transition Graph
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

        info!("Teleporter: Executing cross-backend transformation...");
        let mut tx = Transaction::new(graph, self.app.registry.clone(), self.app.journal.clone());
        tx.execute().await?;

        // 3. Declarative Manifest Alignment
        debug!("Teleporter: Updating declarative manifests...");
        let _ = self.manifest_engine.delete_package(package_name);
        
        let new_spec_str = format!("{}:{}", target_backend_name, package_name);
        self.manifest_engine.add_to_local(&new_spec_str)?;

        info!("Teleporter: Successfully migrated '{}' from {} to {}.", package_name, src_backend_name, target_backend_name);
        Ok(())
    }

    async fn save_ghost(&self, name: &str, backend: &str, props: &HashMap<String, String>, target: &str) -> Result<()> {
        let mut ghosts: HashMap<String, GhostMetadata> = if self.ghost_path.exists() {
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
        crate::utils::file::atomic_write(&self.ghost_path, &data)?;
        debug!("GhostTracker: Metadata archived for {}", name);
        Ok(())
    }

    pub async fn get_ghost(&self, name: &str) -> Result<Option<GhostMetadata>> {
        if !self.ghost_path.exists() {
            return Ok(None);
        }
        let data = tokio::fs::read_to_string(&self.ghost_path).await.map_err(Error::from)?;
        let ghosts: HashMap<String, GhostMetadata> = serde_json::from_str(&data).unwrap_or_default();
        Ok(ghosts.get(name).cloned())
    }

    pub async fn list_ghosts(&self) -> Result<Vec<(String, String)>> {
        if !self.ghost_path.exists() {
            return Ok(vec![]);
        }
        let data = tokio::fs::read_to_string(&self.ghost_path).await.map_err(Error::from)?;
        let ghosts: HashMap<String, GhostMetadata> = serde_json::from_str(&data).unwrap_or_default();
        Ok(ghosts.into_iter().map(|(name, meta)| (name, meta.backend)).collect())
    }

    pub async fn restore(&self, name: &str) -> Result<()> {
        let ghost = self.get_ghost(name).await?
            .ok_or_else(|| Error::PackageNotFound(format!("No ghost metadata for {}", name)))?;
        
        info!("Teleporter: Restoring package '{}' from ghost (original backend: {})", name, ghost.backend);
        let spec = PackageSpec {
            name: name.to_string(),
            backend: ghost.backend.clone(),
            options: ghost.options.clone(),
            requires: ghost.requires.clone(),
        };
        
        let backend_caps = self.app.registry.get(&spec.backend)
            .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;
            
        if let Some(installer) = backend_caps.as_installable() {
            installer.install(&[spec], true).await?;
            let mut state = self.app.state.lock().await;
            state.add(&ghost.backend, name, None, ghost.options.clone());
            state.save()?;
            self.remove_ghost(name).await?;
            info!("Teleporter: Successfully restored '{}'", name);
            Ok(())
        } else {
            Err(Error::Other(format!("Backend {} does not support installation", ghost.backend)))
        }
    }

    async fn remove_ghost(&self, name: &str) -> Result<()> {
        let mut ghosts: HashMap<String, GhostMetadata> = if self.ghost_path.exists() {
            let data = tokio::fs::read_to_string(&self.ghost_path).await.map_err(Error::from)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            return Ok(());
        };
        ghosts.remove(name);
        let data = serde_json::to_string_pretty(&ghosts).map_err(Error::from)?;
        crate::utils::file::atomic_write(&self.ghost_path, &data)?;
        Ok(())
    }
}