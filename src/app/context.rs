use crate::config::grammar::Origin;
use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::migrate::Migrator;
use crate::app::profile::ProfileManager;
use crate::app::run::Runner;
use crate::app::scheduler::notify::NotificationManager;
use crate::app::scheduler::SchedulerManager;
use crate::app::shell::GhostShell;
use crate::app::shim_manager::ShimManager;
use crate::app::sync::resolver::StateResolver;
use crate::app::sync::SyncEngine;
use crate::app::teleport::Teleporter;
use crate::app::undo::UndoManager;
use crate::backends::{create_default_registry, BackendRegistry};
use crate::config::Config;
use crate::core::{
    CommandExecutor, Error, Journal, Package, PackageCache, PackageSpec, Result, SnapshotManager,
    StateRegistry, Validator,
};
use crate::utils::progress::{create_progress_reporter, ProgressReporter};

use super::{LuaHooks, MetricsCollector, UniversalSearch};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, instrument, warn};

/// Turn a non-package statement's options into a `PackageSpec` the `service`/`link`
/// backends consume. Their `Installable::install` reads the options it knows (`enabled`,
/// `status`, `target`, `content`, `template`, `decrypt`, …); a key it doesn't know is
/// simply ignored, which is why the grammar — not this conversion — is where an unknown
/// key is refused. Options are single-valued here (a service is enabled or not), so the
/// first value of each key is taken.
fn spec_from_extra(backend: &str, name: &str, opts: &crate::config::grammar::Options) -> PackageSpec {
    let mut options = std::collections::HashMap::new();
    for (key, values) in opts.iter() {
        if let Some(first) = values.first() {
            options.insert(key.to_string(), first.clone());
        }
    }
    PackageSpec {
        name: name.to_string(),
        backend: backend.to_string(),
        options,
        requires: Vec::new(),
        present: true,
    }
}

pub struct App {
    pub config: Arc<Config>,
    pub cache: Arc<PackageCache>,
    pub registry: Arc<BackendRegistry>,
    pub executor: CommandExecutor,
    pub metrics: MetricsCollector,
    pub progress: Arc<dyn ProgressReporter>,
    pub hooks: Arc<LuaHooks>,
    pub state: Arc<Mutex<StateRegistry>>,
    pub snapshot_manager: Arc<SnapshotManager>,
    pub journal: Arc<Mutex<Journal>>,
    pub diagnostics: Arc<FailureDiagnosticEngine>,
    pub scheduler: Arc<SchedulerManager>,
    pub notifications: Arc<NotificationManager>,
}

impl App {
    /// `state_path` overrides where LiNix's own data lives. `None` means the real data
    /// dir; a test passes a temp path so it never touches — or accumulates in — the
    /// user's.
    pub async fn new_with_executor_and_state_path(
        config: Config,
        executor: CommandExecutor,
        state_path: Option<PathBuf>,
    ) -> Result<Self> {
        debug!("LiNix Kernel: Initiating mission-critical service bootstrap.");

        let hooks = Arc::new(LuaHooks::new(&config)?);

        let registry =
            Arc::new(create_default_registry(executor.duplicate(), &config, hooks.clone()).await);
        let progress = create_progress_reporter(config.show_progress);

        // The journal lives beside the registry: both are LiNix's record of what it did,
        // so isolating one and not the other left the WAL pointing at real user data.
        let journal_path = state_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.join("journal.json"))
            .unwrap_or_else(|| crate::utils::safe_data_dir().join("journal.json"));

        let state_registry = if let Some(path) = state_path {
            tokio::task::spawn_blocking(move || StateRegistry::load_from(&path))
                .await
                .map_err(|e| {
                    Error::Other(format!("Kernel Thread Panic during state load: {}", e))
                })?
        } else {
            tokio::task::spawn_blocking(StateRegistry::load_default)
                .await
                .map_err(|e| {
                    Error::Other(format!("Kernel Thread Panic during state load: {}", e))
                })?
        }?;
        let state = Arc::new(Mutex::new(state_registry));

        let snapshot_manager = Arc::new(SnapshotManager::new(executor.duplicate(), &config).await);
        let journal = Arc::new(Mutex::new(Journal::at(journal_path)?));

        let scheduler = Arc::new(SchedulerManager::new()?);
        let config_arc = Arc::new(config);
        let notifications = Arc::new(NotificationManager::new(config_arc.clone()));

        let diagnostics = Arc::new(FailureDiagnosticEngine::init(&config_arc).await);

        info!("LiNix Kernel: v6.0.0 kernel initialized successfully.");

        Ok(Self {
            config: config_arc,
            cache: Arc::new(PackageCache::new()),
            registry,
            executor,
            metrics: MetricsCollector::new(),
            progress,
            hooks,
            state,
            snapshot_manager,
            journal,
            diagnostics,
            scheduler,
            notifications,
        })
    }

    pub async fn new_with_executor(config: Config, executor: CommandExecutor) -> Result<Self> {
        Self::new_with_executor_and_state_path(config, executor, None).await
    }

    pub async fn new(config: Config) -> Result<Self> {
        let executor = CommandExecutor::new(config.dry_run, config.verbose);
        Self::new_with_executor_and_state_path(config, executor, None).await
    }

    pub fn migrator(&self) -> Migrator {
        Migrator::new(self.registry.clone(), self.state.clone(), &self.config)
    }

    pub fn teleporter(&self) -> Teleporter {
        Teleporter::new(
            self.registry.clone(),
            self.journal.clone(),
            self.state.clone(),
            self.diagnostics.clone(),
            self.config.clone(),
        )
    }

    pub fn shell(&self) -> GhostShell {
        GhostShell::new(
            self.registry.clone(),
            self.state.clone(),
            self.config.clone(),
            self.executor.duplicate(),
            self.metrics.clone(),
            self.progress.clone(),
            self.hooks.clone(),
            self.snapshot_manager.clone(),
            self.journal.clone(),
            self.diagnostics.clone(),
        )
    }

    pub fn profile_manager(&self) -> ProfileManager {
        ProfileManager::new(
            self.registry.clone(),
            self.executor.clone(),
            self.metrics.clone(),
            self.progress.clone(),
            self.hooks.clone(),
            self.snapshot_manager.clone(),
            self.journal.clone(),
            self.state.clone(),
            self.config.clone(),
            self.diagnostics.clone(),
        )
    }

    pub fn undo_manager(&self) -> UndoManager {
        UndoManager::new(
            self.snapshot_manager.clone(),
            self.state.clone(),
            self.executor.clone(),
            self.config.config_root(),
        )
    }

    pub fn runner(&self) -> Runner {
        Runner::new(self.registry.clone(), self.config.clone())
    }

    pub async fn shim_manager(&self) -> Result<ShimManager> {
        ShimManager::new().await
    }

    pub async fn sync_engine(&self) -> SyncEngine<'_> {
        SyncEngine::new(
            &self.config,
            self.registry.clone(),
            self.executor.duplicate(),
            self.metrics.clone(),
            self.progress.clone(),
            self.hooks.clone(),
            self.snapshot_manager.clone(),
            self.journal.clone(),
            self.state.clone(),
            self.diagnostics.clone(),
        )
        .await
    }

    /// This machine's backend vocabulary, for anything that reads or writes a line.
    pub async fn vocabulary(&self) -> Result<crate::app::vocab::Vocab> {
        let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;
        let priority = resolver.priority_for_host().await?;
        Ok(crate::app::vocab::Vocab::new(
            &self.registry,
            &self.config,
            &priority,
        ))
    }

    /// Write a declaration into your files (P1: an imperative command is a shortcut for
    /// editing a file and syncing), and say which file it touched (II.8).
    ///
    /// `into` is II.8's `--into`: a module (lowercase) or a profile (Capitalized). Without
    /// it, the line lands in the module named for how it arrived (V.40).
    pub async fn declare(
        &self,
        line: &str,
        into: Option<&str>,
        landing: crate::model::Landing,
    ) -> Result<crate::model::Edit> {
        let vocab = self.vocabulary().await?;
        let layout = self.config.layout();
        let target = match into {
            Some(name) => crate::model::Target::parse(name, &Origin::argument())?,
            None => landing.target(),
        };
        let edit = crate::model::Editor::new(&layout, &vocab)
            .add(&target, line)
            .map_err(Error::from)?;
        info!("{}", edit.describe("Added"));
        Ok(edit)
    }

    /// Apply every `repo:` line, then refresh the indexes of the backends touched.
    ///
    /// **First in the ordering (II.7): repos → refresh → packages → dependents.** A package
    /// from a PPA is uninstallable until the PPA is added and `apt update` has seen it, so
    /// this must complete before the package plan runs — it is not a step the planner can
    /// interleave. Each repo names its backend (V.47), so there is no guessing which tool
    /// adds it. Idempotent: adding a repo that already exists is a no-op every backend
    /// tolerates, which is what lets a repo live in a file that syncs on every run.
    pub async fn apply_repositories(
        &self,
        state: &crate::model::DesiredState,
    ) -> Result<()> {
        use crate::config::grammar::Statement;

        let mut touched: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (stmt, origin) in &state.extras {
            let Statement::Repo { backend, spec } = stmt else {
                continue;
            };
            let Some(b) = self.registry.get(backend) else {
                warn!("{}: backend `{}` is not available here — skipping repo `{}`.", origin, backend, spec);
                continue;
            };
            let Some(repos) = b.as_repo_manager() else {
                return Err(Error::Config(format!(
                    "{}: `{}` cannot manage repositories, so `repo:{}:{}` has nowhere to go.",
                    origin, backend, backend, spec
                )));
            };
            if self.config.dry_run {
                info!("[DRY-RUN] would add repo `{}` to {}", spec, backend);
            } else {
                info!("Repo: adding `{}` to {} ({})", spec, backend, origin);
                repos.add_repo(spec, spec, b.sudo_for_write()).await?;
            }
            touched.insert(backend.clone());
        }

        // Refresh once per backend, after all its repos are in — an index refresh is the
        // slow part, and doing it per-repo would pay that cost N times for one backend.
        for backend in touched {
            if self.config.dry_run {
                info!("[DRY-RUN] would refresh {} package index", backend);
                continue;
            }
            if let Some(b) = self.registry.get(&backend) {
                if let Some(up) = b.as_upgradable() {
                    info!("Repo: refreshing {} package index", backend);
                    if let Err(e) = up.update(b.sudo_for_write()).await {
                        warn!("Repo: {} index refresh failed: {} — a package from a new repo may not be found yet.", backend, e);
                    }
                }
            }
        }
        Ok(())
    }

    /// Apply the dependent extras — shims, services and links — AFTER the package plan has
    /// run (II.7's dependent phase, the mirror of `apply_repositories`'s phase 1).
    ///
    /// **Why after packages, not interleaved:** each of these presupposes a package. A
    /// `shim:` wraps a binary that must already be on disk; a `service:` enables a unit a
    /// package just installed; a `link:` writes the config a package expects to read. So
    /// they cannot be planned alongside packages — they must wait for the whole package
    /// plan to finish. Applied in declaration order, so a user who writes the config `link:`
    /// above the `service:` that reads it gets that order honoured.
    ///
    /// Idempotent, like the repo phase: re-deploying an existing shim, re-enabling a running
    /// service, or re-writing an unchanged link are all no-ops, which is what lets these
    /// lines live in a file that syncs on every run. This is the forward (declared →
    /// applied) direction only; reconciling away a *removed* dependent line is drift the
    /// package planner does not yet track for extras.
    pub async fn apply_dependents(
        &self,
        state: &crate::model::DesiredState,
    ) -> Result<()> {
        use crate::config::grammar::Statement;

        for (stmt, origin) in state.dependents() {
            match stmt {
                Statement::Shim(name, _opts) => {
                    if self.config.dry_run {
                        info!("[DRY-RUN] would deploy shim `{}`", name);
                        continue;
                    }
                    info!("Shim: deploying `{}` ({})", name, origin);
                    self.shim_manager().await?.create_shim(name).await?;
                }
                Statement::Service(name, opts) => {
                    let Some(b) = self.registry.get("service") else {
                        warn!(
                            "{}: the service backend is not available here — skipping `service:{}`.",
                            origin, name
                        );
                        continue;
                    };
                    if self.config.dry_run {
                        info!("[DRY-RUN] would apply service `{}`", name);
                        continue;
                    }
                    let Some(inst) = b.as_installable() else {
                        continue;
                    };
                    info!("Service: applying `{}` ({})", name, origin);
                    let spec = spec_from_extra("service", name, opts);
                    inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
                        .await?;
                }
                Statement::Link(name, opts) => {
                    let Some(b) = self.registry.get("link") else {
                        warn!(
                            "{}: the link backend is not available here — skipping `link:{}`.",
                            origin, name
                        );
                        continue;
                    };
                    if self.config.dry_run {
                        info!("[DRY-RUN] would apply link `{}`", name);
                        continue;
                    }
                    let Some(inst) = b.as_installable() else {
                        continue;
                    };
                    info!("Link: applying `{}` ({})", name, origin);
                    let spec = spec_from_extra("link", name, opts);
                    inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
                        .await?;
                }
                // dependents() yields only these three variants.
                _ => {}
            }
        }
        Ok(())
    }

    /// Whether any active file declares this package.
    ///
    /// Asked through the resolver, so "declared" means the same thing here as it does to
    /// `sync` — a second definition of declared is a second answer.
    pub async fn declares(&self, target: &str) -> Result<bool> {
        let vocab = self.vocabulary().await?;
        let layout = self.config.layout();
        let facts = crate::config::parser::HostFacts::current();
        let files = crate::model::active_module_files(&layout, &vocab, &facts);
        let editor = crate::model::Editor::new(&layout, &vocab).with_facts(facts);
        Ok(editor.declares_in(&files, target))
    }

    /// Remove a package's declaration from every file the active profiles reach (II.8's
    /// `uninstall`), and say which files changed.
    pub async fn undeclare(&self, target_pkg: &str) -> Result<Vec<crate::model::Edit>> {
        let vocab = self.vocabulary().await?;
        let layout = self.config.layout();
        let facts = crate::config::parser::HostFacts::current();
        let files = crate::model::active_module_files(&layout, &vocab, &facts);
        let edits = crate::model::Editor::new(&layout, &vocab)
            .with_facts(facts)
            .remove_from(&files, target_pkg)
            .map_err(Error::from)?;
        for e in &edits {
            info!("{}", e.describe("Removed"));
        }
        Ok(edits)
    }

    #[instrument(skip(self))]
    pub async fn resolve_spec(&self, spec_str: &str) -> Result<Vec<PackageSpec>> {
        let mut resolved = Vec::new();
        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();

        let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;
        queue.push_back(resolver.parse_and_probe_spec(spec_str).await?);

        while let Some(spec) = queue.pop_front() {
            let key = format!("{}:{}", spec.backend, spec.name);
            if !seen.insert(key) {
                continue;
            }

            Validator::validate_package_name_for(&spec.name, &spec.backend)?;
            for req in &spec.requires {
                queue.push_back(resolver.parse_and_probe_spec(req).await?);
            }
            resolved.push(spec);
        }
        Ok(resolved)
    }

    pub async fn update(&self) -> Result<()> {
        info!("Kernel: Initiating metadata synchronization across enabled backends.");
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                upgradable.update(backend.sudo_for_write()).await?;
            }
        }
        Ok(())
    }

    pub async fn upgrade(&self) -> Result<()> {
        let _ = self.snapshot_manager.auto_snapshot("pre_upgrade").await?;
        info!("Kernel: Commencing system-wide batch upgrade.");
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                upgradable.upgrade(backend.sudo_for_write()).await?;
            }
        }
        self.metrics.print_summary();
        Ok(())
    }

    pub async fn list(&self, backend_filter: Option<&str>) -> Result<Vec<Package>> {
        let mut all_packages = Vec::new();
        for backend in self.registry.available() {
            if let Some(filter) = backend_filter {
                if backend.name() != filter {
                    continue;
                }
            }
            if let Some(queryable) = backend.as_queryable() {
                match queryable.list_installed().await {
                    Ok(pkgs) => all_packages.extend(pkgs),
                    Err(e) => debug!(
                        "Kernel: Query failed for backend '{}': {}",
                        backend.name(),
                        e
                    ),
                }
            }
        }
        Ok(all_packages)
    }

    pub async fn get_info(&self, package_name: &str) -> Result<Option<Package>> {
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(Some(pkg)) = queryable.info(package_name).await {
                    return Ok(Some(pkg));
                }
            }
        }
        Ok(None)
    }

    /// Everything installed that LiNix does not manage — the dependency closure included.
    ///
    /// **This is not `unmanaged` (II.8), which is "what `adopt` would adopt".** They are two
    /// questions with very different answers: on a stock Ubuntu this is ~476 packages and
    /// `adopt` is ~103. Only `purge-unmanaged` wants this one, and its whole job is deleting
    /// all of it (II.11) — which is why the ratio check exists.
    pub async fn installed_but_unmanaged(&self) -> Result<Vec<Package>> {
        let mut unmanaged = Vec::new();
        let state = self.state.lock().await;
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                if let Ok(installed) = queryable.list_installed().await {
                    for pkg in installed {
                        if !state.is_managed(&pkg.backend, &pkg.name) {
                            unmanaged.push(pkg);
                        }
                    }
                }
            }
        }
        Ok(unmanaged)
    }

    /// Remove any managed packages whose lease has expired, across their backends, and
    /// persist the updated state. Called during post-command maintenance so that
    /// temporary installs (`linix install foo@lease=30d`) really do uninstall themselves
    /// once time is up, without waiting for the next explicit `sync`/`prune`. No-op in
    /// dry-run mode.
    pub async fn sweep_expired_leases(&self) -> Result<()> {
        if self.config.dry_run {
            return Ok(());
        }
        let expired = { self.state.lock().await.get_expired_packages() };
        if expired.is_empty() {
            return Ok(());
        }

        // A lease is a promise to remove something later; it is not a promise to remove
        // something the system needs. Drop protected packages from the sweep rather than
        // failing: this runs as maintenance after every state-changing command, so a hard
        // error here would break unrelated commands. The package simply stays, and its
        // lease stays expired, which is the safe direction.
        let backends: std::collections::HashSet<String> =
            expired.iter().map(|(b, _)| b.clone()).collect();
        let os_essential =
            crate::app::sync::guard::essential_names(&self.registry, &backends).await;
        let (protected, expired): (Vec<_>, Vec<_>) = expired.into_iter().partition(|(b, n)| {
            crate::app::sync::guard::protection_of(&self.config, b, n, &os_essential).is_some()
        });
        for (backend, name) in &protected {
            warn!(
                "Kernel: lease on {}:{} expired, but it is protected — leaving it installed. \
                 Run `linix protected {}:{}` to see why.",
                backend, name, backend, name
            );
        }
        if expired.is_empty() {
            return Ok(());
        }

        // The count check still applies: a state file that expires hundreds of packages at
        // once is a bug, not an intention.
        let pairs: Vec<(String, String)> = expired.clone();
        if let Err(e) = crate::app::sync::guard::enforce(
            &self.config,
            &self.registry,
            &pairs,
            crate::app::sync::guard::GuardScope::Leases,
        )
        .await
        {
            warn!(
                "Kernel: expired-lease sweep refused, leaving them installed.\n{}",
                e
            );
            return Ok(());
        }

        info!(
            "Kernel: {} package(s) have expired leases — reclaiming.",
            expired.len()
        );
        for (backend, name) in expired {
            if let Some(b) = self.registry.get(&backend) {
                if let Some(inst) = b.as_installable() {
                    info!("Lease expired: removing {}:{}", backend, name);
                    if let Err(e) = inst
                        .remove(std::slice::from_ref(&name), b.sudo_for_write())
                        .await
                    {
                        warn!(
                            "Kernel: failed to remove expired {}:{}: {}",
                            backend, name, e
                        );
                        continue;
                    }
                    self.state.lock().await.remove(&backend, &name);
                }
            }
        }
        self.state.lock().await.save()?;
        Ok(())
    }

    /// Reinstall a single package by backend + name (best-effort restore). Version is
    /// intentionally not pinned — restore is reinstall-by-name, and a backend that no
    /// longer offers the package surfaces as an `Err` the caller can warn-and-move-on.
    async fn restore_package(&self, backend: &str, name: &str) -> Result<()> {
        let b = self
            .registry
            .get(backend)
            .ok_or_else(|| Error::BackendNotFound(backend.to_string()))?;
        let inst = b
            .as_installable()
            .ok_or_else(|| Error::Other(format!("Backend '{}' cannot install", backend)))?;
        let spec = PackageSpec {
            name: name.to_string(),
            backend: backend.to_string(),
            options: std::collections::HashMap::new(),
            requires: Vec::new(),
            present: true,
        };
        inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
            .await
    }

    /// Restore any packages whose temporary-uninstall timer has elapsed (the mirror of
    /// `sweep_expired_leases`). If a package can no longer be installed, we warn and move
    /// on — the suspension is cleared either way so a permanently-gone package doesn't
    /// nag on every run. No-op in dry-run mode.
    pub async fn sweep_due_suspensions(&self) -> Result<()> {
        if self.config.dry_run {
            return Ok(());
        }
        let due = { self.state.lock().await.get_due_suspensions() };
        if due.is_empty() {
            return Ok(());
        }
        info!(
            "Kernel: {} temporary uninstall(s) are due for restoration.",
            due.len()
        );
        for s in due {
            match self.restore_package(&s.backend, &s.name).await {
                Ok(()) => {
                    info!("Restored temporarily-removed {}:{}", s.backend, s.name);
                    let mut state = self.state.lock().await;
                    state.add(
                        &s.backend,
                        &s.name,
                        s.version.clone(),
                        std::collections::HashMap::new(),
                        Some("imperative".into()),
                        false,
                    );
                    state.clear_suspension(&s.backend, &s.name);
                }
                Err(e) => {
                    warn!(
                        "Kernel: could not restore {}:{} ({}); dropping the suspension.",
                        s.backend, s.name, e
                    );
                    self.state
                        .lock()
                        .await
                        .clear_suspension(&s.backend, &s.name);
                }
            }
        }
        self.state.lock().await.save()?;
        Ok(())
    }

    /// Restore every package suspended under a given ephemeral shell session (called when
    /// that ghost shell exits). Same warn-and-move-on contract as the timed sweep.
    pub async fn restore_session_suspensions(&self, session_id: &str) -> Result<()> {
        let owned = { self.state.lock().await.get_session_suspensions(session_id) };
        for s in owned {
            match self.restore_package(&s.backend, &s.name).await {
                Ok(()) => {
                    info!(
                        "Restored session-suspended {}:{} on shell exit",
                        s.backend, s.name
                    );
                    let mut state = self.state.lock().await;
                    state.add(
                        &s.backend,
                        &s.name,
                        s.version.clone(),
                        std::collections::HashMap::new(),
                        Some("imperative".into()),
                        false,
                    );
                    state.clear_suspension(&s.backend, &s.name);
                }
                Err(e) => {
                    warn!(
                        "Kernel: could not restore session-suspended {}:{} ({}); dropping it.",
                        s.backend, s.name, e
                    );
                    self.state
                        .lock()
                        .await
                        .clear_suspension(&s.backend, &s.name);
                }
            }
        }
        self.state.lock().await.save()?;
        Ok(())
    }

    /// A [`GitManager`] scoped to the LiNix repo root (II.1), which holds `modules/`,
    /// `profiles/`, `active`, `priority` and `locks/`.
    ///
    /// Safety: `config_root()` never resolves to the current working directory — an empty or
    /// relative stored root falls back to the platform config dir — so git never operates on
    /// whatever repo the user happens to be standing in.
    pub fn git_manager(&self) -> crate::core::GitManager {
        crate::core::GitManager::new(self.config.config_root())
    }

    /// Auto-commit manifest/config changes IF the config dir is already a git repo. This is
    /// opt-in: users enable manifest version control by running `linix git init` once; until
    /// then this is a silent no-op. Never fails a command — a git hiccup is logged, not fatal.
    pub async fn git_autocommit(&self, message: &str) {
        if self.config.dry_run {
            return;
        }
        let git = self.git_manager();
        if !git.is_repo() {
            return;
        }
        match git.commit_all(message) {
            Ok(Some(hash)) => info!(
                "Git: committed manifest change {} ({})",
                &hash[..hash.len().min(8)],
                message
            ),
            Ok(None) => {} // nothing changed
            Err(e) => warn!("Git: auto-commit skipped: {}", e),
        }
    }

    pub async fn clean_orphans(&self) -> Result<()> {
        info!("Kernel: Commencing system-wide orphan pruning cycle.");
        let (mut cleaned, mut skipped, mut failed) = (0u32, 0u32, 0u32);
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                match upgradable.clean_orphans(backend.sudo_for_write()).await {
                    Ok(()) => cleaned += 1,
                    // A backend with no orphan concept is a benign skip, not a failure.
                    Err(Error::Unsupported(_)) => skipped += 1,
                    Err(e) => {
                        failed += 1;
                        debug!(
                            "Kernel: orphan cleanup failed for {}: {}",
                            backend.name(),
                            e
                        );
                    }
                }
            }
        }
        info!(
            "Kernel: orphan pruning complete — {} cleaned, {} not applicable, {} failed.",
            cleaned, skipped, failed
        );
        Ok(())
    }

    pub async fn prune_snapshots(&self, force: bool) -> Result<()> {
        let settings = &self.config.snapshots;
        let is_dry_run = if force { false } else { self.config.dry_run };
        info!(
            "Kernel: Commencing snapshot maintenance cycle (Limit: {} days / {} count).",
            settings.max_age_days, settings.max_count
        );
        self.snapshot_manager
            .prune_stale_snapshots(settings.max_age_days, settings.max_count, is_dry_run)
            .await
    }

    /// The backends this host uses, in priority order (II.6's `priority` file). Empty only
    /// when the file is missing or unreadable — a state the model refuses to resolve in
    /// anyway — in which case a caller treats "no filter" as "every available backend".
    ///
    /// This is what replaced `config.enabled_backends`/`hostname_backends`: one file, with
    /// `when` blocks for the per-host case, instead of a config section that expressed the
    /// same fact a second way.
    pub async fn priority_backends(&self) -> Vec<String> {
        let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;
        resolver
            .priority_for_host()
            .await
            .map(|p| p.order().to_vec())
            .unwrap_or_default()
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let enabled = self.priority_backends().await;
        let searcher = UniversalSearch::new(&self.registry, &self.config, enabled);
        searcher.search(query).await
    }

    pub async fn create_shim(&self, binary_name: &str, _source_spec: &str) -> Result<()> {
        let manager = self.shim_manager().await?;
        manager.create_shim(binary_name).await
    }
}
