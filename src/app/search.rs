use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Package, Result};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{info, warn, debug};

/// A high-performance search orchestrator that queries multiple backends in parallel.
/// Only backends that implement the `Searchable` capability are queried.
pub struct UniversalSearch<'a> {
    registry: &'a BackendRegistry,
    config: &'a Config,
}

impl<'a> UniversalSearch<'a> {
    pub fn new(registry: &'a BackendRegistry, config: &'a Config) -> Self {
        Self { registry, config }
    }

    /// Performs a cross-backend search and returns a deduplicated, sorted list of results.
    /// Uses a worker-pool pattern to prevent socket exhaustion or rate-limiting.
    pub async fn search(&self, query: &str) -> Result<Vec<Package>> {
        info!("Initiating parallel universal search for: '{}'", query);

        // Filter backends: Only those enabled in config and available on the system.
        let searchable_backends: Vec<_> = if self.config.enabled_backends.is_empty() {
            self.registry.available()
        } else {
            self.registry.get_filtered(&self.config.enabled_backends)
        }
        .into_iter()
        .filter(|b| b.as_searchable().is_some())
        .collect();

        if searchable_backends.is_empty() {
            debug!("No searchable backends available.");
            return Ok(vec![]);
        }

        // Use a semaphore to limit concurrent IO tasks (Phased parallel execution)
        let semaphore = Arc::new(Semaphore::new(self.config.max_parallel));
        let mut worker_pool = JoinSet::new();

        for backend in searchable_backends {
            let sem = semaphore.clone();
            let query_string = query.to_string();
            let b = backend.clone();

            worker_pool.spawn(async move {
                // Wait for a permit to ensure we don't exceed max_parallel
                let _permit = sem.acquire().await.unwrap();
                debug!("Searching backend: {}", b.name());
                
                // Re-acquire the searchable reference inside the task
                // We use unwrap here safely because we filtered for is_some() above
                match b.as_searchable().unwrap().search(&query_string).await {
                    Ok(results) => {
                        debug!("Backend '{}' returned {} results", b.name(), results.len());
                        results
                    },
                    Err(e) => {
                        warn!("Search failed for backend '{}': {}", b.name(), e);
                        vec![]
                    }
                }
            });
        }

        let mut all_packages = Vec::new();
        let mut seen = HashSet::new();

        // Collect results as workers finish
        while let Some(res) = worker_pool.join_next().await {
            match res {
                Ok(packages) => {
                    for pkg in packages {
                        // Deduplicate using the SOLID key: "backend:name"
                        let key = format!("{}:{}", pkg.backend, pkg.name);
                        if seen.insert(key) {
                            all_packages.push(pkg);
                        }
                    }
                },
                Err(e) => warn!("A search task panicked or was cancelled: {}", e),
            }
        }

        // Sort by name for a consistent CLI experience
        all_packages.sort_by(|a, b| a.name.cmp(&b.name));
        
        info!("Universal search completed. Found {} unique results.", all_packages.len());
        Ok(all_packages)
    }
}