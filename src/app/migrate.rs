use crate::App;
use crate::core::{Result, Package, Error};
use crate::config::parser::write_group_file;
use chrono::Local;
use tracing::{info, warn, debug};
use std::collections::HashSet;

/// The Ingestion Engine (Roadmap Point 3).
/// Responsible for moving a "dirty" system with existing manual installs into LiNix management.
/// This fulfills the requirement to "migrate your system easily into linix."
pub struct Migrator<'a> {
    app: &'a App,
}

impl<'a> Migrator<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Primary migration logic.
    /// 
    /// 1. Discovery: Crawls all available backends to find packages marked as "manually installed" 
    ///    by the underlying manager (e.g., 'apt-mark showmanual' or 'brew leaves').
    /// 2. Deduplication: Filters out packages that are already managed by LiNix in registry.json.
    /// 3. Persistence: Generates a new declarative group file (e.g. groups/migration_2023.txt) 
    ///    so the user has a starting point for their configuration.
    /// 4. Ownership: Adds discovered packages to the LiNix State Registry so that the next 'sync' 
    ///    recognizes them as managed state rather than "drift" to be removed.
    pub async fn migrate(&self) -> Result<()> {
        info!("Migrator: Initiating full system discovery...");

        let mut discovered_packages = Vec::new();
        let mut seen = HashSet::new();

        // 1. Discovery Phase: Crawl the 33-backend registry
        for backend in self.app.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                debug!("Migrator: Probing {}...", backend.name());
                match queryable.list_manual().await {
                    Ok(pkgs) => {
                        for pkg in pkgs {
                            // Construct the unique SOLID key: backend:name
                            let key = format!("{}:{}", pkg.backend, pkg.name);
                            
                            // Only migrate if not already managed and not a duplicate from another backend
                            if !self.app.state.is_managed(&pkg.backend, &pkg.name) && seen.insert(key) {
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
            info!("Migrator: No unmanaged manual packages found. Your system is already fully managed by LiNix.");
            return Ok(());
        }

        info!("Migrator: Found {} unmanaged packages across system backends.", discovered_packages.len());

        // 2. Manifest Generation Phase
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let filename = format!("migrated_{}.txt", timestamp);
        let path = self.app.config.groups_dir.join(&filename);

        let package_strings: Vec<String> = discovered_packages.iter()
            .map(|p| format!("{}:{}", p.backend, p.name))
            .collect();

        // Write to the groups directory so it's picked up by the next sync
        write_group_file(&path, &package_strings)?;
        info!("Migrator: Declarative manifest generated: {:?}", path);

        // 3. Ownership Acquisition Phase
        // We update the state registry immediately to take "logical ownership" of these packages.
        let mut state = self.app.state.clone();
        for pkg in &discovered_packages {
            state.add(&pkg.backend, &pkg.name, pkg.version.clone());
        }
        state.save()?;

        info!("Migrator: Ownership records updated. System is now consistent.");
        
        println!("\nMigration Complete!");
        println!("{:-<40}", "");
        println!("Target Manifest:   {}", path.display());
        println!("Packages Ingested: {}", discovered_packages.len());
        println!("{:-<40}", "");
        println!("Note: You should now review {} and move packages into specific group files.", filename);

        Ok(())
    }

    /// Performs a "Dry-Run" migration to show the user what would be added to their config.
    pub async fn audit(&self) -> Result<Vec<Package>> {
        let mut unmanaged = Vec::new();
        for backend in self.app.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                let pkgs = queryable.list_manual().await?;
                for pkg in pkgs {
                    if !self.app.state.is_managed(backend.name(), &pkg.name) {
                        unmanaged.push(pkg);
                    }
                }
            }
        }
        Ok(unmanaged)
    }
}