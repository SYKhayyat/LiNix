use crate::app::diagnostics::FailureDiagnosticEngine; // Modernized: DI Import
use crate::backends::BackendRegistry;
use crate::config::manifest::ManifestEngine;
use crate::core::{
    Error, GraphAction, Journal, PackageSpec, Result, StateRegistry, Transaction,
};
use petgraph::stable_graph::StableDiGraph;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
pub use tokio::sync::Mutex;
use tracing::{debug, error, info, instrument, trace};

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
        global_dir: &Path,
        wish_dirs: Vec<std::path::PathBuf>,
    ) -> Self {
        Self {
            registry,
            journal,
            state,
            manifest_engine: ManifestEngine::new(global_dir, wish_dirs),
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
        info!(
            "Teleporter: Transitioning '{}' to backend '{}'...",
            package_name, target_backend_name
        );

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
        let (src_backend, src_pkg) =
            source_backend.ok_or_else(|| Error::PackageNotFound(package_name.to_string()))?;

        let src_backend_name = src_backend.name();
        if src_backend_name == target_backend_name {
            info!(
                "Teleporter: Package is already managed by '{}'.",
                target_backend_name
            );
            return Ok(());
        }

        // --- 2. TRANSACTION ---
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
            self.diagnostics.clone(),
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
                        false, // Migration is permanent
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

                info!(
                    "Teleporter: Success. '{}' moved to {}.",
                    package_name, target_backend_name
                );
                Ok(())
            }
            Err(e) => {
                error!(
                    "Teleporter: Transition FAILED: {}. Ghost metadata preserved.",
                    e
                );
                Err(e)
            }
        }
    }

}
