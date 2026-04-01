use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Package, Result, PackageManager};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{info, warn};

pub struct UniversalSearch<'a> {
    registry: &'a BackendRegistry,
    config: &'a Config,
}

impl<'a> UniversalSearch<'a> {
    pub fn new(registry: &'a BackendRegistry, config: &'a Config) -> Self {
        Self { registry, config }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Package>> {
        info!("Universal search for '{}'...", query);

        let managers: Vec<Arc<dyn PackageManager>> = if self.config.enabled_backends.is_empty() {
            self.registry.available()
        } else {
            self.registry.get_filtered(&self.config.enabled_backends)
        };

        let semaphore = Arc::new(Semaphore::new(self.config.max_parallel));
        let mut tasks = tokio::task::JoinSet::new();

        for manager in managers {
            let sem = semaphore.clone();
            let query_str = query.to_string();
            let mgr = manager.clone();

            tasks.spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                match mgr.search(&query_str).await {
                    Ok(packages) => packages,
                    Err(e) => {
                        warn!("Search failed for {}: {}", mgr.name(), e);
                        Vec::new()
                    }
                }
            });
        }

        let mut all_packages = Vec::new();
        let mut seen = HashSet::new();
        while let Some(res) = tasks.join_next().await {
            if let Ok(packages) = res {
                for pkg in packages {
                    let key = format!("{}:{}", pkg.backend, pkg.name);
                    if seen.insert(key) { all_packages.push(pkg); }
                }
            }
        }
        
        all_packages.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(all_packages)
    }
}