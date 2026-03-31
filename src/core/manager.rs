use crate::core::{Package, PackageSpec, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Represents the high-level health of a package manager backend
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HealthStatus { 
    Ok, 
    Warning, 
    Error 
}

/// Detailed health report for the 'doctor' command
pub struct HealthReport {
    pub status: HealthStatus,
    pub message: Option<String>,
}

#[async_trait]
pub trait PackageManager: Send + Sync {
    /// The unique identifier for this manager (e.g., "apt", "brew")
    fn name(&self) -> &str;

    /// Checks if the underlying CLI tool is installed and reachable in PATH
    fn is_available(&self) -> bool;

    /// Basic installation of a list of package names
    async fn install(&self, packages: &[String], sudo: bool) -> Result<()>;

    /// Uninstallation of a list of package names
    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()>;

    /// Returns a list of every package currently managed by this backend
    async fn list_installed(&self) -> Result<Vec<Package>>;

    /// FEATURE: Smart Export
    /// Returns only packages explicitly installed by the user, skipping 
    /// automatic dependencies (e.g., 'apt-mark showmanual' logic).
    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    /// Refresh the remote package database
    async fn update(&self, _sudo: bool) -> Result<()> { 
        Ok(()) 
    }

    /// Upgrade all currently installed packages to their latest versions
    async fn upgrade(&self, _sudo: bool) -> Result<()> { 
        Ok(()) 
    }

    /// Search for a package string in the remote repository
    async fn search(&self, _query: &str) -> Result<Vec<Package>> { 
        Ok(vec![]) 
    }

    /// Clean up cached files or orphaned dependencies
    async fn clean_orphans(&self, _sudo: bool) -> Result<()> { 
        Ok(()) 
    }

    /// Whether this manager supports a specific 'autoremove' or 'cleanup' command
    fn supports_orphan_cleanup(&self) -> bool { 
        false 
    }

    /// Fetch metadata for a specific package name
    async fn info(&self, _package: &str) -> Result<Option<Package>> { 
        Ok(None) 
    }

    /// High-level installation that handles versioning and custom options
    async fn install_with_options(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        let packages: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        self.install(&packages, sudo).await
    }

    // --- Repository Management ---

    /// Add a new source/PPA/channel
    async fn add_repo(&self, name: &str, url: &str, _sudo: bool) -> Result<()> {
        Err(crate::core::Error::UnsupportedPlatform(format!(
            "Backend '{}' does not support adding repositories (tried {} -> {})", 
            self.name(), name, url
        )))
    }

    /// Remove an existing source/PPA/channel
    async fn remove_repo(&self, name: &str, _sudo: bool) -> Result<()> {
        Err(crate::core::Error::UnsupportedPlatform(format!(
            "Backend '{}' does not support removing repositories ({})", 
            self.name(), name
        )))
    }

    /// List all configured sources
    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        Ok(vec![])
    }
    
    // --- System Diagnosis ---

    /// Per-backend health check for the 'doctor' command
    async fn check_health(&self) -> Result<HealthReport> {
        if self.is_available() {
            Ok(HealthReport { 
                status: HealthStatus::Ok, 
                message: None 
            })
        } else {
            Ok(HealthReport { 
                status: HealthStatus::Error, 
                message: Some(format!(
                    "The underlying CLI tool for '{}' is missing from PATH. Check your system installation.", 
                    self.name()
                )) 
            })
        }
    }
}