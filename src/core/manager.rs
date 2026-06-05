use crate::core::{Package, PackageSpec, Result};
use async_trait::async_trait;
use std::sync::Arc;
use serde::{Deserialize, Serialize};

/// Represents the health status of a specific backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Backend is available and fully functional.
    Ok,
    /// Backend is present but requires attention (e.g. out of date, missing optional deps).
    Degraded,
    /// Backend is unusable (e.g. binary missing, network unreachable).
    Critical,
}

/// A structured report for system diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub message: Option<String>,
}

// ============================================================================
// Capability Traits (ISP-Compliant)
// ============================================================================

/// Core trait that every backend must implement.
/// Wrapped in async_trait to ensure dyn-compatibility for trait objects.
#[async_trait]
pub trait BackendCore: Send + Sync {
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
}

/// Capability trait for backends that can modify the system state.
#[async_trait]
pub trait Installable: Send + Sync {
    /// Installs a set of packages. 
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
    
    /// Checks if a specific package exists in remote repositories.
    async fn remote_has(&self, name: &str) -> Result<bool> {
        let results = self.search(name).await?;
        Ok(results.iter().any(|p| p.name == name))
    }
    
    /// Gets detailed remote information for a package.
    async fn remote_info(&self, name: &str) -> Result<Option<Package>> {
        let results = self.search(name).await?;
        Ok(results.into_iter().find(|p| p.name == name))
    }
}

/// Capability trait for backends that support maintenance and upgrades.
#[async_trait]
pub trait Upgradable: Send + Sync {
    /// Refreshes local metadata or cache (e.g. 'apt update').
    async fn update(&self, sudo: bool) -> Result<()>;
    
    /// Upgrades all packages managed by this backend to their latest versions.
    async fn upgrade(&self, sudo: bool) -> Result<()>;

    /// identifies and removes unused dependencies.
    async fn clean_orphans(&self, sudo: bool) -> Result<()>;
}

/// Capability trait for backends that support source/repository management.
#[async_trait]
pub trait RepoManager: Send + Sync {
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()>;
    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()>;
    async fn list_repos(&self) -> Result<Vec<(String, String)>>;
    fn can_manage_repos(&self) -> bool { true }
}

// ============================================================================
// Capability Aggregation
// ============================================================================

/// Container for a backend and its optional capabilities.
pub struct BackendCapabilities {
    core: Arc<dyn BackendCore>,
    installable: Option<Arc<dyn Installable>>,
    searchable: Option<Arc<dyn Searchable>>,
    queryable: Option<Arc<dyn Queryable>>,
    upgradable: Option<Arc<dyn Upgradable>>,
    repo_manager: Option<Arc<dyn RepoManager>>,
}

impl BackendCapabilities {
    pub fn builder(core: Arc<dyn BackendCore>) -> BackendCapabilitiesBuilder {
        BackendCapabilitiesBuilder::new(core)
    }

    pub fn core(&self) -> &Arc<dyn BackendCore> { &self.core }
    pub fn name(&self) -> &str { self.core.name() }
    pub fn is_available(&self) -> bool { self.core.is_available() }

    pub fn is_installable(&self) -> bool { self.installable.is_some() }
    pub fn as_installable(&self) -> Option<&Arc<dyn Installable>> { self.installable.as_ref() }

    pub fn is_searchable(&self) -> bool { self.searchable.is_some() }
    pub fn as_searchable(&self) -> Option<&Arc<dyn Searchable>> { self.searchable.as_ref() }

    pub fn is_queryable(&self) -> bool { self.queryable.is_some() }
    pub fn as_queryable(&self) -> Option<&Arc<dyn Queryable>> { self.queryable.as_ref() }

    pub fn is_upgradable(&self) -> bool { self.upgradable.is_some() }
    pub fn as_upgradable(&self) -> Option<&Arc<dyn Upgradable>> { self.upgradable.as_ref() }

    pub fn is_repo_manager(&self) -> bool { self.repo_manager.is_some() }
    pub fn as_repo_manager(&self) -> Option<&Arc<dyn RepoManager>> { self.repo_manager.as_ref() }
}

pub struct BackendCapabilitiesBuilder {
    core: Arc<dyn BackendCore>,
    installable: Option<Arc<dyn Installable>>,
    searchable: Option<Arc<dyn Searchable>>,
    queryable: Option<Arc<dyn Queryable>>,
    upgradable: Option<Arc<dyn Upgradable>>,
    repo_manager: Option<Arc<dyn RepoManager>>,
}

impl BackendCapabilitiesBuilder {
    fn new(core: Arc<dyn BackendCore>) -> Self {
        Self {
            core,
            installable: None,
            searchable: None,
            queryable: None,
            upgradable: None,
            repo_manager: None,
        }
    }
    
    pub fn with_installable(mut self, i: Arc<dyn Installable>) -> Self { self.installable = Some(i); self }
    pub fn with_searchable(mut self, s: Arc<dyn Searchable>) -> Self { self.searchable = Some(s); self }
    pub fn with_queryable(mut self, q: Arc<dyn Queryable>) -> Self { self.queryable = Some(q); self }
    pub fn with_upgradable(mut self, u: Arc<dyn Upgradable>) -> Self { self.upgradable = Some(u); self }
    pub fn with_repo_manager(mut self, r: Arc<dyn RepoManager>) -> Self { self.repo_manager = Some(r); self }
    
    pub fn build(self) -> BackendCapabilities {
        BackendCapabilities {
            core: self.core,
            installable: self.installable,
            searchable: self.searchable,
            queryable: self.queryable,
            upgradable: self.upgradable,
            repo_manager: self.repo_manager,
        }
    }
}

/// Legacy Backend trait - kept for compatibility.
#[async_trait]
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    fn is_available(&self) -> bool;
    async fn check_health(&self) -> Result<HealthReport>;
    fn as_installable(&self) -> Option<&dyn Installable>;
    fn as_searchable(&self) -> Option<&dyn Searchable>;
    fn as_queryable(&self) -> Option<&dyn Queryable>;
    fn as_upgradable(&self) -> Option<&dyn Upgradable>;
    fn as_repo_manager(&self) -> Option<&dyn RepoManager>;
}