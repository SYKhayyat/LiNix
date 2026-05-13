use crate::App;
use crate::core::{Result, Error, PackageSpec, GraphAction, Transaction};
use crate::config::manifest::ManifestEngine;
use petgraph::stable_graph::StableDiGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn, debug};

/// Represents preserved metadata for a package that has been removed or teleported.
/// Fulfills Roadmap Point 14.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostMetadata {
    pub backend: String,
    pub options: HashMap<String, String>,
    pub properties: HashMap<String, String>,
    pub requires: Vec<String>,
    pub teleported_to: Option<String>,
}

/// Orchestrates the movement of packages between different backends.
/// Hardened for Version 3.4.0 with Surgical Teleportation (Point 7).
/// 
/// It utilizes the ManifestEngine to ensure that when a package is teleported 
/// (e.g. from 'apt' to 'cargo'), its definition is removed from its original 
/// declarative file and moved into the primary 'local.txt' manifest.
pub struct Teleporter<'a> {
    app: &'a App,
    ghost_path: PathBuf,
    manifest_engine: ManifestEngine,
}

impl<'a> Teleporter<'a> {
    pub fn new(app: &'a App) -> Self {
        let ghost_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("ghosts.json");
        
        Self { 
            app, 
            ghost_path,
            manifest_engine: ManifestEngine::new(&app.config.groups_dir),
        }
    }

    /// Moves a package from its current backend to a new one.
    /// Example: teleport("ripgrep", "cargo")
    pub async fn teleport(&self, package_name: &str, target_backend_name: &str) -> Result<()> {
        info!("Teleporter: Initiating transition of '{}' to backend '{}'...", package_name, target_backend_name);

        // 1. Locate the current owner of the package and its metadata
        let mut source_backend = None;
        for backend in self.app.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(Some(pkg)) = queryable.info(package_name).await {
                    source_backend = Some((backend.clone(), pkg));
                    break;
                }
            }
        }

        let (src_backend, src_pkg) = source_backend.ok_or_else(|| {
            Error::PackageNotFound(format!("Cannot teleport '{}': Not found in any active backend.", package_name))
        })?;

        let src_backend_name = src_backend.name();
        if src_backend_name == target_backend_name {
            info!("Teleporter: Package is already managed by '{}'. No action needed.", target_backend_name);
            return Ok(());
        }

        // 2. Capture "Ghost" metadata before removal (Point 14)
        self.save_ghost(package_name, src_backend_name, &src_pkg.properties, target_backend_name).await?;

        // 3. Construct the Transaction DAG
        let mut graph = StableDiGraph::new();
        
        // Node A: Removal from source
        let remove_node = graph.add_node(GraphAction::Remove {
            name: package_name.to_string(),
            backend: src_backend_name.to_string(),
        });

        // Node B: Installation in target
        let target_spec = PackageSpec {
            name: package_name.to_string(),
            backend: target_backend_name.to_string(),
            options: HashMap::new(), // Future: Implement capability to map options between backends
            requires: vec![],
        };
        let install_node = graph.add_node(GraphAction::Install(target_spec.clone()));

        // Dependency: Must remove before install to avoid file path collisions
        graph.add_edge(remove_node, install_node, ());

        // 4. Execution Phase
        info!("Teleporter: Executing cross-backend transformation...");
        let mut tx = Transaction::new(graph, self.app.registry.clone(), self.app.journal.clone());
        tx.execute().await?;

        // 5. Surgical Manifest Reflection (Version 3.4.0 Hardening)
        // We delete the package from its original source manifest (preserving comments)
        // and add the new specification to local.txt.
        debug!("Teleporter: Updating declarative manifests...");
        match self.manifest_engine.delete_package(package_name) {
            Ok(_) => debug!("Teleporter: Removed '{}' from original manifest file.", package_name),
            Err(e) => warn!("Teleporter: Could not find '{}' in manifest files for deletion (might be an unmanaged package). Error: {}", package_name, e),
        }

        let new_spec_str = format!("{}:{}", target_backend_name, package_name);
        self.manifest_engine.add_to_local(&new_spec_str)?;

        info!("Teleporter: Successfully migrated '{}' from {} to {}.", package_name, src_backend_name, target_backend_name);
        Ok(())
    }

    /// Internal logic to store ghost metadata in ghosts.json.
    async fn save_ghost(&self, name: &str, backend: &str, props: &HashMap<String, String>, target: &str) -> Result<()> {
        let mut ghosts: HashMap<String, GhostMetadata> = if self.ghost_path.exists() {
            let data = tokio::fs::read_to_string(&self.ghost_path).await?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            HashMap::new()
        };

        ghosts.insert(name.to_string(), GhostMetadata {
            backend: backend.to_string(),
            options: HashMap::new(), 
            properties: props.clone(),
            requires: vec![],
            teleported_to: Some(target.to_string()),
        });

        let data = serde_json::to_string_pretty(&ghosts).map_err(|e| Error::Other(e.to_string()))?;
        crate::utils::file::atomic_write(&self.ghost_path, &data)?;
        debug!("GhostTracker: Metadata archived for {}", name);
        Ok(())
    }

    /// Retrieves ghost metadata for a package if it exists.
    pub async fn get_ghost(&self, name: &str) -> Result<Option<GhostMetadata>> {
        if !self.ghost_path.exists() { return Ok(None); }
        let data = tokio::fs::read_to_string(&self.ghost_path).await?;
        let ghosts: HashMap<String, GhostMetadata> = serde_json::from_str(&data).unwrap_or_default();
        Ok(ghosts.get(name).cloned())
    }

    /// Lists all packages that have been teleported or removed.
    pub async fn list_ghosts(&self) -> Result<Vec<(String, String)>> {
        if !self.ghost_path.exists() { return Ok(vec![]); }
        let data = tokio::fs::read_to_string(&self.ghost_path).await?;
        let ghosts: HashMap<String, GhostMetadata> = serde_json::from_str(&data).unwrap_or_default();
        Ok(ghosts.into_iter().map(|(name, meta)| (name, meta.backend)).collect())
    }
}