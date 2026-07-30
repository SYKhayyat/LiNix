use crate::app::adopt::Adopter;
use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::profile::ProfileManager;
use crate::app::run::Runner;
use crate::app::scheduler::notify::NotificationManager;
use crate::app::scheduler::SchedulerManager;
use crate::app::shell::EphemeralShell;
use crate::app::shim_manager::ShimManager;
use crate::app::snapshot_restore::SnapshotRestore;
use crate::app::sync::resolver::StateResolver;
use crate::app::sync::SyncEngine;
use crate::backends::{create_default_registry, BackendRegistry};
use crate::config::grammar::Origin;
use crate::config::Config;
use crate::core::{
    CommandExecutor, Error, Journal, Package, PackageCache, PackageSpec, Result, SnapshotManager,
    StateRegistry,
};
use crate::utils::progress::{create_progress_reporter, ProgressReporter};

use super::{LuaHooks, MetricsCollector, UniversalSearch};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, instrument, warn};

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
        debug!("starting up");

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

        debug!("ready");

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

    pub fn adopter(&self) -> Adopter {
        Adopter::new(self.registry.clone(), self.state.clone(), &self.config)
    }

    pub fn shell(&self) -> EphemeralShell {
        EphemeralShell::new(
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

    pub fn snapshot_restore(&self) -> SnapshotRestore {
        SnapshotRestore::new(self.snapshot_manager.clone(), self.state.clone())
    }

    pub fn runner(&self) -> Runner {
        Runner::new(self.registry.clone(), self.config.clone())
    }

    pub async fn shim_manager(&self) -> Result<ShimManager> {
        ShimManager::with_bin_dir(self.config.bin_dir.clone()).await
    }

    pub fn repositories(&self) -> crate::app::Repositories<'_> {
        crate::app::Repositories {
            config: &self.config,
            registry: &self.registry,
        }
    }

    pub fn dependents(&self) -> crate::app::Dependents<'_> {
        crate::app::Dependents {
            config: &self.config,
            registry: &self.registry,
            executor: &self.executor,
        }
    }

    pub fn schedules(&self) -> crate::app::Schedules<'_> {
        crate::app::Schedules {
            config: &self.config,
            executor: &self.executor,
            scheduler: &self.scheduler,
        }
    }

    pub fn firewall(&self) -> crate::app::Firewall<'_> {
        crate::app::Firewall {
            config: &self.config,
            executor: &self.executor,
        }
    }

    pub fn dotfiles(&self) -> crate::app::Dotfiles<'_> {
        crate::app::Dotfiles {
            config: &self.config,
            executor: &self.executor,
        }
    }

    pub fn bootstrap(&self) -> crate::app::Bootstrap<'_> {
        crate::app::Bootstrap {
            config: &self.config,
            executor: &self.executor,
            registry: &self.registry,
        }
    }

    pub fn prereqs(&self) -> crate::app::Prereqs<'_> {
        crate::app::Prereqs {
            config: &self.config,
            executor: &self.executor,
            registry: &self.registry,
        }
    }

    pub fn execs(&self) -> crate::app::Execs<'_> {
        crate::app::Execs {
            config: &self.config,
            executor: &self.executor,
            registry: &self.registry,
        }
    }

    pub fn extras(&self) -> crate::app::Extras<'_> {
        crate::app::Extras {
            config: &self.config,
            executor: &self.executor,
            registry: &self.registry,
            scheduler: &self.scheduler,
        }
    }

    pub fn leases(&self) -> crate::app::Leases<'_> {
        crate::app::Leases {
            config: &self.config,
            registry: &self.registry,
            state: &self.state,
        }
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

    /// What this machine is, plus this run's variables — the facts every `when` in your
    /// files is evaluated against (IX.6). Anything that reads `active` needs these, not the
    /// detected ones on their own.
    pub async fn host_facts(&self) -> Result<crate::config::parser::HostFacts> {
        StateResolver::new(&self.config, self.registry.clone(), false)
            .await
            .facts_for_host()
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

    /// Refuse a line the model would reject on every later read, before it reaches a file.
    ///
    /// A backend that is not in `priority` is not a package that failed to install — it is a
    /// line nothing will ever parse, and once written it is a hard error for `status`,
    /// `plan`, `check` and every install after it, until a human edits the file. `install`
    /// writes first and syncs second on purpose (S15), so this is the only place the check
    /// can happen before the write.
    ///
    /// Static only: the grammar, the alias table and V.15's priority check. A bare name is
    /// NOT probed here — that costs a search per manager, and a name nothing claims is a
    /// different failure with a different cure (`Unresolvable`, withdrawn after the sync).
    pub async fn reject_unusable_line(&self, line: &str) -> Result<()> {
        StateResolver::new(&self.config, self.registry.clone(), false)
            .await
            .validate_line(line)
            .await
    }

    /// "Added" when the file changed, "Would add" when a preview only says it would.
    fn edit_verb(&self, done: &'static str, planned: &'static str) -> &'static str {
        if self.config.dry_run {
            planned
        } else {
            done
        }
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
        self.reject_unusable_line(line).await?;
        let vocab = self.vocabulary().await?;
        let layout = self.config.layout();
        let target = match into {
            Some(name) => crate::model::Target::parse(name, &Origin::argument())?,
            None => landing.target(),
        };
        let edit = crate::model::Editor::new(
            &layout,
            &vocab,
            self.host_facts().await?,
            crate::model::Writes::for_run(self.config.dry_run),
        )
        .add(&target, line)
        .map_err(Error::from)?;
        info!("{}", edit.describe(self.edit_verb("Added", "Would add")));
        Ok(edit)
    }

    /// Whether any active file declares this package.
    ///
    /// Asked through the resolver, so "declared" means the same thing here as it does to
    /// `sync` — a second definition of declared is a second answer.
    pub async fn declares(&self, target: &str) -> Result<bool> {
        let vocab = self.vocabulary().await?;
        let layout = self.config.layout();
        let facts = self.host_facts().await?;
        let files = crate::model::active_module_files(&layout, &vocab, &facts);
        // Reads only; a `Writes` it never uses is still the honest one to hand it.
        let editor =
            crate::model::Editor::new(&layout, &vocab, facts, crate::model::Writes::Planned);
        Ok(editor.declares_in(&files, target))
    }

    /// Move a declared package to `new_backend` by rewriting its line in place (II.8's
    /// `teleport`), and say which files changed. Empty result = the package is declared in no
    /// active file, which the caller reports rather than silently doing nothing.
    pub async fn retarget(
        &self,
        target_pkg: &str,
        new_backend: &str,
    ) -> Result<Vec<crate::model::Edit>> {
        // The same write-then-discover fault as `declare`: a move to a manager `priority`
        // does not list rewrites the line into one nothing can parse, and the package it
        // came from is already gone from the file.
        self.reject_unusable_line(&format!("{}:{}", new_backend, target_pkg))
            .await?;
        let vocab = self.vocabulary().await?;
        let layout = self.config.layout();
        let facts = self.host_facts().await?;
        let files = crate::model::active_module_files(&layout, &vocab, &facts);
        let edits = crate::model::Editor::new(
            &layout,
            &vocab,
            facts,
            crate::model::Writes::for_run(self.config.dry_run),
        )
        .retarget_backend(&files, target_pkg, new_backend)
        .map_err(Error::from)?;
        for e in &edits {
            info!("{}", e.describe(self.edit_verb("Moved", "Would move")));
        }
        Ok(edits)
    }

    /// Remove a package's declaration from every file the active profiles reach (II.8's
    /// `uninstall`), and say which files changed.
    pub async fn undeclare(&self, target_pkg: &str) -> Result<Vec<crate::model::Edit>> {
        let vocab = self.vocabulary().await?;
        let layout = self.config.layout();
        let facts = self.host_facts().await?;
        let files = crate::model::active_module_files(&layout, &vocab, &facts);
        let edits = crate::model::Editor::new(
            &layout,
            &vocab,
            facts,
            crate::model::Writes::for_run(self.config.dry_run),
        )
        .remove_from(&files, target_pkg)
        .map_err(Error::from)?;
        for e in &edits {
            info!("{}", e.describe(self.edit_verb("Removed", "Would remove")));
        }
        Ok(edits)
    }

    #[instrument(skip(self))]
    pub async fn resolve_spec(&self, spec_str: &str) -> Result<Vec<PackageSpec>> {
        self.resolver().await.resolve_spec(spec_str).await
    }

    /// A resolver over this app's config and registry. One construction, so a caller that needs
    /// two answers from the model does not build two resolvers that can disagree.
    pub async fn resolver(&self) -> StateResolver<'_> {
        StateResolver::new(&self.config, self.registry.clone(), false).await
    }

    /// The backend a `backend:name` string names, if it names one. `None` for a bare name.
    pub async fn declared_backend(&self, spec_str: &str) -> Result<Option<String>> {
        self.resolver().await.declared_backend(spec_str).await
    }

    /// The `(backend, name)` a `service:`/`link:`/`setting:` string denotes — the three
    /// prefixes that are also backends, and therefore the three `list` prints as two columns.
    pub async fn queried_resource(&self, spec_str: &str) -> Result<Option<(String, String)>> {
        self.resolver().await.queried_resource(spec_str).await
    }

    /// Refuse a `backend:name` argument whose prefix is not a backend (Q9).
    ///
    /// Q9 ruled that every verb taking a backend name refuses an unknown one, and listed the
    /// four that take it as a `--backend` flag — "checked from the code rather than from the one
    /// that was reported". The `backend:name` *spec* form was not in that enumeration, so the
    /// ruling was applied to half its surface: `linix hold nosuchbackend:foo` recorded a hold
    /// against a manager that does not exist and answered `Held 1 package(s).` at exit 0.
    ///
    /// A real backend that cannot run here is a different answer and stays exit 0 — Q9 clause 3,
    /// and `require_known_backend` is where that distinction lives.
    pub async fn require_known_spec_backends(&self, specs: &[String]) -> Result<()> {
        for spec in specs {
            let named = self.declared_backend(spec).await?;
            self.require_known_backend(named.as_deref())?;
        }
        Ok(())
    }

    /// Refresh every backend's metadata, and do not let one stop the rest.
    ///
    /// `?` on the first failure meant a single manager that could not refresh — a plugin
    /// missing, a repo down — silently skipped every backend after it, and the ones that
    /// did refresh went unmentioned. Each failure is named and the command still reports
    /// one, because a refresh that half-happened is not a refresh that worked.
    pub async fn update(&self) -> Result<()> {
        use futures::stream::{self, StreamExt};
        info!("refreshing package metadata");
        // Each backend's refresh is an independent network fetch (`apt update`, `brew update`,
        // …) — seconds of waiting with nothing shared between them. Overlap the waits, capped
        // at `max_parallel`. Unlike `upgrade`, this changes no package, so concurrent runs
        // cannot contend on a package database.
        let cap = self.config.max_parallel.max(1);
        let failed: Vec<String> = stream::iter(self.registry.available())
            .map(|backend| async move {
                let upgradable = backend.as_upgradable()?;
                match upgradable.update(backend.sudo_for_write()).await {
                    Ok(()) => None,
                    Err(e) => {
                        warn!("{}: could not refresh — {}", backend.name(), e);
                        Some(format!("{} ({})", backend.name(), e))
                    }
                }
            })
            .buffer_unordered(cap)
            .filter_map(|x| async move { x })
            .collect()
            .await;
        if failed.is_empty() {
            return Ok(());
        }
        Err(Error::Other(format!(
            "{} backend(s) could not refresh their metadata; the rest were refreshed: {}",
            failed.len(),
            failed.join("; ")
        )))
    }

    pub async fn upgrade(&self) -> Result<()> {
        let _ = self
            .snapshot_manager
            .auto_snapshot(crate::core::snapshot::SnapshotLabel::PreUpgrade)
            .await?;
        info!("upgrading all packages");
        // The same rule as `update`, and for the same reason: one manager that cannot
        // upgrade must not silently cancel every manager after it in the list.
        let mut failed: Vec<String> = Vec::new();
        for backend in self.registry.available() {
            if let Some(upgradable) = backend.as_upgradable() {
                if let Err(e) = upgradable.upgrade(backend.sudo_for_write()).await {
                    warn!("{}: could not upgrade — {}", backend.name(), e);
                    failed.push(format!("{} ({})", backend.name(), e));
                }
            }
        }
        self.metrics
            .print_summary(crate::app::metrics::Narration::Change);
        if failed.is_empty() {
            return Ok(());
        }
        Err(Error::Other(format!(
            "{} backend(s) could not upgrade; the rest were upgraded: {}",
            failed.len(),
            failed.join("; ")
        )))
    }

    /// Refuse a `--backend` name nothing claims, and say so when a real one is not installed
    /// here. Returns `true` when the named backend can answer right now.
    ///
    /// `install nosuchbackend:foo` refused loudly and named the file to edit; `list -b
    /// nosuchbackend` printed nothing and exited 0 — which is byte-identical to a real backend
    /// with nothing installed, so a typo was reported, in the program's own voice, as "that
    /// manager is empty". Owner ruling 2026-07-28 (Q9): `list` refuses the way `install` does.
    ///
    /// The second answer is the one that is easy to miss. `apt` on Windows is a real backend
    /// that cannot run here, and it produced the same silence as the typo. Those are different
    /// facts and they now read differently — but only the typo is an error, because a name that
    /// is genuinely a backend is not a mistake the user made.
    ///
    /// The message is `install`'s, deliberately: two spellings of one refusal is how E18's
    /// family started.
    pub fn require_known_backend(&self, name: Option<&str>) -> Result<bool> {
        let Some(name) = name else {
            return Ok(true);
        };
        match self.registry.get(name) {
            None => Err(Error::Config(format!(
                "`{}` is not a backend LiNix uses\n  \
                 add `{}` to your `priority` file, or check the spelling. Not listed means \
                 LiNix does not use it at all.",
                name, name
            ))),
            Some(b) => {
                if b.is_available() {
                    Ok(true)
                } else {
                    tracing::warn!(
                        "`{}` is a manager LiNix knows, but it is not installed on this \
                         machine — so there is nothing for it to report. `linix check health` \
                         says which managers are ready here.",
                        name
                    );
                    Ok(false)
                }
            }
        }
    }

    pub async fn list(&self, backend_filter: Option<&str>) -> Result<Vec<Package>> {
        let backends: Vec<_> = self
            .registry
            .available()
            .into_iter()
            .filter(|b| backend_filter.is_none_or(|f| b.name() == f))
            .collect();
        // Every backend's lister is a separate process (`apt list`, `cargo install --list`,
        // …) with nothing to share, so querying them one after another is latency the machine
        // is not spending — it is waiting. Fan out, bounded by `max_parallel`.
        let results = self
            .query_backends_concurrently(backends, |q| async move {
                q.list_installed().await.unwrap_or_default()
            })
            .await;
        Ok(results.into_iter().flatten().collect())
    }

    pub async fn get_info(&self, package_name: &str) -> Result<Option<Package>> {
        // An explicit `backend:name` narrows the question to one manager. This used to hand
        // the raw string to every backend, so `linix info cargo:ripgrep` asked each of them
        // for a package literally named "cargo:ripgrep" — a name none of them has. That is
        // both the wrong question and the slow one: every manager was probed, and the answer
        // was always "not found", while `linix search ripgrep` in the same tree found it.
        //
        // Split by the one parser (`resolve_spec`, which goes through the grammar), never by
        // `split_once(':')` here — a second place that decides what a prefix means is the bug
        // CLAUDE.md names, and C13 records six parsers that had it.
        // Does the string name a manager, and is it one? Asked first, and refused with the
        // sentence `install` and `list --backend` already use — from the same function, so
        // there is one answer to one question. `info nosuchbackend:foo` used to reach the
        // fan-out below and ask every manager on the machine for a package literally named
        // `nosuchbackend:foo`: the wrong answer ("not installed" — the *manager* does not
        // exist), arrived at slowly, at exit 0 (N-3).
        // Note the `?`. The grammar is what rejects an unknown prefix — `parse_prefix` writes
        // the sentence — and every version of this bug has been someone dropping that error on
        // the floor: `get_info` had `if let Ok(specs) = resolve_spec(…)`, and the refusal it
        // discarded was the answer.
        let named_backend = self.declared_backend(package_name).await?;
        // The registry's half of the same question: a name `priority` lists and this build has
        // no backend for.
        self.require_known_backend(named_backend.as_deref())?;

        // `service:`, `link:` and `setting:` are each a grammar prefix AND a registered
        // backend, and `list` prints them as those two columns — so a string copied out of a
        // listing parses as a typed resource statement rather than as `backend:name`, and
        // everything below understands only packages. `list` reported
        // `service:com.apple.SafariHistoryServiceAgent` and `info` about that exact name said
        // "not installed" (R-4). A list that disagrees with the machine breaks the one thing it
        // promises, so this is answered before the package path rather than after it.
        if let Ok(Some((backend_name, resource))) = self.queried_resource(package_name).await {
            if let Some(backend) = self.registry.get(&backend_name) {
                if let Some(q) = backend.as_queryable() {
                    // A resource the backend does not have answers `None`, the same as any
                    // other name it does not carry — the point is that it was *asked*.
                    return q.info(&resource).await;
                }
            }
        }

        // A resolution failure is not an error here: `info` answers about the machine, and a
        // name no manager *carries* can still be installed on it. Only the prefix check above
        // is fatal.
        let specs = self.resolve_spec(package_name).await.unwrap_or_default();
        for spec in &specs {
            let Some(backend) = self.registry.get(&spec.backend) else {
                continue;
            };
            let Some(q) = backend.as_queryable() else {
                continue;
            };
            if let Ok(Some(found)) = q.info(&spec.name).await {
                return Ok(Some(found));
            }
        }

        // The user named the manager, and it does not have it. Asking a different one would
        // answer a question nobody asked — `info cargo:ripgrep` must never report the choco
        // copy.
        if named_backend.is_some() {
            return Ok(None);
        }

        // A bare name: *which* manager has it installed is a fact about this machine, and
        // `priority` order is not that fact. The resolver picks by priority, so `info hexyl`
        // asked `choco` (first in `priority`, and it carries the name), choco had nothing
        // installed, and LiNix reported a package the user has under `cargo` as absent — while
        // `list` reported it present. Two read commands must never contradict each other about
        // the machine.
        //
        // Asked of every backend at once, and the first answer wins. Serial, this waited on
        // every manager that did not have it before reaching the one that did.
        let name = specs
            .first()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| package_name.to_string());
        let backends = self.registry.available();
        let found = self
            .query_backends_concurrently(backends, move |q| {
                let name = name.clone();
                async move { q.info(&name).await.ok().flatten() }
            })
            .await;
        Ok(found.into_iter().flatten().next())
    }

    /// Everything installed that LiNix does not manage — the dependency closure included.
    ///
    /// **This is not `unmanaged` (II.8), which is "what `adopt` would adopt".** They are two
    /// questions with very different answers: on a stock Ubuntu this is ~476 packages and
    /// `adopt` is ~103. Only `purge-unmanaged` wants this one, and its whole job is deleting
    /// all of it (II.11) — which is why the ratio check exists.
    pub async fn installed_but_unmanaged(&self) -> Result<Vec<Package>> {
        let backends = self.registry.available();
        let listed = self
            .query_backends_concurrently(backends, |q| async move {
                q.list_installed().await.unwrap_or_default()
            })
            .await;
        // D5: a `.deb`/`.rpm` a download backend handed to a system manager is listed by that
        // manager as installed, but a download declaration owns it — so it is not unmanaged, and
        // `purge-unmanaged` must defer to the recorded installer rather than delete it. Match by
        // name: the installer is `dpkg`/`rpm`, the lister is `apt`/`dnf`, and the name is the one
        // identity they share.
        let owned = self.owned_system_package_names().await;
        // The managed check touches the state lock once, after the process work is done,
        // rather than holding it across every backend's query.
        let state = self.state.lock().await;
        Ok(listed
            .into_iter()
            .flatten()
            .filter(|pkg| !state.is_managed(&pkg.backend, &pkg.name))
            .filter(|pkg| !owned.contains(&pkg.name))
            .collect())
    }

    /// Every system package a download backend (`github:`/`web:`) installed through a second
    /// manager (D5), by name. Used to keep those packages out of the unmanaged crawl so they are
    /// neither double-counted nor purged out from under the declaration that owns them.
    pub async fn owned_system_package_names(&self) -> std::collections::HashSet<String> {
        let backends = self.registry.available();
        let owned =
            self.query_backends_concurrently(backends, |q| async move {
                q.owned_system_packages().await
            })
            .await;
        owned
            .into_iter()
            .flatten()
            .map(|(_installer, pkg)| pkg)
            .collect()
    }

    /// Run one read-only query against every queryable backend concurrently, capped at
    /// `max_parallel`, returning each backend's result in registry order (a failed or absent
    /// query contributes nothing). One place for the fan-out so `list`, `info` and the
    /// unmanaged crawl cannot drift in how they bound concurrency or swallow errors.
    async fn query_backends_concurrently<T, F, Fut>(
        &self,
        backends: Vec<Arc<crate::core::BackendCapabilities>>,
        query: F,
    ) -> Vec<T>
    where
        T: Send + 'static,
        F: Fn(Arc<dyn crate::core::Queryable>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = T> + Send,
    {
        use futures::stream::{FuturesOrdered, StreamExt};
        let cap = self.config.max_parallel.max(1);
        let query = Arc::new(query);
        let mut ordered = FuturesOrdered::new();
        let mut queued = backends.into_iter().filter_map(|b| {
            b.as_queryable().cloned().map(|q| {
                let query = query.clone();
                async move { query(q).await }
            })
        });

        let mut out = Vec::new();
        for _ in 0..cap {
            if let Some(fut) = queued.next() {
                ordered.push_back(fut);
            }
        }
        while let Some(res) = ordered.next().await {
            out.push(res);
            if let Some(fut) = queued.next() {
                ordered.push_back(fut);
            }
        }
        out
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

    pub async fn prune_snapshots(&self, force: bool) -> Result<()> {
        let is_dry_run = if force { false } else { self.config.dry_run };
        let policy = self.config.snapshot_retention();
        info!(
            "pruning snapshots (keep_last {} / keep_days {})",
            policy.keep_last, policy.keep_days
        );
        // One retention engine: the same `RetentionPolicy` + `prune_with_policy` that `sync`
        // uses, which always keeps the most-recent snapshot (a floor the old
        // `prune_stale_snapshots` lacked — it could delete the last rollback point) and only
        // ever reaps LiNix-owned snapshots.
        self.snapshot_manager
            .prune_with_policy(&policy, chrono::Utc::now(), is_dry_run)
            .await
            .map(|_| ())
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
}
