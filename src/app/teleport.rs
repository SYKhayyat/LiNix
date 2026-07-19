use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::vocab::Vocab;
use crate::backends::BackendRegistry;
use crate::config::parser::HostFacts;
use crate::config::Config;
use crate::core::{
    Error, GraphAction, Journal, PackageSpec, Result, StateRegistry, Transaction,
};
use crate::model::{active_module_files, Editor, Landing};
use petgraph::stable_graph::StableDiGraph;
use std::collections::HashMap;
use std::sync::Arc;
pub use tokio::sync::Mutex;
use tracing::{debug, error, info, instrument, trace};

pub struct Teleporter {
    registry: Arc<BackendRegistry>,
    journal: Arc<Mutex<Journal>>,
    state: Arc<Mutex<StateRegistry>>,
    config: Arc<Config>,
    diagnostics: Arc<FailureDiagnosticEngine>,
}

impl Teleporter {
    pub fn new(
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        state: Arc<Mutex<StateRegistry>>,
        diagnostics: Arc<FailureDiagnosticEngine>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            registry,
            journal,
            state,
            config,
            diagnostics,
        }
    }

    /// II.8: `teleport` edits the line and syncs. The old backend's line goes, the new
    /// one arrives — the same two edits you would have made by hand.
    async fn move_the_line(&self, package_name: &str, target_backend: &str) -> Result<()> {
        let priority = crate::app::sync::resolver::StateResolver::new(
            &self.config,
            self.registry.clone(),
            false,
        )
        .await
        .priority_for_host()
        .await?;
        let vocab = Vocab::new(&self.registry, &self.config, &priority);
        let layout = self.config.layout();
        let facts = HostFacts::current();

        let editor = Editor::new(&layout, &vocab).with_facts(facts.clone());
        let files = active_module_files(&layout, &vocab, &facts);
        let removed = editor
            .remove_from(&files, package_name)
            .map_err(Error::from)?;

        // Put the new line where the old one was, so a teleported package stays in the
        // module you chose to keep it in rather than migrating to `imperative`.
        let target = removed
            .first()
            .and_then(|e| e.file.file_stem())
            .and_then(|stem| crate::model::ModuleName::new(&stem.to_string_lossy()).ok())
            .map(crate::model::Target::Module)
            .unwrap_or_else(|| Landing::Imperative.target());
        let edit = editor
            .add(&target, &format!("{}:{}", target_backend, package_name))
            .map_err(Error::from)?;
        info!("{}", edit.describe("Moved"));
        Ok(())
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

                trace!("Teleporter: Aligning your files with the move...");
                self.move_the_line(package_name, target_backend_name).await?;

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
