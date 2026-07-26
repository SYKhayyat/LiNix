use crate::core::{Package, PackageSpec, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Ok,
    /// Backend is present but requires attention (e.g. out of date, missing optional deps).
    Degraded,
    /// Backend is unusable (e.g. binary missing, network unreachable).
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub message: Option<String>,
}

#[async_trait]
pub trait BackendCore: Send + Sync {
    fn name(&self) -> &str;

    fn is_available(&self) -> bool;

    /// System managers (apt, dnf) return true. User managers (cargo, npm, scoop) return
    /// false.
    fn needs_root(&self) -> bool;

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

#[async_trait]
pub trait Installable: Send + Sync {
    /// The `sudo` parameter is provided by the execution engine based on `needs_root()`.
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()>;

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()>;

    /// Remove, and also destroy the package's configuration — Debian's `purge`.
    ///
    /// Separate from `remove` because a deleted module line says "stop installing this",
    /// which is not the same sentence as "destroy how I had it set up". A manager that
    /// draws no such distinction refuses, rather than silently doing an ordinary removal
    /// under a name that promised more.
    async fn purge(&self, _names: &[String], _sudo: bool) -> Result<()> {
        Err(crate::core::Error::Unsupported("purge".into()))
    }

    /// Whether [`Installable::purge`] would do something different from `remove`.
    fn supports_purge(&self) -> bool {
        false
    }
}

#[async_trait]
pub trait Queryable: Send + Sync {
    async fn list_installed(&self) -> Result<Vec<Package>>;

    /// Returns only packages explicitly requested by the user (non-dependencies).
    /// Backends whose installed set is user-requested by nature (`cargo install`, and
    /// every manager with no dependency concept) may return `list_installed` verbatim.
    /// A backend that cannot tell the two apart must report `tracks_manual() == false`
    /// and return an empty list — never the whole installed set.
    async fn list_manual(&self) -> Result<Vec<Package>>;

    /// Whether `list_manual` reflects real user intent rather than a guess.
    ///
    /// Adoption writes what it discovers into the global state registry, and
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

    async fn info(&self, name: &str) -> Result<Option<Package>>;

    /// System packages this backend installed *through another manager* (D5), as
    /// `(installer, package_name)` pairs. When `github:`/`web:` hands a `.deb` to `dpkg`, the
    /// package shows up in apt's installed list, but it is not unmanaged drift — it is owned
    /// here. The unmanaged crawl subtracts these so it neither reports the file twice nor lets
    /// `purge-unmanaged` delete a package a download declaration is responsible for. Default:
    /// none, for every manager that never hands a file to a second one.
    async fn owned_system_packages(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}

/// Every package name this manager could install, without being told what to look for.
///
/// Separate from [`Searchable`], which answers a query: a search matches names *and*
/// descriptions and ranks them, so "does `^fonts-` match?" cannot be asked of it. II.15's
/// `re:` needs the full name list and nothing else.
///
/// **Most managers cannot do this and must not pretend to.** A system manager has a local
/// index of everything its repositories carry; a language registry has millions of packages
/// and no list endpoint. A backend with no honest answer implements nothing here, and a `re:`
/// line naming it is refused rather than silently expanding to nothing.
#[async_trait]
pub trait Enumerable: Send + Sync {
    async fn available_names(&self) -> Result<Vec<String>>;
}

#[async_trait]
pub trait Searchable: Send + Sync {
    async fn search(&self, query: &str) -> Result<Vec<Package>>;

    async fn remote_has(&self, name: &str) -> Result<bool> {
        let results = self.search(name).await?;
        Ok(results.iter().any(|p| p.name == name))
    }

    async fn remote_info(&self, name: &str) -> Result<Option<Package>> {
        let results = self.search(name).await?;
        Ok(results.into_iter().find(|p| p.name == name))
    }
}

#[async_trait]
pub trait Upgradable: Send + Sync {
    async fn update(&self, sudo: bool) -> Result<()>;

    async fn upgrade(&self, sudo: bool) -> Result<()>;

    /// The packages this backend considers orphaned — named, not removed.
    ///
    /// Removal is the caller's job precisely because it cannot be the backend's: a set that
    /// nobody can enumerate cannot be shown to the user, checked against the protected list,
    /// or counted by the guard. `Unsupported` means this backend has no orphan concept, and
    /// a backend that cannot list its orphans never has them removed.
    async fn list_orphans(&self) -> Result<Vec<String>> {
        Err(crate::core::Error::Unsupported("orphan listing".into()))
    }

    /// Delete downloaded package archives and other caches. Frees disk and removes no
    /// installed package — which is why it needs neither a preview nor the guard.
    async fn clean_cache(&self, _sudo: bool) -> Result<()> {
        Err(crate::core::Error::Unsupported("cache cleaning".into()))
    }
}

#[async_trait]
pub trait RepoManager: Send + Sync {
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()>;
    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()>;
    async fn list_repos(&self) -> Result<Vec<(String, String)>>;
}

/// The `ChangePlanner` expands the returned names recursively against the same backend,
/// so they must be the names that backend itself uses — a normalized or display name
/// re-enters the graph as an unresolvable node.
#[async_trait]
pub trait MetadataProvider: Send + Sync {
    /// Direct dependencies only; the caller handles transitive expansion.
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>>;
}

/// Lets the engine query a backend for one capability without every backend having to
/// implement every trait.
pub struct BackendCapabilities {
    core: Arc<dyn BackendCore>,
    installable: Option<Arc<dyn Installable>>,
    searchable: Option<Arc<dyn Searchable>>,
    enumerable: Option<Arc<dyn Enumerable>>,
    queryable: Option<Arc<dyn Queryable>>,
    upgradable: Option<Arc<dyn Upgradable>>,
    repo_manager: Option<Arc<dyn RepoManager>>,
    metadata_provider: Option<Arc<dyn MetadataProvider>>,
}

impl BackendCapabilities {
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

    pub fn as_enumerable(&self) -> Option<&Arc<dyn Enumerable>> {
        self.enumerable.as_ref()
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

pub struct BackendCapabilitiesBuilder {
    core: Arc<dyn BackendCore>,
    installable: Option<Arc<dyn Installable>>,
    searchable: Option<Arc<dyn Searchable>>,
    enumerable: Option<Arc<dyn Enumerable>>,
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
            enumerable: None,
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
    pub fn with_enumerable(mut self, e: Arc<dyn Enumerable>) -> Self {
        self.enumerable = Some(e);
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
            enumerable: self.enumerable,
            queryable: self.queryable,
            upgradable: self.upgradable,
            repo_manager: self.repo_manager,
            metadata_provider: self.metadata_provider,
        }
    }
}
