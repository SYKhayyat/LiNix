use crate::core::{Package, PackageSpec, Result};
use async_trait::async_trait;

/// Represents the health status of a specific backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HealthStatus {
    /// Backend is available and fully functional.
    Ok,
    /// Backend is present but requires attention (e.g. out of date, missing optional deps).
    Degraded,
    /// Backend is unusable (e.g. binary missing, network unreachable).
    Critical,
}

/// A structured report for system diagnostics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub message: Option<String>,
}

/// The base trait every backend must implement.
/// It provides access to specific capabilities via an "Interface Query" pattern.
/// This allows LiNix to treat different managers as collections of capabilities.
pub trait Backend: Send + Sync {
    /// Unique identifier for the backend (e.g., "apt", "cargo").
    fn name(&self) -> &str;

    /// Checks if the underlying tool is available on the system.
    fn is_available(&self) -> bool;

    /// Diagnostic check used by the 'Doctor' command.
    async fn check_health(&self) -> Result<HealthReport> {
        if self.is_available() {
            Ok(HealthReport { status: HealthStatus::Ok, message: None })
        } else {
            Ok(HealthReport { 
                status: HealthStatus::Critical, 
                message: Some(format!("Binary for {} not found in PATH", self.name())) 
            })
        }
    }

    // --- CAPABILITY DISCOVERY ---
    // Returns Some(&dyn Trait) if the capability is supported, otherwise None.

    fn as_installable(&self) -> Option<&dyn Installable> { None }
    fn as_searchable(&self) -> Option<&dyn Searchable> { None }
    fn as_queryable(&self) -> Option<&dyn Queryable> { None }
    fn as_upgradable(&self) -> Option<&dyn Upgradable> { None }
    fn as_repo_manager(&self) -> Option<&dyn RepoManager> { None }
}

/// Capability trait for backends that can modify the system state.
#[async_trait]
pub trait Installable: Send + Sync {
    /// Installs a set of packages. 
    /// Receives a slice of PackageSpecs to allow backends to batch requests (e.g. 'apt install pkg1 pkg2').
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()>;
    
    /// Removes a set of packages by name.
    async fn remove(&self, names: &[String], sudo: bool) -> Result<()>;
}

/// Capability trait for backends that can inspect local system state.
#[async_trait]
pub trait Queryable: Send + Sync {
    /// Returns every package installed via this backend.
    async fn list_installed(&self) -> Result<Vec<Package>>;
    
    /// Returns only packages explicitly requested by the user (non-dependencies).
    async fn list_manual(&self) -> Result<Vec<Package>>;
    
    /// Fetches rich metadata for a specific package.
    async fn info(&self, name: &str) -> Result<Option<Package>>;
}

/// Capability trait for backends that can search remote repositories.
#[async_trait]
pub trait Searchable: Send + Sync {
    /// Performs a remote query and returns matching packages.
    async fn search(&self, query: &str) -> Result<Vec<Package>>;
}

/// Capability trait for backends that support maintenance and upgrades.
#[async_trait]
pub trait Upgradable: Send + Sync {
    /// Refreshes local metadata or cache (e.g. 'apt update').
    async fn update(&self, sudo: bool) -> Result<()>;
    
    /// Upgrades all packages managed by this backend to their latest versions.
    async fn upgrade(&self, sudo: bool) -> Result<()>;
    
    /// Identifies and removes unused dependencies (orphans).
    async fn clean_orphans(&self, sudo: bool) -> Result<()> { Ok(()) }
}

/// Capability trait for backends that support source/repository management.
#[async_trait]
pub trait RepoManager: Send + Sync {
    /// Adds a new source repository (e.g. PPA, Brew Tap).
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()>;
    
    /// Removes a source repository.
    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()>;
    
    /// Lists all currently configured source repositories.
    async fn list_repos(&self) -> Result<Vec<(String, String)>>;
}