use crate::app::{LuaHooks, MetricsCollector};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::config::parser::load_all_packages;
use crate::core::{CommandExecutor, PackageCache, Result, Error};
use crate::utils::progress::ProgressReporter;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{info, warn};

/// Engine for syncing packages
pub struct SyncEngine<'a> {
    config: &'a Config,
    registry: &'a BackendRegistry,
    #[allow(dead_code)]
    executor: &'a CommandExecutor,
    #[allow(dead_code)]
    cache: &'a PackageCache,
    #[allow(dead_code)]
    metrics: &'a MetricsCollector,
    progress: &'a dyn ProgressReporter,
    hooks: &'a LuaHooks,
}

/// Changes to be applied
#[derive(Debug, Default)]
pub struct SyncChanges {
    pub to_install: HashMap<String, Vec<String>>,
    pub to_remove: HashMap<String, Vec<String>>,
}

impl SyncChanges {
    pub fn is_empty(&self) -> bool {
        self.to_install.values().all(|v| v.is_empty())
            && self.to_remove.values().all(|v| v.is_empty())
    }

    pub fn total_install(&self) -> usize {
        self.to_install.values().map(|v| v.len()).sum()
    }

    pub fn total_remove(&self) -> usize {
        self.to_remove.values().map(|v| v.len()).sum()
    }
}

impl<'a> SyncEngine<'a> {
    pub fn new(
        config: &'a Config,
        registry: &'a BackendRegistry,
        executor: &'a CommandExecutor,
        cache: &'a PackageCache,
        metrics: &'a MetricsCollector,
        progress: &'a dyn ProgressReporter,
        hooks: &'a LuaHooks,
    ) -> Self {
        Self {
            config,
            registry,
            executor,
            cache,
            metrics,
            progress,
            hooks,
        }
    }

    /// Load desired packages from config
    pub fn load_desired_packages(&self) -> Result<HashMap<String, HashSet<String>>> {
        let mut packages_by_backend: HashMap<String, HashSet<String>> = HashMap::new();

        // Load from groups directory
        let all_packages = load_all_packages(&self.config.groups_dir)?;

        // Parse packages into backends
        // Format: backend:package or just package (default to system backend)
        for package in all_packages {
            let (backend, pkg_name) = if package.contains(':') {
                let parts: Vec<&str> = package.splitn(2, ':').collect();
                (parts[0].to_string(), parts[1].to_string())
            } else {
                // Detect appropriate backend
                let backend = self.detect_backend(&package);
                (backend, package)
            };

            packages_by_backend
                .entry(backend)
                .or_default()
                .insert(pkg_name);
        }

        // Add hostname-specific packages
        let hostname_packages = self.config.get_hostname_packages();
        for package in hostname_packages {
            let (backend, pkg_name) = if package.contains(':') {
                let parts: Vec<&str> = package.splitn(2, ':').collect();
                (parts[0].to_string(), parts[1].to_string())
            } else {
                let backend = self.detect_backend(&package);
                (backend, package)
            };

            packages_by_backend
                .entry(backend)
                .or_default()
                .insert(pkg_name);
        }

        Ok(packages_by_backend)
    }

    /// Detect appropriate backend for a package
    fn detect_backend(&self, package: &str) -> String {
        // Check for GitHub URLs
        if package.starts_with("github:") || package.contains("github.com") {
            return "github".to_string();
        }

        // Check for flatpak refs
        if package.contains('.') && package.matches('.').count() >= 2 {
            // Could be a flatpak app ID like com.spotify.Client
            if self
                .registry
                .get("flatpak")
                .map(|m| m.is_available())
                .unwrap_or(false)
            {
                return "flatpak".to_string();
            }
        }

        // Default to system package manager
        self.detect_system_backend()
    }

    /// Detect the system package manager
    fn detect_system_backend(&self) -> String {
        let system_backends = ["apt", "dnf", "pacman", "zypper", "apk"];

        for backend in system_backends {
            if let Some(manager) = self.registry.get(backend) {
                if manager.is_available() {
                    return backend.to_string();
                }
            }
        }

        // Fallback
        "apt".to_string()
    }

    /// Calculate changes needed
    pub async fn calculate_changes(&self) -> Result<SyncChanges> {
        let desired = self.load_desired_packages()?;
        let mut changes = SyncChanges::default();

        let managers = if self.config.enabled_backends.is_empty() {
            self.registry.available()
        } else {
            self.registry.get_filtered(&self.config.enabled_backends)
        };

        for manager in managers {
            let backend = manager.name().to_string();

            // Get desired packages for this backend
            let desired_for_backend = desired.get(&backend).cloned().unwrap_or_default();

            if desired_for_backend.is_empty() {
                continue;
            }

            // Get installed packages
            let installed = match manager.list_installed().await {
                Ok(pkgs) => pkgs.into_iter().map(|p| p.name).collect::<HashSet<_>>(),
                Err(e) => {
                    warn!("Failed to list installed packages for {}: {}", backend, e);
                    continue;
                }
            };

            // Calculate to_install (desired - installed)
            let to_install: Vec<String> = desired_for_backend
                .difference(&installed)
                .cloned()
                .collect();

            if !to_install.is_empty() {
                changes.to_install.insert(backend.clone(), to_install);
            }
        }

        // Load bloatware to remove
        if self.config.remove_bloatware && self.config.bloatware_file.exists() {
            let bloatware =
                crate::config::parser::parse_bloatware_file(&self.config.bloatware_file)?;

            for manager in self.registry.available() {
                let backend = manager.name().to_string();

                let installed = match manager.list_installed().await {
                    Ok(pkgs) => pkgs.into_iter().map(|p| p.name).collect::<HashSet<_>>(),
                    Err(_) => continue,
                };

                let bloatware_set: HashSet<_> = bloatware.iter().cloned().collect();
                let to_remove: Vec<String> =
                    installed.intersection(&bloatware_set).cloned().collect();

                if !to_remove.is_empty() {
                    changes
                        .to_remove
                        .entry(backend)
                        .or_default()
                        .extend(to_remove);
                }
            }
        }

        Ok(changes)
    }

    /// Execute sync operation
    pub async fn sync(&self) -> Result<()> {
        let changes = self.calculate_changes().await?;

        if changes.is_empty() {
            info!("System is in sync, no changes needed");
            return Ok(());
        }

        // Display changes
        self.display_changes(&changes);

        // Confirm
        if !self.config.yes && !self.confirm_changes()? {
            return Err(Error::Cancelled);
        }

        // Execute changes
        self.execute_changes(&changes).await?;

        Ok(())
    }

    /// Display pending changes
    fn display_changes(&self, changes: &SyncChanges) {
        println!("\n=== Sync Changes ===\n");

        if !changes.to_install.is_empty() {
            println!("Packages to INSTALL ({}):", changes.total_install());
            for (backend, packages) in &changes.to_install {
                for pkg in packages {
                    println!("  + [{}] {}", backend, pkg);
                }
            }
            println!();
        }

        if !changes.to_remove.is_empty() {
            println!("Packages to REMOVE ({}):", changes.total_remove());
            for (backend, packages) in &changes.to_remove {
                for pkg in packages {
                    println!("  - [{}] {}", backend, pkg);
                }
            }
            println!();
        }
    }

    /// Confirm changes with user
    fn confirm_changes(&self) -> Result<bool> {
        print!("Proceed with changes? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        Ok(input.trim().to_lowercase() == "y" || input.trim().to_lowercase() == "yes")
    }

    /// Execute the changes
    async fn execute_changes(&self, changes: &SyncChanges) -> Result<()> {
    let _semaphore = Arc::new(Semaphore::new(self.config.max_parallel));

        // Install packages
        for (backend, packages) in &changes.to_install {
            if let Some(manager) = self.registry.get(backend) {
                let progress = self.progress.start(
                    packages.len() as u64,
                    &format!("Installing via {}", backend),
                );

                // Run pre-install hooks
                for pkg in packages {
                    self.hooks.run_hook("before_install", pkg).await?;
                }

                // Install
                manager.install(packages, true).await?;

                // Run post-install hooks
                for pkg in packages {
                    self.hooks.run_hook("after_install", pkg).await?;
                    progress.inc(1);
                }

                progress.finish_with_message(&format!(
                    "Installed {} packages via {}",
                    packages.len(),
                    backend
                ));
            }
        }

        // Remove packages
        for (backend, packages) in &changes.to_remove {
            if let Some(manager) = self.registry.get(backend) {
                let progress = self
                    .progress
                    .start(packages.len() as u64, &format!("Removing via {}", backend));

                // Run pre-remove hooks
                for pkg in packages {
                    self.hooks.run_hook("before_remove", pkg).await?;
                }

                // Remove
                manager.remove(packages, true).await?;

                // Run post-remove hooks
                for pkg in packages {
                    self.hooks.run_hook("after_remove", pkg).await?;
                    progress.inc(1);
                }

                progress.finish_with_message(&format!(
                    "Removed {} packages via {}",
                    packages.len(),
                    backend
                ));
            }
        }

        Ok(())
    }

    /// Find unmanaged packages
    pub async fn find_unmanaged(&self) -> Result<Vec<(String, Vec<String>)>> {
        let desired = self.load_desired_packages()?;
        let mut unmanaged = Vec::new();

        for manager in self.registry.available() {
            let backend = manager.name().to_string();

            let desired_for_backend = desired.get(&backend).cloned().unwrap_or_default();

            let installed = match manager.list_installed().await {
                Ok(pkgs) => pkgs.into_iter().map(|p| p.name).collect::<HashSet<_>>(),
                Err(e) => {
                    warn!("Failed to list installed packages for {}: {}", backend, e);
                    continue;
                }
            };

            // Unmanaged = installed - desired
            let unmanaged_pkgs: Vec<String> = installed
                .difference(&desired_for_backend)
                .cloned()
                .collect();

            if !unmanaged_pkgs.is_empty() {
                unmanaged.push((backend, unmanaged_pkgs));
            }
        }

        Ok(unmanaged)
    }

    /// Clean unmanaged packages
    pub async fn clean(&self) -> Result<()> {
        let unmanaged = self.find_unmanaged().await?;

        if unmanaged.is_empty() {
            info!("No unmanaged packages found");
            return Ok(());
        }

        // Display
        println!("\n=== Unmanaged Packages to Remove ===\n");
        for (backend, packages) in &unmanaged {
            for pkg in packages {
                println!("  - [{}] {}", backend, pkg);
            }
        }
        println!();

        // Confirm
        if !self.config.yes && !self.confirm_changes()? {
            return Err(Error::Cancelled);
        }

        // Remove
        for (backend, packages) in unmanaged {
            if let Some(manager) = self.registry.get(&backend) {
                manager.remove(&packages, true).await?;
            }
        }

        Ok(())
    }
}
