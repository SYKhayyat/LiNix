use crate::App;
use crate::core::{Result, Package};
use crate::config::parser::write_group_file;
use chrono::Local;
use tracing::{info, warn, debug};
use std::collections::HashSet;

/// The Ingestion Engine (Roadmap Point 3).
/// Responsible for moving a "dirty" system with existing manual installs into LiNix management.
pub struct Migrator<'a> {
    app: &'a App,
}

impl<'a> Migrator<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Primary migration logic.
    pub async fn migrate(&self) -> Result<()> {
        info!("Migrator: Initiating full system discovery...");

        let mut discovered_packages = Vec::new();
        let mut seen = HashSet::new();

        // Acquire lock once for the discovery phase
        let state = self.app.state.lock().await;

        // 1. Discovery Phase: Crawl the registry
        for backend in self.app.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                debug!("Migrator: Probing {}...", backend.name());
                match queryable.list_manual().await {
                    Ok(pkgs) => {
                        for pkg in pkgs {
                            let key = format!("{}:{}", pkg.backend, pkg.name);
                            
                            // Only migrate if not already managed
                            if !state.is_managed(&pkg.backend, &pkg.name) && seen.insert(key) {
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
        let path = self.app.config.groups_dir.join(&filename);

        let package_strings: Vec<String> = discovered_packages.iter()
            .map(|p| format!("{}:{}", p.backend, p.name))
            .collect();

        write_group_file(&path, &package_strings)?;
        info!("Migrator: Declarative manifest generated: {:?}", path);

        // 3. Ownership Acquisition Phase
        // Drop the immutable guard and get a mutable one for updating
        drop(state);
        let mut state_mut = self.app.state.lock().await;
        
        for pkg in &discovered_packages {
            state_mut.add_simple(&pkg.backend, &pkg.name, pkg.version.clone());
        }
        state_mut.save()?;

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
        let state = self.app.state.lock().await;

        for backend in self.app.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                let pkgs = queryable.list_manual().await?;
                for pkg in pkgs {
                    if !state.is_managed(backend.name(), &pkg.name) {
                        unmanaged.push(pkg);
                    }
                }
            }
        }
        Ok(unmanaged)
    }
}