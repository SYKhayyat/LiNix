use crate::core::{Package, PackageSpec, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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

/// A structured report for system diagnostics used by the 'Doctor' command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub message: Option<String>,
}

// ============================================================================
// Capability Traits (ISP-Compliant)
// ============================================================================

/// Core trait that every package management backend must implement.
///
/// This trait defines the identity and system-level availability of the backend.
#[async_trait]
pub trait BackendCore: Send + Sync {
    /// Returns the unique identifier for the backend (e.g., "apt", "cargo").
    fn name(&self) -> &str;

    /// Checks if the underlying tool binary is available in the system PATH.
    fn is_available(&self) -> bool;

    /// Phase 2.2: Returns true if the backend requires root/sudo privileges
    /// for modification (Install/Remove/Upgrade).
    ///
    /// System managers (apt, dnf) return true.
    /// User managers (cargo, npm, scoop) return false.
    fn needs_root(&self) -> bool;

    /// Performs a diagnostic check to verify if the backend is healthy.
    async fn check_health(&self) -> Result<HealthReport> {
        if self.is_available() {
            Ok(HealthReport {
                status: HealthStatus::Ok,
                message: None,
            })
        } else {
            Ok(HealthReport {
                status: HealthStatus::Critical,
                message: Some(format!("Binary for {} not found in PATH", self.name())),
            })
        }
    }
}

/// Capability trait for backends that can modify the system state (Write Access).
#[async_trait]
pub trait Installable: Send + Sync {
    /// Installs a set of packages according to the provided specifications.
    /// The `sudo` parameter is provided by the execution engine based on `needs_root()`.
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()>;

    /// Purges a set of packages from the system by name.
    async fn remove(&self, names: &[String], sudo: bool) -> Result<()>;
}

/// Capability trait for backends that can inspect local system state (Read Access).
#[async_trait]
pub trait Queryable: Send + Sync {
    /// Returns a list of every package currently installed via this backend.
    async fn list_installed(&self) -> Result<Vec<Package>>;

    /// Returns only packages explicitly requested by the user (non-dependencies).
    /// Backends whose installed set is user-requested by nature (`cargo install`, and
    /// every manager with no dependency concept) may return `list_installed` verbatim.
    /// A backend that cannot tell the two apart must report `tracks_manual() == false`
    /// and return an empty list — never the whole installed set.
    async fn list_manual(&self) -> Result<Vec<Package>>;

    /// Whether `list_manual` reflects real user intent rather than a guess.
    ///
    /// Adoption (`migrate`) writes what it discovers into the global state registry, and
    /// anything in that registry is a removal candidate on the next sync. So a backend
    /// that answers "everything installed" when it means "I don't know" gets a system's
    /// entire dependency graph adopted and then purged. Defaults to true, which is right
    /// for managers that install no dependencies; managers with a real dependency graph
    /// and no way to query intent must override it to false.
    fn tracks_manual(&self) -> bool {
        true
    }

    /// How `list_manual` decided what the user chose, phrased so a person can judge it.
    ///
    /// Adoption writes an estimate into a file the user is then asked to trust. An
    /// estimate whose provenance is hidden cannot be checked, and this one is wrong often
    /// enough to matter: naming the command lets a reader reproduce it and disagree.
    fn manual_source(&self) -> String {
        "everything this manager installed (it installs no dependencies of its own)".to_string()
    }

    /// Names the OS itself marks as essential — packages automated removal must refuse to
    /// touch regardless of what a manifest declares. Default: empty (no such concept).
    async fn essential(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Fetches rich metadata (version, install path, etc.) for a specific package.
    async fn info(&self, name: &str) -> Result<Option<Package>>;
}

/// Capability trait for backends that can search remote repositories.
#[async_trait]
pub trait Searchable: Send + Sync {
    /// Performs a remote query and returns a list of matching available packages.
    async fn search(&self, query: &str) -> Result<Vec<Package>>;

    /// Checks if a specific package exists in the remote repository.
    async fn remote_has(&self, name: &str) -> Result<bool> {
        let results = self.search(name).await?;
        Ok(results.iter().any(|p| p.name == name))
    }

    /// Gets detailed remote information for a package without installing it.
    async fn remote_info(&self, name: &str) -> Result<Option<Package>> {
        let results = self.search(name).await?;
        Ok(results.into_iter().find(|p| p.name == name))
    }
}

/// Capability trait for backends that support maintenance and batch upgrades.
#[async_trait]
pub trait Upgradable: Send + Sync {
    /// Refreshes local metadata, cache, or package indices (e.g. 'apt update').
    async fn update(&self, sudo: bool) -> Result<()>;

    /// Upgrades all packages managed by this backend to their latest compatible versions.
    async fn upgrade(&self, sudo: bool) -> Result<()>;

    /// identifies and removes unused or orphaned dependencies.
    async fn clean_orphans(&self, sudo: bool) -> Result<()>;
}

/// Capability trait for backends that support source/repository management.
#[async_trait]
pub trait RepoManager: Send + Sync {
    /// Adds a new package source (e.g., PPA, Tap, or Bucket).
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()>;
    /// Removes an existing package source.
    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()>;
    /// Lists all configured repositories/sources.
    async fn list_repos(&self) -> Result<Vec<(String, String)>>;
}

/// Phase 1.1: Capability trait for providing backend-native dependency metadata.
///
/// This is used by the `ChangePlanner` to perform recursive expansion of the system
/// dependency graph. The names returned must be the package names used natively
/// by the underlying package manager.
#[async_trait]
pub trait MetadataProvider: Send + Sync {
    /// Returns a list of native package names that are direct dependencies for the given package.
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>>;
}

// ============================================================================
// Capability Aggregation (Composition over Inheritance)
// ============================================================================

/// A container that aggregates a backend's core identity and its optional capabilities.
///
/// This structure allows the LiNix engine to query backends for specific
/// functionalities (ISP) without requiring every backend to implement every trait.
pub struct BackendCapabilities {
    core: Arc<dyn BackendCore>,
    installable: Option<Arc<dyn Installable>>,
    searchable: Option<Arc<dyn Searchable>>,
    queryable: Option<Arc<dyn Queryable>>,
    upgradable: Option<Arc<dyn Upgradable>>,
    repo_manager: Option<Arc<dyn RepoManager>>,
    metadata_provider: Option<Arc<dyn MetadataProvider>>,
}

impl BackendCapabilities {
    /// Initializes the capability builder for a backend.
    pub fn builder(core: Arc<dyn BackendCore>) -> BackendCapabilitiesBuilder {
        BackendCapabilitiesBuilder::new(core)
    }

    pub fn core(&self) -> &Arc<dyn BackendCore> {
        &self.core
    }
    pub fn name(&self) -> &str {
        self.core.name()
    }
    pub fn is_available(&self) -> bool {
        self.core.is_available()
    }
    pub fn needs_root(&self) -> bool {
        self.core.needs_root()
    }

    /// Single source of truth for the privilege policy on **write** operations
    /// (install / remove / upgrade / clean_orphans / repo changes): escalate iff the
    /// backend declares it needs root. Call this instead of `needs_root()` at write
    /// sites so the policy lives in one place, not scattered ad hoc per call site.
    pub fn sudo_for_write(&self) -> bool {
        self.core.needs_root()
    }

    /// Privilege policy for **read-only** queries (list/info/search/dependency probes):
    /// never escalate. Provided as a named constant so read sites document intent
    /// rather than passing a bare `false`.
    pub fn sudo_for_read(&self) -> bool {
        false
    }

    pub fn is_installable(&self) -> bool {
        self.installable.is_some()
    }
    pub fn as_installable(&self) -> Option<&Arc<dyn Installable>> {
        self.installable.as_ref()
    }

    pub fn is_searchable(&self) -> bool {
        self.searchable.is_some()
    }
    pub fn as_searchable(&self) -> Option<&Arc<dyn Searchable>> {
        self.searchable.as_ref()
    }

    pub fn is_queryable(&self) -> bool {
        self.queryable.is_some()
    }
    pub fn as_queryable(&self) -> Option<&Arc<dyn Queryable>> {
        self.queryable.as_ref()
    }

    pub fn is_upgradable(&self) -> bool {
        self.upgradable.is_some()
    }
    pub fn as_upgradable(&self) -> Option<&Arc<dyn Upgradable>> {
        self.upgradable.as_ref()
    }

    pub fn is_repo_manager(&self) -> bool {
        self.repo_manager.is_some()
    }
    pub fn as_repo_manager(&self) -> Option<&Arc<dyn RepoManager>> {
        self.repo_manager.as_ref()
    }

    pub fn is_metadata_provider(&self) -> bool {
        self.metadata_provider.is_some()
    }
    pub fn as_metadata_provider(&self) -> Option<&Arc<dyn MetadataProvider>> {
        self.metadata_provider.as_ref()
    }
}

/// Builder for constructing BackendCapabilities with a fluent interface.
pub struct BackendCapabilitiesBuilder {
    core: Arc<dyn BackendCore>,
    installable: Option<Arc<dyn Installable>>,
    searchable: Option<Arc<dyn Searchable>>,
    queryable: Option<Arc<dyn Queryable>>,
    upgradable: Option<Arc<dyn Upgradable>>,
    repo_manager: Option<Arc<dyn RepoManager>>,
    metadata_provider: Option<Arc<dyn MetadataProvider>>,
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
            metadata_provider: None,
        }
    }

    pub fn with_installable(mut self, i: Arc<dyn Installable>) -> Self {
        self.installable = Some(i);
        self
    }
    pub fn with_searchable(mut self, s: Arc<dyn Searchable>) -> Self {
        self.searchable = Some(s);
        self
    }
    pub fn with_queryable(mut self, q: Arc<dyn Queryable>) -> Self {
        self.queryable = Some(q);
        self
    }
    pub fn with_upgradable(mut self, u: Arc<dyn Upgradable>) -> Self {
        self.upgradable = Some(u);
        self
    }
    pub fn with_repo_manager(mut self, r: Arc<dyn RepoManager>) -> Self {
        self.repo_manager = Some(r);
        self
    }
    pub fn with_metadata_provider(mut self, m: Arc<dyn MetadataProvider>) -> Self {
        self.metadata_provider = Some(m);
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
            metadata_provider: self.metadata_provider,
        }
    }
}
