use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Package, Result};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

/// Universal search across all backends
pub struct UniversalSearch<'a> {
    registry: &'a BackendRegistry,
    config: &'a Config,
}

impl<'a> UniversalSearch<'a> {
    pub fn new(registry: &'a BackendRegistry, config: &'a Config) -> Self {
        Self { registry, config }
    }

    /// Search across all available backends
    pub async fn search(&self, query: &str) -> Result<Vec<Package>> {
        info!("Searching for '{}' across all backends", query);
		

        let managers: Vec<std::sync::Arc<dyn crate::core::PackageManager>> = if self.config.enabled_backends.is_empty() {
            self.registry.available()
        } else {
            self.registry.get_filtered(&self.config.enabled_backends)
        };

        let semaphore = Arc::new(Semaphore::new(self.config.max_parallel));
        let mut handles = Vec::new();

        for manager in managers {
            let sem = semaphore.clone();
            let query = query.to_string();
            let manager = manager.clone();

            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();

                debug!("Searching in {}", manager.name());

                match manager.search(&query).await {
                    Ok(packages) => {
                        debug!("Found {} results in {}", packages.len(), manager.name());
                        packages
                    }
                    Err(e) => {
                        warn!("Search failed for {}: {}", manager.name(), e);
                        Vec::new()
                    }
                }
            });

            handles.push(handle);
        }

        // Collect results
        let mut all_packages = Vec::new();
        let mut seen = HashSet::new();

        for handle in handles {
            if let Ok(packages) = handle.await {
                for pkg in packages {
                    let key = format!("{}:{}", pkg.backend, pkg.name);
                    if !seen.contains(&key) {
                        seen.insert(key);
                        all_packages.push(pkg);
                    }
                }
            }
        }

        // Sort by name
        all_packages.sort_by(|a, b| a.name.cmp(&b.name));

        info!("Found {} total results", all_packages.len());
        Ok(all_packages)
    }

    /// Search in a specific backend
    pub async fn search_in(&self, query: &str, backend: &str) -> Result<Vec<Package>> {
        info!("Searching for '{}' in {}", query, backend);

        let manager = self
            .registry
            .get(backend)
            .ok_or_else(|| crate::core::Error::BackendNotFound(backend.to_string()))?;

        if !manager.is_available() {
            return Err(crate::core::Error::BackendUnavailable(backend.to_string()));
        }

        manager.search(query).await
    }

    /// Format search results for display
    pub fn format_results(packages: &[Package], json: bool) -> String {
        if json {
            serde_json::to_string_pretty(packages).unwrap_or_else(|_| "[]".to_string())
        } else {
            let mut output = String::new();

            if packages.is_empty() {
                return "No packages found.".to_string();
            }

            // Group by backend
            let mut by_backend: std::collections::HashMap<&str, Vec<&Package>> =
                std::collections::HashMap::new();
            for pkg in packages {
                by_backend.entry(&pkg.backend).or_default().push(pkg);
            }

            for (backend, pkgs) in by_backend {
                output.push_str(&format!("\n[{}]\n", backend));
                for pkg in pkgs {
                    let version = pkg.version.as_deref().unwrap_or("N/A");
                    output.push_str(&format!("  {} ({})\n", pkg.name, version));

                    if let Some(desc) = &pkg.description {
                        output.push_str(&format!("    {}\n", desc));
                    }
                }
            }

            output
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_results_empty() {
        let packages: Vec<Package> = Vec::new();
        let result = UniversalSearch::format_results(&packages, false);
        assert_eq!(result, "No packages found.");
    }

    #[test]
    fn test_format_results_json() {
        let packages = vec![Package::new("test-pkg", "apt")];
        let result = UniversalSearch::format_results(&packages, true);
        assert!(result.contains("test-pkg"));
        assert!(result.contains("apt"));
    }
}
