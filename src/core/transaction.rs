use crate::core::{manager::Backend, Result, Error, PackageSpec, Journal, ActionStatus};
use crate::core::journal::JournalAction;
use crate::backends::BackendRegistry;
use crate::app::bridge::DependencyBridge;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing::{info, warn, error, debug};
use petgraph::stable_graph::StableDiGraph;
use petgraph::graph::NodeIndex;
use petgraph::Direction;

/// Represents a discrete action within the Directed Acyclic Graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum GraphAction {
    Install(PackageSpec),
    Remove { name: String, backend: String },
}

/// The result of an individual node execution, sent back to the orchestrator.
struct TaskResult {
    node_index: NodeIndex,
    backend_name: String,
    package_name: String,
    /// Properties discovered during execution (e.g., local store paths) to be staged.
    properties: HashMap<String, String>,
    result: Result<()>,
}

/// The High-Performance Mission-Critical Execution Engine.
/// Hardened for Version 3.5.0 with Atomic State Staging.
/// 
/// Changes from v3.4.0:
/// 1. Uses the upgraded Journal (WAL) to store full PackageSpecs.
/// 2. Captures backend properties during install and stages them in the WAL.
/// 3. Returns a set of "Committed Tasks" to the SyncEngine for atomic registry updates.
pub struct Transaction {
    pub graph: StableDiGraph<GraphAction, ()>,
    registry: Arc<BackendRegistry>,
    journal: Arc<Mutex<Journal>>,
    completed_indices: HashSet<NodeIndex>,
    history: Vec<NodeIndex>,
}

impl Transaction {
    pub fn new(
        graph: StableDiGraph<GraphAction, ()>, 
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>
    ) -> Self {
        Self {
            graph,
            registry,
            journal,
            completed_indices: HashSet::new(),
            history: Vec::new(),
        }
    }

    /// Executes the system transformation.
    pub async fn execute(&mut self) -> Result<()> {
        let total_nodes = self.graph.node_count();
        info!("Transaction: Commencing parallel execution of {} nodes.", total_nodes);

        let mut in_progress = HashSet::new();
        let mut worker_pool = JoinSet::new();
        let bridge = DependencyBridge::new();

        while self.completed_indices.len() < total_nodes {
            // 1. Identify "Ready" nodes (Dependencies satisfied)
            let ready_nodes: Vec<NodeIndex> = self.graph.node_indices()
                .filter(|&idx| {
                    !self.completed_indices.contains(&idx) &&
                    !in_progress.contains(&idx) &&
                    self.graph.neighbors_directed(idx, Direction::Incoming)
                        .all(|dep| self.completed_indices.contains(&dep))
                })
                .collect();

            // 2. Dispatch tasks to the worker pool
            for idx in ready_nodes {
                in_progress.insert(idx);
                let action = self.graph[idx].clone();
                let registry = self.registry.clone();
                let journal = self.journal.clone();

                worker_pool.spawn(async move {
                    let (p_name, b_name, j_action) = match action {
                        GraphAction::Install(ref s) => (s.name.clone(), s.backend.clone(), JournalAction::Install(s.clone())),
                        GraphAction::Remove { ref name, ref backend } => (name.clone(), backend.clone(), JournalAction::Remove { name: name.clone(), backend: backend.clone() }),
                    };

                    // v3.5.0 Hardening: Record full action spec in WAL
                    let journal_id = {
                        let mut j = journal.lock().await;
                        match j.record_start(j_action) {
                            Ok(id) => id,
                            Err(e) => return TaskResult { node_index: idx, backend_name: b_name, package_name: p_name, properties: HashMap::new(), result: Err(e) },
                        }
                    };

                    let mut properties = HashMap::new();
                    let res = match &action {
                        GraphAction::Install(spec) => {
                            if let Some(backend) = registry.get(&spec.backend) {
                                if let Some(handler) = backend.as_installable() {
                                    let install_res = handler.install(&[spec.clone()], true).await;
                                    
                                    // If install succeeded, query properties (like local paths) to stage in WAL
                                    if install_res.is_ok() {
                                        if let Some(queryable) = backend.as_queryable() {
                                            if let Ok(Some(info)) = queryable.info(&spec.name).await {
                                                properties = info.properties;
                                            }
                                        }
                                    }
                                    install_res
                                } else {
                                    Err(Error::Transaction(format!("Backend {} cannot install", spec.backend)))
                                }
                            } else {
                                Err(Error::BackendNotFound(spec.backend.clone()))
                            }
                        }
                        GraphAction::Remove { name, backend: b_name } => {
                            if let Some(b) = registry.get(b_name) {
                                if let Some(handler) = b.as_installable() {
                                    handler.remove(&[name.clone()], true).await
                                } else {
                                    Err(Error::Transaction(format!("Backend {} cannot remove", b_name)))
                                }
                            } else {
                                Err(Error::BackendNotFound(b_name.clone()))
                            }
                        }
                    };

                    // Commit staged properties to the WAL Journal
                    let mut j = journal.lock().await;
                    if res.is_ok() {
                        let _ = j.record_success(&journal_id, properties.clone());
                    } else {
                        let _ = j.record_failure(&journal_id, &format!("{:?}", res.as_ref().err()));
                    }

                    TaskResult { 
                        node_index: idx, 
                        backend_name: b_name, 
                        package_name: p_name, 
                        properties, 
                        result: res 
                    }
                });
            }

            // 3. Monitor completions
            if let Some(finished_task) = worker_pool.join_next().await {
                let task_data = finished_task.map_err(|e| Error::Other(format!("Worker Panic: {}", e)))?;
                
                match task_data.result {
                    Ok(_) => {
                        in_progress.remove(&task_data.node_index);
                        self.completed_indices.insert(task_data.node_index);
                        self.history.push(task_data.node_index);
                        debug!("Node confirmed: {}:{}", task_data.backend_name, task_data.package_name);
                    }
                    Err(e) => {
                        error!("Failure at node {}:{}: {}", task_data.backend_name, task_data.package_name, e);
                        
                        if let Error::CommandFailed(ref msg) = e {
                            bridge.print_suggestions(msg, &task_data.backend_name);
                        }

                        warn!("Transaction: Initiating rollback for system safety.");
                        worker_pool.abort_all();
                        self.rollback().await;
                        return Err(e);
                    }
                }
            }

            if worker_pool.is_empty() && self.completed_indices.len() < total_nodes {
                return Err(Error::Transaction("Deadlock: Graph is stuck with unresolved nodes.".into()));
            }
        }

        info!("Transaction: All nodes applied successfully.");
        Ok(())
    }

    /// Reverts successfully applied nodes in reverse order.
    async fn rollback(&mut self) {
        for &idx in self.history.iter().rev() {
            let action = &self.graph[idx];
            let _ = match action {
                GraphAction::Install(spec) => {
                    if let Some(backend) = self.registry.get(&spec.backend) {
                        if let Some(handler) = backend.as_installable() {
                            handler.remove(&[spec.name.clone()], true).await
                        } else { Ok(()) }
                    } else { Ok(()) }
                }
                GraphAction::Remove { name, backend } => {
                    if let Some(b) = self.registry.get(backend) {
                        if let Some(handler) = b.as_installable() {
                            let spec = PackageSpec {
                                name: name.clone(),
                                backend: backend.clone(),
                                options: HashMap::new(),
                                requires: vec![],
                            };
                            handler.install(&[spec], true).await
                        } else { Ok(()) }
                    } else { Ok(()) }
                }
            };
        }
    }
}