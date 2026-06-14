use crate::core::{Result, Error, PackageSpec, Journal};
use crate::core::journal::JournalAction;
use crate::backends::BackendRegistry;
use crate::app::diagnostics::FailureDiagnosticEngine;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, debug, error, trace}; 
use petgraph::stable_graph::StableDiGraph;
use petgraph::graph::NodeIndex;
use petgraph::Direction;

/// Configuration for transaction execution profiles.
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
        Self::patient()
    }
}

impl TransactionConfig {
    /// High-Performance Profile: Optimized for local filesystem or high-speed cache 
    /// operations where network latency is not a factor.
    pub fn quick() -> Self {
        Self {
            max_concurrent: 8,
            node_timeout: Duration::from_secs(60),
            total_timeout: Duration::from_secs(600),
            max_retries: 1,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(2),
            auto_rollback: true,
        }
    }

    /// Resilient Profile: Optimized for mixed remote and system-level operations.
    pub fn patient() -> Self {
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

/// Represents a discrete modification unit within the DAG.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum GraphAction {
    Install(PackageSpec),
    Remove { name: String, backend: String },
}

/// Comprehensive telemetry for a completed (or failed) node operation.
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub node_index: NodeIndex,
    pub backend_name: String,
    pub package_name: String,
    pub properties: HashMap<String, String>,
    pub attempt: u32,
    pub duration: Duration,
    pub bytes_downloaded: u64,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub result: Result<()>,
}

/// The High-Performance Mission-Critical Execution Engine.
pub struct Transaction {
    pub graph: StableDiGraph<GraphAction, ()>,
    registry: Arc<BackendRegistry>,
    journal: Arc<Mutex<Journal>>,
    diagnostics: Arc<FailureDiagnosticEngine>,
    config: TransactionConfig,
    completed_indices: HashSet<NodeIndex>,
    history: Vec<NodeIndex>,
    cancellation_token: CancellationToken,
}

impl Transaction {
    /// Initializes a new Transaction with default configuration.
    pub fn new(
        graph: StableDiGraph<GraphAction, ()>, 
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        diagnostics: Arc<FailureDiagnosticEngine>,
    ) -> Self {
        Self::with_config(graph, registry, journal, diagnostics, TransactionConfig::default())
    }
    
    /// Initializes a new Transaction with a specific performance profile.
    pub fn with_config(
        graph: StableDiGraph<GraphAction, ()>, 
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        diagnostics: Arc<FailureDiagnosticEngine>,
        config: TransactionConfig,
    ) -> Self {
        Self {
            graph,
            registry,
            journal,
            diagnostics,
            config,
            completed_indices: HashSet::new(),
            history: Vec::new(),
            cancellation_token: CancellationToken::new(),
        }
    }

    /// Primary execution driver. Implements the global transaction timeout.
    pub async fn execute_with_telemetry(&mut self) -> Result<Vec<TaskResult>> {
        let total_timeout = self.config.total_timeout;
        let start_time = Instant::now();

        info!("Transaction: Initializing parallel execution for {} nodes.", self.graph.node_count());
        
        match tokio::time::timeout(total_timeout, self.execute_internal()).await {
            Ok(res) => {
                debug!("Transaction: DAG closure reached in {:?}", start_time.elapsed());
                res
            },
            Err(_) => {
                error!("Transaction: CRITICAL FAILURE - Global timeout of {:?} reached.", total_timeout);
                self.cancellation_token.cancel();
                if self.config.auto_rollback {
                    let _ = self.rollback().await;
                }
                Err(Error::Transaction(format!("Transaction timed out after {:?}", total_timeout)))
            }
        }
    }

    /// Simplified execution entry point.
    pub async fn execute(&mut self) -> Result<()> {
        self.execute_with_telemetry().await.map(|_| ())
    }
    
    /// The parallel execution loop.
    async fn execute_internal(&mut self) -> Result<Vec<TaskResult>> {
        let total_nodes = self.graph.node_count();
        let mut in_progress = HashSet::new();
        let mut worker_pool = JoinSet::new();
        let mut telemetry_results = Vec::new();
        
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent));

        while self.completed_indices.len() < total_nodes {
            if self.cancellation_token.is_cancelled() {
                worker_pool.abort_all();
                if self.config.auto_rollback { let _ = self.rollback().await; }
                return Err(Error::Transaction("Transaction cancelled.".into()));
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
                let permit = match semaphore.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(e) => return Err(Error::Transaction(format!("Semaphore failure: {}", e))),
                };
                
                in_progress.insert(idx);
                
                let action = self.graph[idx].clone();
                let registry = self.registry.clone();
                let journal = self.journal.clone();
                let cancel_token = self.cancellation_token.clone();
                let config = self.config.clone();

                worker_pool.spawn(async move {
                    let _permit_holder = permit; 
                    Self::execute_node_with_retry(
                        action, registry, journal, config, cancel_token, idx
                    ).await
                });
            }

            if let Some(finished_task) = worker_pool.join_next().await {
                let task_data = finished_task.map_err(|e| Error::Transaction(format!("Worker Panic: {}", e)))?;
                
                // Ownership Guard: Determine failure status before moving data
                let is_failure = task_data.result.is_err();

                if !is_failure {
                    trace!("Node {}:{} succeeded.", task_data.backend_name, task_data.package_name);
                    in_progress.remove(&task_data.node_index);
                    self.completed_indices.insert(task_data.node_index);
                    self.history.push(task_data.node_index);
                    telemetry_results.push(task_data);
                } else {
                    let error_msg = task_data.result.as_ref().err()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "Execution Error".into());

                    error!("Node {}:{} FAILED: {}", task_data.backend_name, task_data.package_name, error_msg);
                    
                    self.diagnostics.print_suggestions(&error_msg, &task_data.backend_name);

                    let final_err = task_data.result.clone().err().unwrap();
                    telemetry_results.push(task_data);

                    if self.config.auto_rollback {
                        info!("Transaction: Commencing auto-rollback...");
                        worker_pool.abort_all();
                        let _ = self.rollback().await;
                    }
                    return Err(final_err);
                }
            } else if in_progress.is_empty() && self.completed_indices.len() < total_nodes {
                return Err(Error::Transaction("DAG Logic Stall: Cycle detected in closure.".into()));
            }
        }
        Ok(telemetry_results)
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

        let start_time_utc = chrono::Utc::now();
        let start_instant = Instant::now();

        let backend_cap = match registry.get(&b_name) {
            Some(cap) => cap,
            None => return TaskResult { 
                node_index, backend_name: b_name.clone(), package_name: p_name, 
                properties: HashMap::new(), attempt: 0, duration: Duration::ZERO, 
                bytes_downloaded: 0, start_time: start_time_utc, 
                result: Err(Error::BackendNotFound(b_name)) 
            },
        };

        let journal_id = {
            let mut j = journal.lock().await;
            match j.record_start(j_action.clone()) {
                Ok(id) => id,
                Err(e) => return TaskResult { 
                    node_index, backend_name: b_name, package_name: p_name, 
                    properties: HashMap::new(), attempt: 0, duration: Duration::ZERO, 
                    bytes_downloaded: 0, start_time: start_time_utc, 
                    result: Err(Error::Journal(format!("WAL error: {}", e))) 
                },
            }
        };

        let mut attempt = 0;
        let mut last_error = None;
        
        while attempt <= config.max_retries {
            attempt += 1;
            if cancel_token.is_cancelled() {
                return TaskResult { 
                    node_index, backend_name: b_name, package_name: p_name, 
                    properties: HashMap::new(), attempt: attempt - 1, duration: start_instant.elapsed(), 
                    bytes_downloaded: 0, start_time: start_time_utc, result: Err(Error::Cancelled) 
                };
            }
            
            if attempt > 1 {
                let backoff = std::cmp::min(config.initial_backoff * (1 << (attempt - 2)), config.max_backoff);
                tokio::time::sleep(backoff).await;
            }

            let result = tokio::time::timeout(config.node_timeout, async {
                match &action {
                    GraphAction::Install(spec) => {
                        if let Some(handler) = backend_cap.as_installable() {
                            handler.install(&[spec.clone()], backend_cap.needs_root()).await?;
                            let mut props = HashMap::new();
                            if let Some(q) = backend_cap.as_queryable() {
                                if let Ok(Some(pkg)) = q.info(&spec.name).await {
                                    props = pkg.properties;
                                }
                            }
                            let bytes = props.get("download_size").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                            Ok((props, bytes))
                        } else { 
                            Err(Error::Transaction(format!("Backend '{}' is not installable.", b_name))) 
                        }
                    }
                    GraphAction::Remove { name, .. } => {
                        if let Some(handler) = backend_cap.as_installable() {
                            handler.remove(&[name.clone()], backend_cap.needs_root()).await?;
                            Ok((HashMap::new(), 0))
                        } else { 
                            Err(Error::Transaction(format!("Backend '{}' is not removable.", b_name))) 
                        }
                    }
                }
            }).await;
            
            match result {
                Ok(Ok((props, bytes))) => {
                    let mut j = journal.lock().await;
                    let _ = j.record_success(&journal_id, props.clone());
                    return TaskResult { 
                        node_index, backend_name: b_name, package_name: p_name, 
                        properties: props, attempt: attempt - 1, duration: start_instant.elapsed(), 
                        bytes_downloaded: bytes, start_time: start_time_utc, result: Ok(()) 
                    };
                }
                Ok(Err(e)) => { last_error = Some(e); }
                Err(_) => { last_error = Some(Error::Transaction("Node timed out.".into())); }
            }
        }
        
        let final_err = last_error.unwrap_or(Error::Transaction("Unknown error".into()));
        let mut j = journal.lock().await;
        let _ = j.record_failure(&journal_id, &format!("{}", final_err));

        TaskResult { 
            node_index, backend_name: b_name, package_name: p_name, 
            properties: HashMap::new(), attempt: attempt - 1, duration: start_instant.elapsed(), 
            bytes_downloaded: 0, start_time: start_time_utc, result: Err(final_err) 
        }
    }

    async fn rollback(&mut self) -> Result<()> {
        info!("Transaction: Reverting modification history.");
        for &idx in self.history.iter().rev() {
            let action = &self.graph[idx];
            match action {
                GraphAction::Install(spec) => {
                    if let Some(b) = self.registry.get(&spec.backend) {
                        if let Some(h) = b.as_installable() { 
                            let _ = h.remove(&[spec.name.clone()], b.needs_root()).await; 
                        }
                    }
                }
                GraphAction::Remove { name, backend } => {
                    if let Some(b) = self.registry.get(backend) {
                        if let Some(h) = b.as_installable() {
                            let spec = PackageSpec { name: name.clone(), backend: backend.clone(), options: HashMap::new(), requires: vec![] };
                            let _ = h.install(&[spec], b.needs_root()).await;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}