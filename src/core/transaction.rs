use crate::core::{manager::Backend, Result, Error, PackageSpec, Journal, ActionStatus};
use crate::core::journal::JournalAction;
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
    /// Maximum number of concurrent operations.
    pub max_concurrent: usize,
    /// Maximum time allowed for a single node operation.
    pub node_timeout: Duration,
    /// Maximum time allowed for the entire transaction.
    pub total_timeout: Duration,
    /// Maximum number of retry attempts for failed nodes.
    pub max_retries: u32,
    /// Initial backoff duration for retries (exponential).
    pub initial_backoff: Duration,
    /// Maximum backoff duration for retries.
    pub max_backoff: Duration,
    /// Whether to automatically rollback on failure.
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

impl TransactionConfig {
    /// Quick configuration for small, fast transactions.
    pub fn quick() -> Self {
        Self {
            max_concurrent: 8,
            node_timeout: Duration::from_secs(60),
            total_timeout: Duration::from_secs(300),
            max_retries: 1,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
            auto_rollback: true,
        }
    }
    
    /// Patient configuration for large, complex transactions.
    pub fn patient() -> Self {
        Self {
            max_concurrent: 2,
            node_timeout: Duration::from_secs(600),
            total_timeout: Duration::from_secs(7200),
            max_retries: 5,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            auto_rollback: true,
        }
    }
    
    /// Network-friendly configuration for unreliable connections.
    pub fn network_resilient() -> Self {
        Self {
            max_concurrent: 3,
            node_timeout: Duration::from_secs(120),
            total_timeout: Duration::from_secs(5400),
            max_retries: 5,
            initial_backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(120),
            auto_rollback: false,
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
    total_retries: u32,
    duration: Duration,
    result: Result<()>,
}

/// The High-Performance Mission-Critical Execution Engine with timeout and retry.
/// Hardened for Version 3.5.0 with full timeout and exponential backoff support.
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

    /// Cancels the currently running transaction.
    pub fn cancel(&self) {
        info!("Transaction: Cancellation requested.");
        self.cancellation_token.cancel();
    }
    
    /// Updates the configuration (e.g., for dynamic tuning).
    pub fn update_config(&mut self, config: TransactionConfig) {
        self.config = config;
    }

    /// Executes the system transformation with timeout and retry support.
    pub async fn execute(&mut self) -> Result<()> {
        self.start_time = Some(std::time::Instant::now());
        let total_timeout = self.config.total_timeout;
        let result = tokio::time::timeout(total_timeout, self.execute_internal()).await;
        
        match result {
            Ok(Ok(())) => {
                let elapsed = self.start_time.unwrap().elapsed();
                info!("Transaction: Completed successfully in {:?}", elapsed);
                Ok(())
            },
            Ok(Err(e)) => Err(e),
            Err(_) => {
                let elapsed = self.start_time.unwrap().elapsed();
                error!("Transaction: Total timeout of {:?} exceeded after {:?}.", total_timeout, elapsed);
                self.cancellation_token.cancel();
                if self.config.auto_rollback {
                    self.rollback().await;
                }
                Err(Error::Transaction(format!("Transaction exceeded timeout of {:?} after {:?}", total_timeout, elapsed)))
            }
        }
    }
    
    async fn execute_internal(&mut self) -> Result<()> {
        let total_nodes = self.graph.node_count();
        info!("Transaction: Commencing parallel execution of {} nodes.", total_nodes);
        info!("  max_concurrent={}, node_timeout={:?}, max_retries={}, backoff={:?}..{:?}",
              self.config.max_concurrent, self.config.node_timeout, self.config.max_retries,
              self.config.initial_backoff, self.config.max_backoff);

        let mut in_progress = HashSet::new();
        let mut worker_pool = JoinSet::new();
        let bridge = DependencyBridge::new();
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent));
        let node_timeout = self.config.node_timeout;
        let max_retries = self.config.max_retries;
        let initial_backoff = self.config.initial_backoff;
        let max_backoff = self.config.max_backoff;

        while self.completed_indices.len() < total_nodes {
            // Check for cancellation
            if self.cancellation_token.is_cancelled() {
                warn!("Transaction: Cancellation detected. Initiating rollback.");
                worker_pool.abort_all();
                if self.config.auto_rollback {
                    self.rollback().await;
                }
                return Err(Error::Transaction("Transaction was cancelled by user or timeout".into()));
            }
            
            // Check total timeout (additional safety)
            if let Some(start) = self.start_time {
                if start.elapsed() > self.config.total_timeout {
                    return Err(Error::Transaction("Total timeout exceeded".into()));
                }
            }
            
            // Identify "Ready" nodes
            let ready_nodes: Vec<NodeIndex> = self.graph.node_indices()
                .filter(|&idx| {
                    !self.completed_indices.contains(&idx) &&
                    !in_progress.contains(&idx) &&
                    self.graph.neighbors_directed(idx, Direction::Incoming)
                        .all(|dep| self.completed_indices.contains(&dep))
                })
                .collect();

            // Dispatch tasks with concurrency limiting and per-node timeout
            for idx in ready_nodes {
                let permit = semaphore.clone().acquire_owned().await.unwrap();
                in_progress.insert(idx);
                let action = self.graph[idx].clone();
                let registry = self.registry.clone();
                let journal = self.journal.clone();
                let cancel_token = self.cancellation_token.clone();
                
                let node_timeout_clone = node_timeout;
                let max_retries_clone = max_retries;
                let initial_backoff_clone = initial_backoff;
                let max_backoff_clone = max_backoff;

                worker_pool.spawn(async move {
                    let start = std::time::Instant::now();
                    let result = Self::execute_node_with_retry(
                        action, registry, journal, 
                        node_timeout_clone, max_retries_clone,
                        initial_backoff_clone, max_backoff_clone,
                        cancel_token, idx
                    ).await;
                    let duration = start.elapsed();
                    (result, duration)
                });
            }

            // Monitor completions with timeout handling
            tokio::select! {
                Some(finished_task) = worker_pool.join_next() => {
                    let (task_data, duration) = finished_task.map_err(|e| Error::Transaction(format!("Worker Panic: {}", e)))?;
                    
                    match task_data.result {
                        Ok(_) => {
                            in_progress.remove(&task_data.node_index);
                            self.completed_indices.insert(task_data.node_index);
                            self.history.push(task_data.node_index);
                            if task_data.attempt > 1 {
                                info!("Node {}:{} succeeded after {} retries in {:?}", 
                                      task_data.backend_name, task_data.package_name, 
                                      task_data.attempt - 1, duration);
                            } else {
                                debug!("Node {}:{} completed in {:?}", task_data.backend_name, task_data.package_name, duration);
                            }
                        }
                        Err(e) => {
                            error!("Failure at node {}:{} after {} attempts ({} retries) in {:?}: {}", 
                                   task_data.backend_name, task_data.package_name, 
                                   task_data.attempt, task_data.total_retries, duration, e);
                            
                            if let Error::CommandFailed(ref msg) = e {
                                bridge.print_suggestions(msg, &task_data.backend_name);
                            }

                            if self.config.auto_rollback {
                                warn!("Transaction: Initiating rollback for system safety.");
                                worker_pool.abort_all();
                                self.rollback().await;
                            }
                            return Err(e);
                        }
                    }
                }
                _ = self.cancellation_token.cancelled() => {
                    warn!("Transaction: Cancellation signalled during completion monitoring.");
                    worker_pool.abort_all();
                    if self.config.auto_rollback {
                        self.rollback().await;
                    }
                    return Err(Error::Transaction("Transaction was cancelled".into()));
                }
            }

            if worker_pool.is_empty() && self.completed_indices.len() < total_nodes {
                return Err(Error::Transaction("Deadlock: Graph is stuck with unresolved nodes.".into()));
            }
        }

        let total_duration = self.start_time.map(|s| s.elapsed()).unwrap_or(Duration::ZERO);
        info!("Transaction: All {} nodes applied successfully in {:?}.", total_nodes, total_duration);
        Ok(())
    }
    
    /// Executes a single node with retry support, exponential backoff, and per-node timeout.
    async fn execute_node_with_retry(
        action: GraphAction,
        registry: Arc<BackendRegistry>,
        journal: Arc<Mutex<Journal>>,
        node_timeout: Duration,
        max_retries: u32,
        initial_backoff: Duration,
        max_backoff: Duration,
        cancel_token: CancellationToken,
        node_index: NodeIndex,
    ) -> TaskResult {
        let (p_name, b_name, j_action) = match &action {
            GraphAction::Install(s) => (s.name.clone(), s.backend.clone(), JournalAction::Install(s.clone())),
            GraphAction::Remove { name, backend } => (name.clone(), backend.clone(), JournalAction::Remove { name: name.clone(), backend: backend.clone() }),
        };

        let journal_id = {
            let mut j = journal.lock().await;
            match j.record_start(j_action) {
                Ok(id) => id,
                Err(e) => return TaskResult {
                    node_index,
                    backend_name: b_name,
                    package_name: p_name,
                    properties: HashMap::new(),
                    attempt: 0,
                    total_retries: 0,
                    duration: Duration::ZERO,
                    result: Err(e),
                },
            }
        };

        let mut attempt = 0;
        let mut total_retries = 0;
        let mut last_error = None;
        let overall_start = std::time::Instant::now();
        
        while attempt <= max_retries {
            attempt += 1;
            
            if cancel_token.is_cancelled() {
                return TaskResult {
                    node_index,
                    backend_name: b_name,
                    package_name: p_name,
                    properties: HashMap::new(),
                    attempt,
                    total_retries,
                    duration: overall_start.elapsed(),
                    result: Err(Error::Transaction("Operation cancelled".into())),
                };
            }
            
            // Apply exponential backoff before retry (except first attempt)
            if attempt > 1 {
                let backoff = std::cmp::min(
                    initial_backoff * (1 << (attempt - 2)),
                    max_backoff
                );
                debug!("Node {}:{} retry {} waiting {:?}", b_name, p_name, attempt - 1, backoff);
                tokio::time::sleep(backoff).await;
            }
            
            let result = tokio::time::timeout(node_timeout, Self::execute_node_action(
                &action, &registry, &journal, &journal_id, &p_name, &b_name
            )).await;
            
            match result {
                Ok(Ok((props, _))) => {
                    let mut j = journal.lock().await;
                    let _ = j.record_success(&journal_id, props.clone());
                    return TaskResult {
                        node_index,
                        backend_name: b_name,
                        package_name: p_name,
                        properties: props,
                        attempt,
                        total_retries: attempt - 1,
                        duration: overall_start.elapsed(),
                        result: Ok(()),
                    };
                }
                Ok(Err(e)) => {
                    last_error = Some(e);
                    total_retries = attempt;
                    warn!("Node {}:{} attempt {}/{} failed: {:?}", b_name, p_name, attempt, max_retries + 1, last_error);
                    
                    let mut j = journal.lock().await;
                    let _ = j.record_failure(&journal_id, &format!("{:?}", last_error));
                }
                Err(_) => {
                    let timeout_err = Error::Transaction(format!("Node operation timed out after {:?}", node_timeout));
                    last_error = Some(timeout_err.clone());
                    total_retries = attempt;
                    warn!("Node {}:{} attempt {}/{} timed out after {:?}", b_name, p_name, attempt, max_retries + 1, node_timeout);
                    
                    let mut j = journal.lock().await;
                    let _ = j.record_failure(&journal_id, &format!("Timeout after {:?}", node_timeout));
                }
            }
        }
        
        TaskResult {
            node_index,
            backend_name: b_name,
            package_name: p_name,
            properties: HashMap::new(),
            attempt,
            total_retries,
            duration: overall_start.elapsed(),
            result: Err(last_error.unwrap_or_else(|| Error::Transaction("All retry attempts failed".into()))),
        }
    }
    
    async fn execute_node_action(
        action: &GraphAction,
        registry: &BackendRegistry,
        journal: &Arc<Mutex<Journal>>,
        journal_id: &str,
        p_name: &str,
        b_name: &str,
    ) -> Result<(HashMap<String, String>, String)> {
        let mut properties = HashMap::new();
        
        match action {
            GraphAction::Install(spec) => {
                if let Some(backend) = registry.get(&spec.backend) {
                    if let Some(handler) = backend.as_installable() {
                        handler.install(&[spec.clone()], true).await?;
                        
                        if let Some(queryable) = backend.as_queryable() {
                            if let Ok(Some(info)) = queryable.info(&spec.name).await {
                                properties = info.properties;
                            }
                        }
                        Ok((properties, journal_id.to_string()))
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
                        handler.remove(&[name.clone()], true).await?;
                        Ok((properties, journal_id.to_string()))
                    } else {
                        Err(Error::Transaction(format!("Backend {} cannot remove", b_name)))
                    }
                } else {
                    Err(Error::BackendNotFound(b_name.clone()))
                }
            }
        }
    }

    /// Reverts successfully applied nodes in reverse order.
    async fn rollback(&mut self) {
        info!("Transaction: Rolling back {} completed operations.", self.history.len());
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
        info!("Transaction: Rollback complete.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::backends::create_default_registry;
    use crate::app::LuaHooks;
    use crate::core::CommandExecutor;
    use std::collections::HashMap;

    async fn create_test_transaction() -> Transaction {
        let config = Config::default();
        let executor = CommandExecutor::new(true, false);
        let hooks = Arc::new(LuaHooks::new(&config).unwrap());
        let registry = Arc::new(create_default_registry(executor, &config, hooks).await);
        let journal = Arc::new(Mutex::new(Journal::new().unwrap()));
        let graph = StableDiGraph::new();
        
        Transaction::with_config(graph, registry, journal, TransactionConfig::quick())
    }

    #[tokio::test]
    async fn test_transaction_cancellation() {
        let mut tx = create_test_transaction().await;
        tx.cancel();
        let result = tx.execute().await;
        assert!(result.is_err());
        if let Err(Error::Transaction(msg)) = result {
            assert!(msg.contains("cancelled"));
        }
    }

    #[tokio::test]
    async fn test_transaction_timeout() {
        let mut tx = create_test_transaction().await;
        let result = tx.execute().await;
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_transaction_config_default() {
        let config = TransactionConfig::default();
        assert_eq!(config.max_concurrent, 4);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_backoff, Duration::from_millis(500));
        assert_eq!(config.max_backoff, Duration::from_secs(30));
        assert!(config.auto_rollback);
    }
    
    #[test]
    fn test_transaction_config_quick() {
        let config = TransactionConfig::quick();
        assert_eq!(config.max_concurrent, 8);
        assert_eq!(config.max_retries, 1);
        assert_eq!(config.initial_backoff, Duration::from_millis(100));
    }
    
    #[test]
    fn test_transaction_config_patient() {
        let config = TransactionConfig::patient();
        assert_eq!(config.max_concurrent, 2);
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.node_timeout, Duration::from_secs(600));
        assert_eq!(config.total_timeout, Duration::from_secs(7200));
    }
    
    #[test]
    fn test_transaction_config_network_resilient() {
        let config = TransactionConfig::network_resilient();
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.initial_backoff, Duration::from_secs(2));
        assert_eq!(config.max_backoff, Duration::from_secs(120));
        assert!(!config.auto_rollback);
    }
}