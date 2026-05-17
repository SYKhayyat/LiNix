use crate::core::{Package, PackageSpec, Result};
use async_trait::async_trait;
use std::any::Any;
use std::sync::Arc;

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

// ============================================================================
// FIX #7: Interface Segregation Principle - Separate capability traits
// Backends now implement ONLY the traits they support, not a monolithic trait
// with optional methods that return None.
// ============================================================================

/// Core trait that every backend must implement.
/// Contains only the essential methods that all backends need.
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
    
    /// Checks if the backend supports repository management.
    /// Override to return false if repo management is not supported.
    fn can_manage_repos(&self) -> bool { true }
}

// ============================================================================
// Capability aggregation struct for type-safe capability handling
// ============================================================================

/// Represents a backend with its supported capabilities.
/// This allows runtime discovery of capabilities without Option<&dyn Trait>.
pub struct BackendCapabilities {
    core: Arc<dyn BackendCore>,
    installable: Option<Arc<dyn Installable>>,
    searchable: Option<Arc<dyn Searchable>>,
    queryable: Option<Arc<dyn Queryable>>,
    upgradable: Option<Arc<dyn Upgradable>>,
    repo_manager: Option<Arc<dyn RepoManager>>,
}

impl BackendCapabilities {
    /// Creates a new BackendCapabilities builder.
    pub fn builder(core: Arc<dyn BackendCore>) -> BackendCapabilitiesBuilder {
        BackendCapabilitiesBuilder::new(core)
    }
    
    /// Gets the core backend reference.
    pub fn core(&self) -> &Arc<dyn BackendCore> {
        &self.core
    }
    
    /// Returns true if this backend supports installation.
    pub fn is_installable(&self) -> bool {
        self.installable.is_some()
    }
    
    /// Gets the installable capability if available.
    pub fn as_installable(&self) -> Option<&Arc<dyn Installable>> {
        self.installable.as_ref()
    }
    
    /// Returns true if this backend supports searching.
    pub fn is_searchable(&self) -> bool {
        self.searchable.is_some()
    }
    
    /// Gets the searchable capability if available.
    pub fn as_searchable(&self) -> Option<&Arc<dyn Searchable>> {
        self.searchable.as_ref()
    }
    
    /// Returns true if this backend supports querying.
    pub fn is_queryable(&self) -> bool {
        self.queryable.is_some()
    }
    
    /// Gets the queryable capability if available.
    pub fn as_queryable(&self) -> Option<&Arc<dyn Queryable>> {
        self.queryable.as_ref()
    }
    
    /// Returns true if this backend supports upgrades.
    pub fn is_upgradable(&self) -> bool {
        self.upgradable.is_some()
    }
    
    /// Gets the upgradable capability if available.
    pub fn as_upgradable(&self) -> Option<&Arc<dyn Upgradable>> {
        self.upgradable.as_ref()
    }
    
    /// Returns true if this backend supports repository management.
    pub fn is_repo_manager(&self) -> bool {
        self.repo_manager.is_some()
    }
    
    /// Gets the repo manager capability if available.
    pub fn as_repo_manager(&self) -> Option<&Arc<dyn RepoManager>> {
        self.repo_manager.as_ref()
    }
}

/// Builder for BackendCapabilities.
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
    
    pub fn with_installable(mut self, installable: Arc<dyn Installable>) -> Self {
        self.installable = Some(installable);
        self
    }
    
    pub fn with_searchable(mut self, searchable: Arc<dyn Searchable>) -> Self {
        self.searchable = Some(searchable);
        self
    }
    
    pub fn with_queryable(mut self, queryable: Arc<dyn Queryable>) -> Self {
        self.queryable = Some(queryable);
        self
    }
    
    pub fn with_upgradable(mut self, upgradable: Arc<dyn Upgradable>) -> Self {
        self.upgradable = Some(upgradable);
        self
    }
    
    pub fn with_repo_manager(mut self, repo_manager: Arc<dyn RepoManager>) -> Self {
        self.repo_manager = Some(repo_manager);
        self
    }
    
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

// ============================================================================
// Legacy compatibility - Deprecated but kept for transition
// ============================================================================

/// Legacy Backend trait - DEPRECATED.
/// Use BackendCore + individual capability traits instead.
#[deprecated(since = "3.5.0", note = "Use BackendCore and separate capability traits")]
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    fn is_available(&self) -> bool;
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
    
    fn as_installable(&self) -> Option<&dyn Installable> { None }
    fn as_searchable(&self) -> Option<&dyn Searchable> { None }
    fn as_queryable(&self) -> Option<&dyn Queryable> { None }
    fn as_upgradable(&self) -> Option<&dyn Upgradable> { None }
    fn as_repo_manager(&self) -> Option<&dyn RepoManager> { None }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    
    struct MockBackendCore;
    
    impl BackendCore for MockBackendCore {
        fn name(&self) -> &str {
            "mock"
        }
        
        fn is_available(&self) -> bool {
            true
        }
    }
    
    struct MockInstallable;
    
    #[async_trait]
    impl Installable for MockInstallable {
        async fn install(&self, _specs: &[PackageSpec], _sudo: bool) -> Result<()> {
            Ok(())
        }
        
        async fn remove(&self, _names: &[String], _sudo: bool) -> Result<()> {
            Ok(())
        }
    }
    
    struct MockSearchable;
    
    #[async_trait]
    impl Searchable for MockSearchable {
        async fn search(&self, query: &str) -> Result<Vec<Package>> {
            if query == "existing-pkg" {
                Ok(vec![Package::new("existing-pkg", "mock")])
            } else {
                Ok(vec![])
            }
        }
    }
    
    struct MockQueryable;
    
    #[async_trait]
    impl Queryable for MockQueryable {
        async fn list_installed(&self) -> Result<Vec<Package>> {
            Ok(vec![Package::new("installed-pkg", "mock")])
        }
        
        async fn list_manual(&self) -> Result<Vec<Package>> {
            Ok(vec![Package::new("manual-pkg", "mock")])
        }
        
        async fn info(&self, name: &str) -> Result<Option<Package>> {
            if name == "existing-pkg" {
                Ok(Some(Package::new(name, "mock")))
            } else {
                Ok(None)
            }
        }
    }
    
    struct MockUpgradable;
    
    #[async_trait]
    impl Upgradable for MockUpgradable {
        async fn update(&self, _sudo: bool) -> Result<()> {
            Ok(())
        }
        
        async fn upgrade(&self, _sudo: bool) -> Result<()> {
            Ok(())
        }
        
        async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
            Ok(())
        }
    }
    
    struct MockRepoManager;
    
    #[async_trait]
    impl RepoManager for MockRepoManager {
        async fn add_repo(&self, _name: &str, _url: &str, _sudo: bool) -> Result<()> {
            Ok(())
        }
        
        async fn remove_repo(&self, _name: &str, _sudo: bool) -> Result<()> {
            Ok(())
        }
        
        async fn list_repos(&self) -> Result<Vec<(String, String)>> {
            Ok(vec![("test-repo".to_string(), "https://test.com".to_string())])
        }
    }
    
    #[test]
    fn test_backend_capabilities_builder() {
        let core = Arc::new(MockBackendCore);
        let installable = Arc::new(MockInstallable);
        let searchable = Arc::new(MockSearchable);
        let queryable = Arc::new(MockQueryable);
        let upgradable = Arc::new(MockUpgradable);
        let repo_manager = Arc::new(MockRepoManager);
        
        let backend = BackendCapabilities::builder(core.clone())
            .with_installable(installable)
            .with_searchable(searchable)
            .with_queryable(queryable)
            .with_upgradable(upgradable)
            .with_repo_manager(repo_manager)
            .build();
        
        assert_eq!(backend.core().name(), "mock");
        assert!(backend.is_installable());
        assert!(backend.as_installable().is_some());
        assert!(backend.is_searchable());
        assert!(backend.as_searchable().is_some());
        assert!(backend.is_queryable());
        assert!(backend.as_queryable().is_some());
        assert!(backend.is_upgradable());
        assert!(backend.as_upgradable().is_some());
        assert!(backend.is_repo_manager());
        assert!(backend.as_repo_manager().is_some());
    }
    
    #[test]
    fn test_backend_capabilities_no_capabilities() {
        let core = Arc::new(MockBackendCore);
        let backend = BackendCapabilities::builder(core).build();
        
        assert!(!backend.is_installable());
        assert!(!backend.is_searchable());
        assert!(!backend.is_queryable());
        assert!(!backend.is_upgradable());
        assert!(!backend.is_repo_manager());
    }
    
    #[tokio::test]
    async fn test_searchable_remote_has() {
        let searchable = MockSearchable;
        
        assert!(searchable.remote_has("existing-pkg").await.unwrap());
        assert!(!searchable.remote_has("nonexistent-pkg").await.unwrap());
    }
    
    #[tokio::test]
    async fn test_searchable_remote_info() {
        let searchable = MockSearchable;
        
        let info = searchable.remote_info("existing-pkg").await.unwrap();
        assert!(info.is_some());
        assert_eq!(info.unwrap().name, "existing-pkg");
        
        let info = searchable.remote_info("nonexistent-pkg").await.unwrap();
        assert!(info.is_none());
    }
}