use crate::core::{Result, Package, Error, StateRegistry};
use crate::backends::BackendRegistry;
use crate::config::Config;
use chrono::Local;
use tracing::{info, warn, debug};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::io::AsyncWriteExt;

/// The Ingestion Engine (Roadmap Point 3).
/// Responsible for moving a "dirty" system with existing manual installs into LiNix management.
/// 
/// Hardened for Phase 4.1: Decoupled from the global App object. Now receives 
/// specific dependencies, improving testability and adhering to the Single Responsibility Principle.
pub struct Migrator {
    registry: Arc<BackendRegistry>,
    state: Arc<Mutex<StateRegistry>>,
    groups_dir: std::path::PathBuf,
}

impl Migrator {
    /// Creates a new Migrator with explicit dependency injection.
    pub fn new(
        registry: Arc<BackendRegistry>,
        state: Arc<Mutex<StateRegistry>>,
        config: &Config,
    ) -> Self {
        Self {
            registry,
            state,
            groups_dir: config.groups_dir.clone(),
        }
    }

    /// Primary migration logic.
    pub async fn migrate(&self) -> Result<()> {
        info!("Migrator: Initiating full system discovery...");

        let mut discovered_packages = Vec::new();
        let mut seen = HashSet::new();

        // 1. Discovery Phase: Crawl the registry
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                debug!("Migrator: Probing {}...", backend.name());
                
                match queryable.list_manual().await {
                    Ok(pkgs) => {
                        let state_guard = self.state.lock().await;
                        for pkg in pkgs {
                            let key = format!("{}:{}", pkg.backend, pkg.name);
                            
                            // Only migrate if not already managed
                            if !state_guard.is_managed(&pkg.backend, &pkg.name) && seen.insert(key) {
                                discovered_packages.push(pkg);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Migrator: Failed to query backend {}: {}. Skipping.", backend.name(), e);
                    }
                }
            }
        }

        if discovered_packages.is_empty() {
            info!("Migrator: No unmanaged manual packages found.");
            return Ok(());
        }

        info!("Migrator: Found {} unmanaged packages.", discovered_packages.len());

        // 2. Manifest Generation Phase
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let filename = format!("migrated_{}.txt", timestamp);
        let path = self.groups_dir.join(&filename);

        let package_strings: Vec<String> = discovered_packages.iter()
            .map(|p| format!("{}:{}", p.backend, p.name))
            .collect();

        // Async directory and file creation
        if let Some(parent) = path.parent() {
            if !tokio::fs::try_exists(parent).await.unwrap_or(false) {
                tokio::fs::create_dir_all(parent).await.map_err(Error::from)?;
            }
        }

        let mut file = tokio::fs::File::create(&path).await.map_err(Error::from)?;
        file.write_all(package_strings.join("\n").as_bytes()).await.map_err(Error::from)?;
        file.flush().await.map_err(Error::from)?;

        info!("Migrator: Declarative manifest generated: {:?}", path);

        // 3. Ownership Acquisition Phase
        {
            let mut state_mut = self.state.lock().await;
            for pkg in &discovered_packages {
                state_mut.add_simple(&pkg.backend, &pkg.name, pkg.version.clone());
            }
            
            // StateRegistry::save is blocking; isolate in spawn_blocking
            let state_clone = state_mut.clone();
            tokio::task::spawn_blocking(move || {
                state_clone.save()
            }).await.map_err(|e| Error::Other(e.to_string()))??;
        }

        info!("Migrator: Ownership records updated.");
        
        println!("\nMigration Complete!");
        println!("{:-<40}", "");
        println!("Target Manifest:   {}", path.display());
        println!("Packages Ingested: {}", discovered_packages.len());
        println!("{:-<40}", "");

        Ok(())
    }

    /// Performs a "Dry-Run" migration to show the user what would be added to their config.
    pub async fn audit(&self) -> Result<Vec<Package>> {
        let mut unmanaged = Vec::new();
        let state_guard = self.state.lock().await;

        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                match queryable.list_manual().await {
                    Ok(pkgs) => {
                        for pkg in pkgs {
                            if !state_guard.is_managed(backend.name(), &pkg.name) {
                                unmanaged.push(pkg);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Migrator Audit: Failed to query backend {}: {}", backend.name(), e);
                    }
                }
            }
        }
        Ok(unmanaged)
    }
}