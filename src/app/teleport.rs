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

pub struct Teleporter {
    registry: Arc<BackendRegistry>,
    journal: Arc<Mutex<Journal>>,
    state: Arc<Mutex<StateRegistry>>,
    manifest_engine: ManifestEngine,
    diagnostics: Arc<FailureDiagnosticEngine>,
}

impl Teleporter {
    pub fn new(
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        state: Arc<Mutex<StateRegistry>>,
        diagnostics: Arc<FailureDiagnosticEngine>,
        groups_dir: &Path,
    ) -> Self {
        Self {
            registry,
            journal,
            state,
            manifest_engine: ManifestEngine::new(groups_dir),
            diagnostics,
        }
    }

    #[instrument(skip(self))]
    pub async fn teleport(&self, package_name: &str, target_backend_name: &str) -> Result<()> {
        info!(
            "Teleporter: Transitioning '{}' to backend '{}'...",
            package_name, target_backend_name
        );

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
            present: true,
        };
        let install_node = graph.add_node(GraphAction::Install(target_spec.clone()));
        graph.add_edge(remove_node, install_node, ());

        info!("Teleporter: Executing atomic transition transaction...");

        let mut tx = Transaction::new(
            graph,
            self.registry.clone(),
            self.journal.clone(),
            self.diagnostics.clone(),
        );

        let result = tx.execute().await;

        match result {
            Ok(_) => {
                debug!("Teleporter: Transaction successful. Aligning state registry.");

                {
                    let mut state = self.state.lock().await;
                    state.remove(src_backend_name, package_name);

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
