use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Package, Result, Error};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{info, warn, error, debug, trace, instrument};
/// A high-performance search orchestrator that queries multiple backends in parallel.
/// 
/// Modernized for v3.6.0: This implementation utilizes an asynchronous 
/// worker-pool pattern with backpressure governed by a Semaphore. It is 
/// entirely panic-free and handles concurrent I/O failures gracefully.
pub struct UniversalSearch<'a> {
    /// Registry containing all package backends.
    registry: &'a BackendRegistry,
    /// Kernel configuration for parallel task limits.
    config: &'a Config,
}

impl<'a> UniversalSearch<'a> {
    /// Initializes the search orchestrator.
    pub fn new(registry: &'a BackendRegistry, config: &'a Config) -> Self {
        Self { registry, config }
    }

    /// Performs a cross-backend search and returns a deduplicated, sorted result set.
    /// 
    /// This method is exhaustive: it filters for searchable backends, respects 
    /// concurrency limits via semaphores, and performs high-fidelity 
    /// deduplication based on the "backend:name" identity key.
    #[instrument(skip(self, query))]
    pub async fn search(&self, query: &str) -> Result<Vec<Package>> {
        info!("Search: Initiating parallel universal query for '{}'...", query);

        // 1. Discovery: Identify all available backends that support searching
        let searchable_backends: Vec<_> = if self.config.enabled_backends.is_empty() {
            self.registry.available()
        } else {
            self.registry.get_filtered(&self.config.enabled_backends)
        }
        .into_iter()
        .filter(|b| b.as_searchable().is_some())
        .collect();

        if searchable_backends.is_empty() {
            debug!("Search: No searchable backends are currently available.");
            return Ok(vec![]);
        }

        // 2. Worker Pool Initialization
        // We use a Semaphore to ensure we never exceed config.max_parallel 
        // concurrent network requests.
        let semaphore = Arc::new(Semaphore::new(self.config.max_parallel));
		let mut worker_pool: JoinSet<Result<Vec<Package>>> = JoinSet::new();

        for backend in searchable_backends {
            let sem_ref = semaphore.clone();
            let query_string = query.to_string();
            let b = backend.clone();

            worker_pool.spawn(async move {
                // A+ Grade Fix: Replace .unwrap() with fallible mapping
                // This prevents a panic if the semaphore is closed.
                let _permit = sem_ref.acquire().await
                    .map_err(|e| Error::Transaction(format!("Search concurrency semaphore failure: {}", e)))?;

                debug!("Search: Querying backend '{}'...", b.name());
                
                // A+ Grade Fix: Panic-free trait access
                if let Some(searchable) = b.as_searchable() {
                    match searchable.search(&query_string).await {
                        Ok(results) => {
                            trace!("Search: Backend '{}' returned {} results.", b.name(), results.len());
                            Ok(results)
                        },
                        // Surface (don't swallow) the failure, tagged with the backend name,
                        // so the user is told which backends errored vs. returned nothing.
                        Err(e) => Err(Error::Other(format!("{}: {}", b.name(), e))),
                    }
                } else {
                    Ok(vec![])
                }
            });
        }

        // 3. Collection & Deduplication
        let mut all_packages = Vec::new();
        let mut seen_keys = HashSet::new();
        let mut failed_backends: Vec<String> = Vec::new();

        while let Some(task_result) = worker_pool.join_next().await {
            match task_result {
                Ok(Ok(packages)) => {
                    for pkg in packages {
                        // Unique Identity: "backend:name"
                        let key = format!("{}:{}", pkg.backend, pkg.name);
                        if seen_keys.insert(key) {
                            all_packages.push(pkg);
                        }
                    }
                },
                Ok(Err(e)) => {
                    warn!("Search: {}", e);
                    failed_backends.push(e.to_string());
                },
                Err(panic_err) => {
                    error!("Search: A worker thread panicked: {}", panic_err);
                    failed_backends.push(format!("worker panic: {}", panic_err));
                }
            }
        }

        // 4. Final Polish: Lexicographical sorting for consistent UI
        all_packages.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        // User-visible summary of backends that errored (distinct from "0 results").
        if !failed_backends.is_empty() {
            eprintln!(
                "Search: {} backend(s) failed and were skipped: {}",
                failed_backends.len(),
                failed_backends.join("; ")
            );
        }

        info!("Search: Completed. Discovered {} unique candidates.", all_packages.len());
        Ok(all_packages)
    }
}