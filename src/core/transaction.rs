use crate::core::{Result, Error, PackageSpec, Journal};
use crate::core::journal::{JournalAction, ActionStatus};
use crate::backends::BackendRegistry;
use crate::app::bridge::DependencyBridge;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn, error, debug};
use petgraph::stable_graph::StableDiGraph;
use petgraph::graph::NodeIndex;
use petgraph::Direction;

/// Configuration for transaction execution with timeout and retry support.
#[derive(Debug, Clone)]
pub struct TransactionConfig {
    pub max_concurrent: usize,
    pub node_timeout: Duration,
    pub total_timeout: Duration,
    pub max_retries: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub auto_rollback: bool,
}

impl Default for TransactionConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            node_timeout: Duration::from_secs(300),
            total_timeout: Duration::from_secs(3600),
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            auto_rollback: true,
        }
    }
}

/// Represents a discrete action within the Directed Acyclic Graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum GraphAction {
    Install(PackageSpec),
    Remove { name: String, backend: String },
}

/// The result of an individual node execution with retry metadata.
#[derive(Debug)]
struct TaskResult {
    node_index: NodeIndex,
    backend_name: String,
    package_name: String,
    properties: HashMap<String, String>,
    attempt: u32,
    duration: Duration,
    result: Result<()>,
}

/// The High-Performance Mission-Critical Execution Engine.
pub struct Transaction {
    pub graph: StableDiGraph<GraphAction, ()>,
    registry: Arc<BackendRegistry>,
    journal: Arc<Mutex<Journal>>,
    config: TransactionConfig,
    completed_indices: HashSet<NodeIndex>,
    history: Vec<NodeIndex>,
    cancellation_token: CancellationToken,
    start_time: Option<std::time::Instant>,
}

impl Transaction {
    pub fn new(
        graph: StableDiGraph<GraphAction, ()>, 
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>
    ) -> Self {
        Self::with_config(graph, registry, journal, TransactionConfig::default())
    }
    
    pub fn with_config(
        graph: StableDiGraph<GraphAction, ()>, 
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        config: TransactionConfig,
    ) -> Self {
        Self {
            graph,
            registry,
            journal,
            config,
            completed_indices: HashSet::new(),
            history: Vec::new(),
            cancellation_token: CancellationToken::new(),
            start_time: None,
        }
    }

    pub async fn execute(&mut self) -> Result<()> {
        self.start_time = Some(std::time::Instant::now());
        let total_timeout = self.config.total_timeout;
        
        match tokio::time::timeout(total_timeout, self.execute_internal()).await {
            Ok(res) => res,
            Err(_) => {
                error!("Transaction: Total timeout of {:?} exceeded.", total_timeout);
                self.cancellation_token.cancel();
                if self.config.auto_rollback {
                    self.rollback().await;
                }
                Err(Error::Transaction(format!("Transaction timed out after {:?}", total_timeout)))
            }
        }
    }
    
    async fn execute_internal(&mut self) -> Result<()> {
        let total_nodes = self.graph.node_count();
        let mut in_progress = HashSet::new();
        let mut worker_pool = JoinSet::new();
        let bridge = DependencyBridge::new();
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent));

        while self.completed_indices.len() < total_nodes {
            if self.cancellation_token.is_cancelled() {
                worker_pool.abort_all();
                if self.config.auto_rollback { self.rollback().await; }
                return Err(Error::Transaction("Transaction cancelled".into()));
            }
            
            let ready_nodes: Vec<NodeIndex> = self.graph.node_indices()
                .filter(|&idx| {
                    !self.completed_indices.contains(&idx) &&
                    !in_progress.contains(&idx) &&
                    self.graph.neighbors_directed(idx, Direction::Incoming)
                        .all(|dep| self.completed_indices.contains(&dep))
                })
                .collect();

            for idx in ready_nodes {
                let _permit = semaphore.clone().acquire_owned().await.unwrap();
                in_progress.insert(idx);
                
                let action = self.graph[idx].clone();
                let registry = self.registry.clone();
                let journal = self.journal.clone();
                let cancel_token = self.cancellation_token.clone();
                let config = self.config.clone();

                worker_pool.spawn(async move {
                    Self::execute_node_with_retry(
                        action, registry, journal, config, cancel_token, idx
                    ).await
                });
            }

            if let Some(finished_task) = worker_pool.join_next().await {
                let task_data = finished_task.map_err(|e| Error::Transaction(format!("Worker Panic: {}", e)))?;
                
                match task_data.result {
                    Ok(_) => {
                        in_progress.remove(&task_data.node_index);
                        self.completed_indices.insert(task_data.node_index);
                        self.history.push(task_data.node_index);
                        
                        // WIRING: Use captured telemetry data in final logs
                        info!(
                            "Node {}:{} completed in {:?} (Attempts: {}, Properties: {})", 
                            task_data.backend_name, 
                            task_data.package_name,
                            task_data.duration,
                            task_data.attempt,
                            task_data.properties.len()
                        );
                    }
                    Err(e) => {
                        error!("Node {}:{} failed after {:?} (Attempt {}): {}", 
                               task_data.backend_name, task_data.package_name, task_data.duration, task_data.attempt, e);
                        
                        if let Error::CommandFailed(ref msg) = e {
                            bridge.print_suggestions(msg, &task_data.backend_name);
                        }
                        if self.config.auto_rollback {
                            worker_pool.abort_all();
                            self.rollback().await;
                        }
                        return Err(e);
                    }
                }
            }
        }
        Ok(())
    }
    
    async fn execute_node_with_retry(
        action: GraphAction,
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        config: TransactionConfig,
        cancel_token: CancellationToken,
        node_index: NodeIndex,
    ) -> TaskResult {
        let (p_name, b_name, j_action) = match &action {
            GraphAction::Install(s) => (s.name.clone(), s.backend.clone(), JournalAction::Install(s.clone())),
            GraphAction::Remove { name, backend } => (name.clone(), backend.clone(), JournalAction::Remove { name: name.clone(), backend: backend.clone() }),
        };

        let mut attempt = 0;
        let mut last_error = None;
        let start = std::time::Instant::now();
        
        while attempt <= config.max_retries {
            attempt += 1;
            
            if cancel_token.is_cancelled() {
                return TaskResult { node_index, backend_name: b_name, package_name: p_name, properties: HashMap::new(), attempt, duration: start.elapsed(), result: Err(Error::Cancelled) };
            }
            
            if attempt > 1 {
                let backoff = std::cmp::min(config.initial_backoff * (1 << (attempt - 2)), config.max_backoff);
                tokio::time::sleep(backoff).await;
            }
            
            let journal_id = {
                let mut j = journal.lock().await;
                j.record_start(j_action.clone()).unwrap()
            };

            let result = tokio::time::timeout(config.node_timeout, async {
                match &action {
                    GraphAction::Install(spec) => {
                        let backend = registry.get(&spec.backend).ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;
                        if let Some(handler) = backend.as_installable() {
                            handler.install(&[spec.clone()], true).await?;
                            let props = if let Some(q) = backend.as_queryable() {
                                q.info(&spec.name).await?.map(|p| p.properties).unwrap_or_default()
                            } else { HashMap::new() };
                            Ok(props)
                        } else { Err(Error::Transaction("Backend not installable".into())) }
                    }
                    GraphAction::Remove { name, backend: b_name } => {
                        let backend = registry.get(b_name).ok_or_else(|| Error::BackendNotFound(b_name.clone()))?;
                        if let Some(handler) = backend.as_installable() {
                            handler.remove(&[name.clone()], true).await?;
                            Ok(HashMap::new())
                        } else { Err(Error::Transaction("Backend not removable".into())) }
                    }
                }
            }).await;
            
            match result {
                Ok(Ok(props)) => {
                    let mut j = journal.lock().await;
                    let _ = j.record_success(&journal_id, props.clone());
                    return TaskResult { node_index, backend_name: b_name, package_name: p_name, properties: props, attempt, duration: start.elapsed(), result: Ok(()) };
                }
                Ok(Err(e)) => {
                    last_error = Some(e);
                    let mut j = journal.lock().await;
                    let _ = j.record_failure(&journal_id, &format!("{:?}", last_error));
                }
                Err(_) => {
                    last_error = Some(Error::Transaction("Node timed out".into()));
                    let mut j = journal.lock().await;
                    let _ = j.record_failure(&journal_id, "Timeout");
                }
            }
        }
        
        TaskResult { node_index, backend_name: b_name, package_name: p_name, properties: HashMap::new(), attempt, duration: start.elapsed(), result: Err(last_error.unwrap()) }
    }

    async fn rollback(&mut self) {
        info!("Transaction: Rolling back {} operations.", self.history.len());
        for &idx in self.history.iter().rev() {
            let action = &self.graph[idx];
            match action {
                GraphAction::Install(spec) => {
                    if let Some(b) = self.registry.get(&spec.backend) {
                        if let Some(h) = b.as_installable() { let _ = h.remove(&[spec.name.clone()], true).await; }
                    }
                }
                GraphAction::Remove { name, backend } => {
                    if let Some(b) = self.registry.get(backend) {
                        if let Some(h) = b.as_installable() {
                            let spec = PackageSpec { name: name.clone(), backend: backend.clone(), options: HashMap::new(), requires: vec![] };
                            let _ = h.install(&[spec], true).await;
                        }
                    }
                }
            }
        }
    }
}