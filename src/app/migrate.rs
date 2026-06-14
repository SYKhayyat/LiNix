use crate::core::{Result, Package, Error, StateRegistry};
use crate::backends::BackendRegistry;
use crate::config::Config;
use chrono::Local;
use tracing::{info, warn, debug, trace, instrument};
use std::collections::{HashSet, HashMap};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::io::AsyncWriteExt;

/// The System Ingestion Engine.
/// 
/// The Migrator identifies components currently installed on the operating 
/// system that are not yet managed by LiNix. It generates declarative 
/// manifests for these components and acquires ownership in the StateRegistry.
pub struct Migrator {
    /// Registry for capability-based discovery across all backends.
    registry: Arc<BackendRegistry>,
    /// Shared mutable access to the mission-critical system state.
    state: Arc<Mutex<StateRegistry>>,
    /// Global application configuration.
    config: Arc<Config>,
}

impl Migrator {
    /// Initializes a new Migrator with explicit kernel dependencies.
    pub fn new(
        registry: Arc<BackendRegistry>,
        state: Arc<Mutex<StateRegistry>>,
        config: &Config,
    ) -> Self {
        Self {
            registry,
            state,
            config: Arc::new(config.clone()),
        }
    }

    /// Primary entry point: Discovery -> Manifesting -> Acquisition.
    /// 
    /// This method performs a non-destructive system crawl to identify 
    /// manual installations and bring them under LiNix control.
    #[instrument(skip(self))]
    pub async fn migrate(&self) -> Result<()> {
        info!("Migrator: Initiating automated system discovery closure.");

        let mut discovered_packages = Vec::new();
        let mut seen_keys = HashSet::new();

        // --- PHASE 1: DISCOVERY ---
        // Query every backend that supports the Queryable trait
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                debug!("Migrator: Probing backend '{}' for unmanaged components...", backend.name());
                
                // Identify packages explicitly installed by the user
                match queryable.list_manual().await {
                    Ok(pkgs) => {
                        let state_guard = self.state.lock().await;
                        for pkg in pkgs {
                            let key = format!("{}:{}", pkg.backend, pkg.name);
                            
                            // Candidate Criteria:
                            // 1. Not currently tracked in LiNix state.
                            // 2. Not already identified in this discovery cycle.
                            // 3. Not a core protected system package (sudo, kernel, etc).
                            if !state_guard.is_managed(&pkg.backend, &pkg.name) 
                               && seen_keys.insert(key.clone()) 
                               && !self.config.is_protected(&pkg.name) 
                            {
                                trace!("Migrator: Candidate identified for ingestion: {}", key);
                                discovered_packages.push(pkg);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Migrator: Backend '{}' discovery failed: {}. Continuing crawl.", backend.name(), e);
                    }
                }
            }
        }

        if discovered_packages.is_empty() {
            info!("Migrator: Discovery cycle complete. System state is already synchronized.");
            return Ok(());
        }

        info!("Migrator: Discovered {} candidates for declarative ingestion.", discovered_packages.len());

        // --- PHASE 2: MANIFEST GENERATION ---
        // Create a new .txt manifest file for the ingested components
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let filename = format!("migrated_{}.txt", timestamp);
        let manifest_path = self.config.groups_dir.join(&filename);

        info!("Migrator: Constructing declarative ingestion manifest: {:?}", manifest_path);

        let manifest_lines: Vec<String> = discovered_packages.iter()
            .map(|p| format!("{}:{}", p.backend, p.name))
            .collect();

        // Ensure manifest destination directory exists asynchronously
        if let Some(parent) = manifest_path.parent() {
            if !tokio::fs::try_exists(parent).await.unwrap_or(false) {
                tokio::fs::create_dir_all(parent).await.map_err(Error::from)?;
            }
        }

        // Atomically create and write the manifest file
        let mut file = tokio::fs::File::create(&manifest_path).await.map_err(Error::from)?;
        let header = format!(
            "# LiNix Ingestion Manifest\n# Timestamp: {}\n# Origin: Automated Migration\n\n", 
            Local::now()
        );
        
        file.write_all(header.as_bytes()).await?;
        file.write_all(manifest_lines.join("\n").as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;

        // --- PHASE 3: STATE ACQUISITION ---
        // Finalize ownership by updating the StateRegistry with source metadata
        {
            let mut state_mut = self.state.lock().await;
            // Feature 3: The filename serves as the source origin for these packages
            let source_meta = Some(format!("manifest:{}", filename));

            for pkg in &discovered_packages {
                // A+ Hardening: Provide all 6 arguments to modernized state.add
                state_mut.add(
                    &pkg.backend, 
                    &pkg.name, 
                    pkg.version.clone(), 
                    HashMap::new(), // Default options for ingested packages
                    source_meta.clone(), 
                    false // Ingested packages are permanent (non-transient)
                );
            }
            
            // Persist ownership records to disk (Offloaded to dedicated task)
            let state_to_persist = state_mut.clone();
            tokio::task::spawn_blocking(move || {
                state_to_persist.save()
            }).await.map_err(|e| Error::Other(format!("State-save thread failure: {}", e)))??;
        }

        info!("Migrator: State registry aligned. Migration successful.");
        
        println!("\nIngestion Complete!");
        println!("{:-<60}", "");
        println!("Manifest Created:  {}", manifest_path.display());
        println!("Packages Ingested: {}", discovered_packages.len());
        println!("{:-<60}", "");
        println!("Success: Discovered components are now managed declaratively by LiNix.");

        Ok(())
    }

    /// Performs a destructive Discovery cycle without generating files or 
    /// acquiring state.
    /// 
    /// Used by the CLI to show users what LiNix *would* ingest.
    pub async fn audit(&self) -> Result<Vec<Package>> {
        let mut unmanaged = Vec::new();
        let mut seen = HashSet::new();
        let state_guard = self.state.lock().await;

        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(pkgs) = queryable.list_manual().await {
                    for pkg in pkgs {
                        let key = format!("{}:{}", pkg.backend, pkg.name);
                        if !state_guard.is_managed(backend.name(), &pkg.name) 
                           && seen.insert(key)
                           && !self.config.is_protected(&pkg.name)
                        {
                            unmanaged.push(pkg);
                        }
                    }
                }
            }
        }
        Ok(unmanaged)
    }
}