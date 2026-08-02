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
        info!("searching all backends for '{}'", query);

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

        // `network_parallel`, not `max_parallel`: these are ~22 sockets, and nothing about
        // waiting on one is bounded by how many cores the machine has. On a four-core laptop
        // the old cap ran the registries in six sequential waves, which is most of why this
        // command measured anywhere between 15s and 160s.
        let semaphore = Arc::new(Semaphore::new(self.config.network_parallel.max(1)));
        // A deadline per backend, because without one this command's latency is the *maximum*
        // over every registry rather than the median: one rate-limited GitHub call sets the
        // whole runtime. `check health` already bounds its per-backend probe for exactly this
        // reason and says how it chose the number; this one is twice the configured HTTP
        // timeout with a 30s floor, so a single request that runs all the way to its own
        // timeout still gets to finish and be counted.
        let deadline =
            std::time::Duration::from_secs((self.config.network_timeout_secs * 2).max(30));
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
                    match tokio::time::timeout(deadline, searchable.search(&query_string)).await {
                        Ok(Ok(results)) => {
                            trace!("Backend '{}' returned {} results.", b.name(), results.len());
                            Ok(results)
                        }
                        // Surface (don't swallow) the failure, tagged with the backend name,
                        // so the user is told which backends errored vs. returned nothing.
                        Ok(Err(e)) => Err(Error::Other(format!("{}: {}", b.name(), e))),
                        Err(_) => Err(Error::Other(format!(
                            "{}: did not answer in {}s",
                            b.name(),
                            deadline.as_secs()
                        ))),
                    }
                } else {
                    Ok(vec![])
                }
            });
        }

        let mut all_packages = Vec::new();
        let mut seen_keys: HashSet<(String, String)> = HashSet::new();
        let mut failed_backends: Vec<String> = Vec::new();

        while let Some(task_result) = worker_pool.join_next().await {
            match task_result {
                Ok(Ok(packages)) => {
                    for pkg in packages {
                        // Identity is backend-qualified: the same name from two backends is
                        // two distinct results, not a duplicate to collapse. A tuple key
                        // rather than a formatted one — the old form allocated a `String` per
                        // result purely to be hashed and thrown away.
                        if seen_keys.insert((pkg.backend.clone(), pkg.name.clone())) {
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
        // query prints in a different order run to run. `sort_by_key` with a `to_lowercase()`
        // built a fresh String on *every comparison*, so this is O(n log n) allocations for a
        // key that could be computed once per element.
        all_packages.sort_by_cached_key(|p| p.name.to_lowercase());

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
