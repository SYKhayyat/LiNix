use crate::core::{Package, PackageSpec, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Ok,
    /// Backend is present but requires attention (e.g. out of date, missing optional deps).
    Degraded,
    /// It is installed, or `priority` names it, and it cannot work.
    Critical,
    /// Not installed here, and nothing asked for it (Q2, II.8, V.91).
    ///
    /// Never counted as a failure. `25 OK, 0 degraded, 23 critical` on a healthy Windows box
    /// was fail-loud pointed at something that had not failed — apt and brew are not broken on
    /// a machine that was never going to have them, and spending the word "critical" on them
    /// is what makes a real critical unreadable.
    Absent,
}

/// The one rendering of "this backend's program is not here" (V.94).
///
/// There were two, and a user saw both in one screen: `` `cabal` is not on PATH, so the
/// `cabal` backend cannot run `` from the generic backend, and `Binary for snap not found in
/// PATH` from this module's default. The default was also wrong about *what* it probed —
/// `lvm` says `lvm` and probes `lvs`, `xbps` says `xbps` and probes `xbps-install`, `krew`
/// says `krew` and probes two programs, `appimage` says `appimage` and probes nothing at all.
/// So the message is built from [`BackendCore::probes`], which each backend must state.
pub fn missing_program(backend: &str, programs: &[String]) -> HealthReport {
    let message = match programs {
        [] => format!("the `{}` backend cannot run on this machine", backend),
        [one] => format!("{}, so the `{}` backend cannot run", located(one), backend),
        // Deliberately says nothing about *how many* of them are needed. `krew` wants both
        // `kubectl` and `kubectl-krew`; `service` wants any one of several init programs.
        // Naming every program it looked for is true of both, and asserting a quantifier here
        // would make one of them wrong — which is the whole defect this message replaced.
        many => format!(
            "the `{}` backend could not find the program(s) it needs: {}",
            backend,
            many.iter()
                .map(|p| format!("`{}`", p))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    HealthReport {
        status: HealthStatus::Absent,
        message: Some(message),
    }
}

/// "not found" rather than "not on PATH": a custom backend's binary may be an absolute path
/// (U16), and telling someone their `/opt/vendor/thing` is "not on PATH" points them at the
/// wrong thing to fix.
fn located(program: &str) -> String {
    if program.contains(['/', '\\']) {
        format!("`{}` does not exist or is not executable", program)
    } else {
        format!("`{}` is not on PATH", program)
    }
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

    /// The program(s) `is_available` actually looks for.
    ///
    /// Stated per backend rather than assumed from the name, because the assumption was wrong
    /// for four of them and told users to install things that do not exist. Empty means this
    /// backend probes no external program — it is built in, or gated on the platform.
    fn probes(&self) -> Vec<String>;

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
            Ok(missing_program(self.name(), &self.probes()))
        }
    }
}

/// The actuator half of a converge: what to run once something upstream has decided the
/// machine and the declaration disagree.
///
/// **It does not decide, and that is why it does not look like an engine.** Half the
/// implementors here converge something that is not a package — a `service:` is enabled, a
/// `setting:` written, a `zfs:` dataset created and given a quota — and each `install` body
/// therefore reads a little state before acting, which looks from the outside like four
/// hand-written converge loops with no shared machinery. They are not. The decision was
/// already made, in exactly one of two places, and the read inside the body is a local
/// idempotence guard behind it:
///
/// - **On the package path**, `ChangePlanner` computes `desired − present` and then asks
///   `is_drifted` — one comparison covering `@quota`, `@size`, `@mount`, `@mount_options`,
///   `@channel` and `@classic` across zfs, lvm, btrfs and snap. A converged declaration never
///   reaches this trait at all.
/// - **On the dependent path** (II.7's phase 3), `Dependents::apply` asks
///   `apply::extras::in_effect` per resource and skips anything already in force. That probe
///   exists because the loop once did *not* ask it: every sync re-copied all three declared
///   links on Windows and left `.linix-backup` files beside them — backups of the copies LiNix
///   had made itself — under a summary reading `already up to date`.
///
/// So a new implementor's obligation is not "write a converge loop". It is: **be reachable
/// only through one of those two deciders**, and be idempotent if run twice anyway.
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
    /// Where this backend's once-per-run listing is kept, and the name to keep it under.
    ///
    /// Every backend is built on a duplicate of one `App`'s executor, and every duplicate
    /// shares the same listings — so this is what makes the memo per-run rather than per
    /// process.
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str);

    /// Ask the manager what it has installed.
    ///
    /// **Implemented per backend; not called directly.** Callers use
    /// [`Queryable::list_installed`], which asks this once per run.
    async fn fetch_installed(&self) -> Result<Vec<Package>>;

    /// Everything this manager has installed — asked **once per run**.
    ///
    /// Eighteen backends implement `info(name)` as "list the whole machine, then find one",
    /// and the callers ask `info` once per *declared* package. Measured on Ubuntu that was
    /// exactly `declared + 1` `dpkg-query` invocations for a read-only `check drift`; measured
    /// on Windows it was ~247 ms of marginal cost per additional declaration, because a
    /// `winget list` takes over a second. `check health`, `adopt`, the unmanaged crawl and the
    /// planner's `installed_sets` each list every backend again on top of that.
    ///
    /// The answer does not change while nothing is being installed, so it is fetched once. The
    /// one thing that can change it is a mutating command, and `CommandExecutor::run` forgets
    /// these when one finishes.
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let (memo, key) = self.installed_cache();
        memo.once(key, self.fetch_installed()).await
    }

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

    /// Whether a bare `linix adopt` takes this backend, or waits to be asked for it by name.
    ///
    /// The neighbour of [`tracks_manual`](Self::tracks_manual), one step further along the same
    /// question. That one refuses a backend that cannot tell a user's choices from
    /// dependencies. This one is for a backend that can list what is on the machine perfectly
    /// well, where *being on the machine is not evidence anybody chose it*.
    ///
    /// Measured on a Windows host: `adopt` wrote 161 declarations, **150 of them every running
    /// Windows service**. Nobody chose those; Windows did — and two, `gpsvc` and `smphost`, had
    /// stopped on their own twenty minutes later, because Windows starts them on a trigger and
    /// stops them when idle. A manifest is a list of what you want, and deleting a line from it
    /// undoes the thing, so 93% of that file was a loaded list nobody wrote.
    ///
    /// `false` does not mean unadoptable: `linix adopt <backend>` takes it, and says so when it
    /// skips (owner ruling, 2026-08-05 — `Q39`).
    fn adopted_unasked(&self) -> bool {
        true
    }

    /// The subset of [`list_manual`](Self::list_manual) this machine brings up on its own.
    ///
    /// `None` from a backend with no such notion — a package is not "enabled" — and from one
    /// whose adapter on this host cannot report it, which `adopt --enabled-only` refuses by
    /// name rather than quietly falling back to everything.
    async fn list_manual_enabled(&self) -> Result<Option<Vec<Package>>> {
        Ok(None)
    }

    /// How `list_manual` decided what the user chose, phrased so a person can judge it.
    ///
    /// Adoption writes an estimate into a file the user is then asked to trust. An
    /// estimate whose provenance is hidden cannot be checked, and this one is wrong often
    /// enough to matter: naming the command lets a reader reproduce it and disagree.
    fn manual_source(&self) -> String {
        "everything this manager installed (it installs no dependencies of its own)".to_string()
    }

    /// The options `adopt` must write beside a name for the declaration to mean what was
    /// observed. Empty for a package: `apt:jq` already says everything the listing said.
    ///
    /// A `service:` line with no options means *enable and start* (`actions_for`), and enabling
    /// on Windows rewrites the service's start type to automatic. The init only ever reports
    /// **running** services, so what was observed is `status=running` and the start type was
    /// never looked at — declaring it anyway would reconfigure the machine's boot on the first
    /// sync after an adopt whose whole promise is to describe it as it already is.
    fn adoption_options(&self) -> Vec<(String, String)> {
        Vec::new()
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
    /// `purge-undeclared` delete a package a download declaration is responsible for. Default:
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

    /// What this manager knows about one exact name — presence and version in one answer.
    ///
    /// There were two methods here, `remote_has` returning a `bool` and `remote_info` returning
    /// the record, both defaulting to a full `search`, and neither ever overridden. The
    /// resolver asked `remote_has`, could not tell an honest `false` from an unimplemented one,
    /// and re-ran *the identical search with the identical argument* to find out — then ran a
    /// third for the version when the line carried an `@version=`. Measured: two `apt-cache
    /// search` calls for every candidate the priority chain rejects, which is most of them, and
    /// three for any pinned name. One question, asked once, answers all of it.
    ///
    /// A manager with a genuinely cheaper targeted query — `brew info`, `apt-cache show`,
    /// `pip index versions` — overrides this and pays one round trip instead of a full search.
    /// `None` means this manager looked and does not have the name; an `Err` means it could not
    /// tell, which the resolver treats as a different answer from "no".
    /// Everything this manager has a newer version for, asked **once** (`Q44`).
    ///
    /// `None` — the default — means this manager has no such verb, and the caller must fall
    /// back to asking about each package separately. `Some(vec![])` is a different answer: the
    /// manager was asked, and nothing is out of date.
    ///
    /// Each returned `Package` carries the name and the version *available*, not the installed
    /// one; the caller already knows what is installed and only needs the other half.
    ///
    /// **This exists because asking per package is not a slower way to get the same answer, it
    /// is the wrong question.** [`lookup`](Self::lookup) defaults to a whole `search` for one
    /// name, so `list --outdated` ran one registry search per installed package: measured on a
    /// Windows host, **771.4s against 2.9s for a plain `list`**. Nearly every manager answers
    /// the entire question in one command — `apt list --upgradable`, `pacman -Qu`,
    /// `npm outdated -g --json` — and the ones that cannot are the exception worth naming.
    async fn outdated_all(&self) -> Result<Option<Vec<Package>>> {
        Ok(None)
    }

    async fn lookup(&self, name: &str) -> Result<Option<Package>> {
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

/// What a manager says one package depends on — **reported, never acted on**.
///
/// `linix info <name>` prints it and `linix why` searches it for reverse dependencies.
/// Nothing plans from it. The planner used to: it added each returned name as an install node,
/// which took ownership of a package nobody declared and wired an edge that split the manager's
/// own command line in two. Whatever a manager installs alongside what you asked for is that
/// manager's business, and it does it at install time whether or not LiNix asks first.
///
/// So a backend answering here owes only what it can honestly report — direct dependencies or
/// the whole closure, whichever its own verb gives — and owes nothing about installability.
#[async_trait]
pub trait MetadataProvider: Send + Sync {
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
