use crate::core::{Result, Error, PackageSpec, Journal};
use crate::core::journal::JournalAction;
use crate::backends::BackendRegistry;
use crate::app::bridge::DependencyBridge;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn, error};
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
        Self::patient()
    }
}

impl TransactionConfig {
    pub fn quick() -> Self {
        Self {
            max_concurrent: 8,
            node_timeout: Duration::from_secs(60),
            total_timeout: Duration::from_secs(600),
            max_retries: 1,
            initial_backoff: Duration::from_millis(200),
            max_backoff: Duration::from_secs(5),
            auto_rollback: true,
        }
    }

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

    pub fn network_resilient() -> Self {
        Self {
            max_concurrent: 2,
            node_timeout: Duration::from_secs(600),
            total_timeout: Duration::from_secs(7200),
            max_retries: 5,
            initial_backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(120),
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

/// The result of an individual node execution with complete telemetry.
#[derive(Debug)]
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
    config: TransactionConfig,
    completed_indices: HashSet<NodeIndex>,
    history: Vec<NodeIndex>,
    cancellation_token: CancellationToken,
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
        }
    }

    pub async fn execute_with_telemetry(&mut self) -> Result<Vec<TaskResult>> {
        let total_timeout = self.config.total_timeout;
        
        match tokio::time::timeout(total_timeout, self.execute_internal()).await {
            Ok(res) => res,
            Err(_) => {
                error!("Transaction: Total timeout of {:?} exceeded.", total_timeout);
                self.cancellation_token.cancel();
                if self.config.auto_rollback {
                    let _ = self.rollback().await;
                }
                Err(Error::Transaction(format!("Transaction timed out after {:?}", total_timeout)))
            }
        }
    }

    pub async fn execute(&mut self) -> Result<()> {
        self.execute_with_telemetry().await.map(|_| ())
    }
    
    async fn execute_internal(&mut self) -> Result<Vec<TaskResult>> {
        let total_nodes = self.graph.node_count();
        let mut in_progress = HashSet::new();
        let mut worker_pool = JoinSet::new();
        let mut telemetry_results = Vec::new();
        let bridge = DependencyBridge::new();
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent));

        while self.completed_indices.len() < total_nodes {
            if self.cancellation_token.is_cancelled() {
                worker_pool.abort_all();
                if self.config.auto_rollback { let _ = self.rollback().await; }
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
                let permit = semaphore.clone().acquire_owned().await
                    .map_err(|e| Error::Transaction(format!("Semaphore error: {}", e)))?;
                
                in_progress.insert(idx);
                
                let action = self.graph[idx].clone();
                let registry = self.registry.clone();
                let journal = self.journal.clone();
                let cancel_token = self.cancellation_token.clone();
                let config = self.config.clone();

                worker_pool.spawn(async move {
                    let _permit = permit; 
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
                        telemetry_results.push(task_data);
                    }
                    Err(ref e) => {
                        let err_clone = e.clone();
                        let backend = task_data.backend_name.clone();
                        let package = task_data.package_name.clone();
                        
                        error!("Node {}:{} failed after {:?} (Attempt {}): {}", 
                               backend, package, task_data.duration, task_data.attempt, err_clone);
                        
                        if let Error::CommandFailed(ref msg) = err_clone {
                            bridge.print_suggestions(msg, &backend);
                        }

                        telemetry_results.push(task_data);

                        if self.config.auto_rollback {
                            worker_pool.abort_all();
                            let _ = self.rollback().await;
                        }
                        return Err(err_clone);
                    }
                }
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
        let start_instant = std::time::Instant::now();

        // FIX: Fulfills logic bug fix 1. Check backend existence BEFORE creating journal entry.
        let backend_cap = match registry.get(&b_name) {
            Some(cap) => cap,
            None => return TaskResult { 
                node_index, backend_name: b_name.clone(), package_name: p_name, 
                properties: HashMap::new(), attempt: 0, duration: Duration::ZERO, 
                bytes_downloaded: 0, start_time: start_time_utc, result: Err(Error::BackendNotFound(b_name)) 
            },
        };

        let sudo_required = backend_cap.needs_root();

        let journal_id = {
            let mut j = journal.lock().await;
            j.record_start(j_action.clone()).unwrap_or_else(|_| "transient_id".to_string())
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
                            handler.install(&[spec.clone()], sudo_required).await?;
                            let props = if let Some(q) = backend_cap.as_queryable() {
                                q.info(&spec.name).await?.map(|p| p.properties).unwrap_or_default()
                            } else { HashMap::new() };
                            
                            let bytes = props.get("download_size").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                            Ok((props, bytes))
                        } else { Err(Error::Transaction(format!("Backend '{}' is not installable", b_name))) }
                    }
                    GraphAction::Remove { name, backend: _ } => {
                        if let Some(handler) = backend_cap.as_installable() {
                            handler.remove(&[name.clone()], sudo_required).await?;
                            Ok((HashMap::new(), 0))
                        } else { Err(Error::Transaction(format!("Backend '{}' is not removable", b_name))) }
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
                Ok(Err(e)) => {
                    warn!("Attempt {} for {}:{} failed: {}", attempt, b_name, p_name, e);
                    last_error = Some(e);
                }
                Err(_) => {
                    warn!("Attempt {} for {}:{} timed out", attempt, b_name, p_name);
                    last_error = Some(Error::Transaction("Node timed out".into()));
                }
            }
        }
        
        let final_err = last_error.unwrap_or(Error::Transaction("Unknown failure".into()));
        let mut j = journal.lock().await;
        let _ = j.record_failure(&journal_id, &format!("{}", final_err));

        TaskResult { 
            node_index, backend_name: b_name, package_name: p_name, 
            properties: HashMap::new(), attempt: attempt - 1, duration: start_instant.elapsed(), 
            bytes_downloaded: 0, start_time: start_time_utc, result: Err(final_err) 
        }
    }

    async fn rollback(&mut self) -> Result<()> {
        info!("Transaction: Commencing rollback of {} successful operations.", self.history.len());
        let mut first_error = None;
        
        for &idx in self.history.iter().rev() {
            let action = &self.graph[idx];
            let mut success = false;
            
            for attempt in 1..=2 {
                let res = match action {
                    GraphAction::Install(spec) => {
                        if let Some(b) = self.registry.get(&spec.backend) {
                            if let Some(h) = b.as_installable() { h.remove(&[spec.name.clone()], b.needs_root()).await }
                            else { Ok(()) }
                        } else { Ok(()) }
                    }
                    GraphAction::Remove { name, backend } => {
                        if let Some(b) = self.registry.get(backend) {
                            if let Some(h) = b.as_installable() {
                                let spec = PackageSpec { 
                                    name: name.clone(), 
                                    backend: backend.clone(), 
                                    options: HashMap::new(), 
                                    requires: vec![] 
                                };
                                h.install(&[spec], b.needs_root()).await
                            } else { Ok(()) }
                        } else { Ok(()) }
                    }
                };

                if res.is_ok() {
                    success = true;
                    break;
                } else if attempt == 1 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                } else {
                    let err = res.err().unwrap();
                    warn!("Rollback failed for operation at index {:?} on attempt 2: {}", idx, err);
                    if first_error.is_none() {
                        first_error = Some(err);
                    }
                }
            }

            if !success {
                error!("CRITICAL: Rollback failed for a system modification.");
            }
        }

        match first_error {
            Some(e) => Err(Error::Transaction(format!("Rollback incomplete: {}", e))),
            None => {
                info!("Transaction: Rollback procedure completed successfully.");
                Ok(())
            }
        }
    }
}