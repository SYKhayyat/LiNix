use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Error, Package, Result};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, error, info, instrument, trace, warn};
pub struct UniversalSearch<'a> {
    registry: &'a BackendRegistry,
    config: &'a Config,
    /// The backends to search, from the `priority` file (II.6). Empty = every available
    /// backend (the file is missing, which the resolver already refuses elsewhere).
    enabled: Vec<String>,
}

impl<'a> UniversalSearch<'a> {
    pub fn new(registry: &'a BackendRegistry, config: &'a Config, enabled: Vec<String>) -> Self {
        Self {
            registry,
            config,
            enabled,
        }
    }

    #[instrument(skip(self, query))]
    pub async fn search(&self, query: &str) -> Result<Vec<Package>> {
        info!(
            "searching all backends for '{}'",
            query
        );

        let searchable_backends: Vec<_> = if self.enabled.is_empty() {
            self.registry.available()
        } else {
            self.registry.get_filtered(&self.enabled)
        }
        .into_iter()
        .filter(|b| b.as_searchable().is_some())
        .collect();

        if searchable_backends.is_empty() {
            debug!("No searchable backends are currently available.");
            return Ok(vec![]);
        }

        // The semaphore caps concurrent NETWORK requests at `max_parallel`; without it a
        // wide registry would open one remote query per backend at once.
        let semaphore = Arc::new(Semaphore::new(self.config.max_parallel));
        let mut worker_pool: JoinSet<Result<Vec<Package>>> = JoinSet::new();

        for backend in searchable_backends {
            let sem_ref = semaphore.clone();
            let query_string = query.to_string();
            let b = backend.clone();

            worker_pool.spawn(async move {
                let _permit = sem_ref.acquire().await.map_err(|e| {
                    Error::Transaction(format!("Search concurrency semaphore failure: {}", e))
                })?;

                debug!("Querying backend '{}'...", b.name());

                if let Some(searchable) = b.as_searchable() {
                    match searchable.search(&query_string).await {
                        Ok(results) => {
                            trace!(
                                "Backend '{}' returned {} results.",
                                b.name(),
                                results.len()
                            );
                            Ok(results)
                        }
                        // Surface (don't swallow) the failure, tagged with the backend name,
                        // so the user is told which backends errored vs. returned nothing.
                        Err(e) => Err(Error::Other(format!("{}: {}", b.name(), e))),
                    }
                } else {
                    Ok(vec![])
                }
            });
        }

        let mut all_packages = Vec::new();
        let mut seen_keys = HashSet::new();
        let mut failed_backends: Vec<String> = Vec::new();

        while let Some(task_result) = worker_pool.join_next().await {
            match task_result {
                Ok(Ok(packages)) => {
                    for pkg in packages {
                        // Identity is backend-qualified: the same name from two backends is
                        // two distinct results, not a duplicate to collapse.
                        let key = format!("{}:{}", pkg.backend, pkg.name);
                        if seen_keys.insert(key) {
                            all_packages.push(pkg);
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!("{}", e);
                    failed_backends.push(e.to_string());
                }
                Err(panic_err) => {
                    error!("A worker thread panicked: {}", panic_err);
                    failed_backends.push(format!("worker panic: {}", panic_err));
                }
            }
        }

        // Sorted because JoinSet completion order is arbitrary — without this the same
        // query prints in a different order run to run.
        all_packages.sort_by_key(|p| p.name.to_lowercase());

        // User-visible summary of backends that errored (distinct from "0 results").
        if !failed_backends.is_empty() {
            eprintln!(
                "Search: {} backend(s) failed and were skipped: {}",
                failed_backends.len(),
                failed_backends.join("; ")
            );
        }

        info!(
            "Completed. Discovered {} unique candidates.",
            all_packages.len()
        );
        Ok(all_packages)
    }
}
