//! What one invocation of Shall is made of, and nothing else.
//!
//! **`App` is a composition root, not a service.** It owns the collaborators a run needs, wires
//! them once, and hands out narrow views of itself — `Inventory`, `Declarations`, `Managers`,
//! `Leases`, the nine `apply` facets. It has no behaviour of its own, deliberately: every method
//! here is either construction or a one-line factory, so nothing can be written *on* `App` that
//! reaches twelve collaborators to use three.
//!
//! The rule that keeps it that way: **if it needs to decide something, it does not belong here.**
//! A method that reads `config` and asks `registry` a question is a facet; give it a type that
//! holds those two and can be built without an `App` at all.

use crate::app::adopt::Adopter;
use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::profile::ProfileManager;
use crate::app::run::Runner;
use crate::app::scheduler::SchedulerManager;
use crate::app::shell::EphemeralShell;
use crate::app::shim_manager::ShimManager;
use crate::app::snapshot_restore::SnapshotRestore;
use crate::app::sync::resolver::StateResolver;
use crate::app::sync::SyncEngine;
use crate::backends::{create_default_registry, BackendRegistry};
use crate::config::Config;
use crate::core::{CommandExecutor, Error, Journal, Result, SnapshotManager, StateRegistry};
use crate::utils::progress::{create_progress_reporter, ProgressReporter};

use super::{
    Backends, Declarations, Inventory, LuaHooks, Machinery, Managers, MetricsCollector, Vcs,
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::debug;

pub struct App {
    pub config: Arc<Config>,
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
    /// What this command has removed so far, so a guard ceiling is a budget for the command and
    /// not for each of its four removal phases. One `App` is one invocation, which is what makes
    /// this the right owner.
    pub reaping: Arc<crate::app::sync::guard::Reaping>,
    /// The registry paired with `priority`, resolved on first use.
    ///
    /// Private, and the only private field here — deliberately. While `registry` is reachable,
    /// any code can fan out to every backend on the machine and bypass the file that says which
    /// ones Shall may use; this is the accessor that cannot be bypassed, and it is one lazily
    /// resolved answer rather than five identical file reads per command.
    backends: tokio::sync::OnceCell<Backends>,
}

impl App {
    /// `state_path` overrides where Shall's own data lives. `None` means the real data
    /// dir; a test passes a temp path so it never touches — or accumulates in — the
    /// user's.
    pub async fn new_with_executor_and_state_path(
        config: Config,
        executor: CommandExecutor,
        state_path: Option<PathBuf>,
    ) -> Result<Self> {
        debug!("starting up");

        let hooks = Arc::new(LuaHooks::new(&config)?);

        // The journal lives beside the registry: both are Shall's record of what it did,
        // so isolating one and not the other left the WAL pointing at real user data.
        let journal_dir = state_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.to_path_buf());

        // Overlapped, because none of these four needs any of the others. Startup was a
        // straight line of independent I/O — the backend registrations, a state-file read, the
        // snapshot provider probe, the WAL open — run for `shall list` as much as for `shall
        // sync`. The state load and the WAL open are file reads; the snapshot probe asks the
        // machine what it can snapshot with; the registry builds ~48 backends.
        //
        // **`build_registry` is polled LAST, and the order is the whole mechanism.** It is an
        // `async fn` with no `.await` in it — 103 lines of straight-line construction — so it
        // never yields, and nothing after it in this tuple can be polled until it returns.
        // Placed first, it held `probe_snapshots` unpolled for its entire duration and the
        // "overlap" was decoration: the instrumented run reported four futures all finishing at
        // 213.6ms, which was one future's cost wearing four labels (AU7). Placed last, every
        // future that can hand its work to another thread has already done so.
        //
        // The registry costs ~5ms now that no backend calibrates a clock in its constructor
        // (AU3) — which is why this is an ordering note and not a `spawn_blocking`.
        let load_state = async {
            match state_path {
                Some(path) => tokio::task::spawn_blocking(move || StateRegistry::load_from(&path)),
                None => tokio::task::spawn_blocking(StateRegistry::load_default),
            }
            .await
            .map_err(|e| Error::Other(format!("Kernel Thread Panic during state load: {}", e)))?
        };
        let open_journal = {
            let dir = journal_dir.clone();
            async move {
                tokio::task::spawn_blocking(move || match dir {
                    Some(d) => Journal::at(d.join(Journal::FILE_NAME)),
                    None => Journal::new(),
                })
                .await
                .map_err(|e| Error::Other(format!("Kernel Thread Panic opening the WAL: {}", e)))?
            }
        };
        let build_registry = async {
            Ok::<_, Error>(create_default_registry(executor.clone(), &config, hooks.clone()).await)
        };
        let probe_snapshots =
            async { Ok::<_, Error>(SnapshotManager::new(executor.clone(), &config).await) };

        let (state_registry, journal, snapshot_manager, registry) =
            tokio::try_join!(load_state, open_journal, probe_snapshots, build_registry)?;

        let registry = Arc::new(registry);
        let progress = create_progress_reporter(config.show_progress);
        let state = Arc::new(Mutex::new(state_registry));
        let snapshot_manager = Arc::new(snapshot_manager);
        let journal = Arc::new(Mutex::new(journal));

        let scheduler = Arc::new(SchedulerManager::new()?);
        let config_arc = Arc::new(config);

        let diagnostics = Arc::new(FailureDiagnosticEngine::init(&config_arc).await);

        debug!("ready");

        Ok(Self {
            config: config_arc,
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
            reaping: Arc::new(crate::app::sync::guard::Reaping::new()),
            backends: tokio::sync::OnceCell::new(),
        })
    }

    pub async fn new(config: Config) -> Result<Self> {
        let mut executor = CommandExecutor::new(config.dry_run, config.verbose);
        executor.set_installed_cache(config.installed_cache_secs);
        Self::new_with_executor_and_state_path(config, executor, None).await
    }

    /// The same app with one setting changed, sharing everything a run has already paid for.
    ///
    /// **Written here because a caller could not write it.** The one place that needed it —
    /// a test wanting `--yes` — rebuilt `App` field by field, twelve of them, which is the god
    /// object charging rent: every field added to the struct had to be added there too, and
    /// forgetting one silently shares the wrong state. Now the struct has a private field and
    /// that literal does not compile at all, which is the point of the private field.
    pub fn reconfigured(&self, edit: impl FnOnce(&mut Config)) -> Self {
        let mut config = (*self.config).clone();
        edit(&mut config);
        Self {
            config: Arc::new(config),
            registry: self.registry.clone(),
            executor: self.executor.clone(),
            metrics: self.metrics.clone(),
            progress: self.progress.clone(),
            hooks: self.hooks.clone(),
            state: self.state.clone(),
            snapshot_manager: self.snapshot_manager.clone(),
            journal: self.journal.clone(),
            diagnostics: self.diagnostics.clone(),
            scheduler: self.scheduler.clone(),
            reaping: self.reaping.clone(),
            // **Deliberately not carried over.** The edit may name a different config repo, and
            // a resolved `priority` is an answer about the repo it was read from — reusing it
            // would answer the new run's question with the old run's file.
            backends: tokio::sync::OnceCell::new(),
        }
    }

    /// The backends this run may use, in `priority` order (II.6's `priority` file).
    ///
    /// **Resolved once per process.** Reading `priority` costs a file read and a host-facts
    /// resolution, and every fan-out in the program wants the answer — the accessor this
    /// replaced was called five times per `check`.
    ///
    /// **And it no longer swallows.** It used to end in `.unwrap_or_default()`, so a `priority`
    /// that would not resolve became an *empty list* — which `UniversalSearch` then read as
    /// *every available backend*, on the stated premise that only a missing file could produce
    /// one. Two swallowed answers composing into the exact inversion of the file's own rule.
    /// The failure is carried into [`Backends`] and refused where the question is asked.
    pub async fn backends(&self) -> &Backends {
        self.backends
            .get_or_init(|| async {
                let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;
                let priority = resolver
                    .priority_for_host()
                    .await
                    .map_err(|e| e.to_string());
                Backends::new(self.registry.clone(), priority)
            })
            .await
    }

    /// A resolver over this app's config and registry — what a name means here, which managers
    /// this host may use, and what the manifest resolves to.
    pub async fn resolver(&self) -> StateResolver<'_> {
        StateResolver::new(&self.config, self.registry.clone(), false).await
    }

    /// What this machine has installed, asked of every manager the model uses.
    pub async fn inventory(&self) -> Inventory<'_> {
        Inventory {
            config: &self.config,
            registry: &self.registry,
            state: &self.state,
            backends: self.backends().await,
        }
    }

    /// Writing a line into your files, and taking one back out.
    pub fn declarations(&self) -> Declarations<'_> {
        Declarations {
            config: &self.config,
            registry: &self.registry,
        }
    }

    /// `update` and `upgrade`: the whole `priority` list at once.
    pub async fn managers(&self) -> Managers<'_> {
        Managers {
            config: &self.config,
            metrics: &self.metrics,
            snapshot_manager: &self.snapshot_manager,
            backends: self.backends().await,
        }
    }

    /// Git over the manifests (II.1), never over the directory the user is standing in.
    pub fn vcs(&self) -> Vcs<'_> {
        Vcs {
            config: &self.config,
        }
    }

    /// Every package frozen against an upgrade, from the ledger **and** from the manifest.
    pub async fn holds(&self) -> crate::app::holds::Holds {
        crate::app::holds::Holds::assemble(&self.resolver().await, &self.state).await
    }

    pub async fn adopter(&self) -> Adopter {
        Adopter::new(
            self.backends().await.clone(),
            self.state.clone(),
            &self.config,
        )
    }

    /// What it takes to change this machine, as one value.
    ///
    /// **The three engines below take this and nothing else.** Each used to name the same
    /// eleven collaborators in its own order, so a new field meant four signatures, six call
    /// sites, and no way for the compiler to notice one that was missed.
    pub fn machinery(&self) -> Machinery {
        Machinery {
            config: self.config.clone(),
            registry: self.registry.clone(),
            executor: self.executor.clone(),
            metrics: self.metrics.clone(),
            progress: self.progress.clone(),
            hooks: self.hooks.clone(),
            snapshot_manager: self.snapshot_manager.clone(),
            journal: self.journal.clone(),
            state: self.state.clone(),
            diagnostics: self.diagnostics.clone(),
            reaping: self.reaping.clone(),
        }
    }

    pub fn shell(&self) -> EphemeralShell {
        EphemeralShell::new(self.machinery())
    }

    pub fn profile_manager(&self) -> ProfileManager {
        ProfileManager::new(self.machinery())
    }

    pub fn snapshot_restore(&self) -> SnapshotRestore {
        SnapshotRestore::new(self.snapshot_manager.clone(), self.state.clone())
    }

    pub fn runner(&self) -> Runner {
        Runner::new(
            self.registry.clone(),
            self.config.clone(),
            self.journal.clone(),
        )
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
            registry: &self.registry,
            reaping: &self.reaping,
        }
    }

    pub fn dotfiles(&self) -> crate::app::Dotfiles<'_> {
        crate::app::Dotfiles {
            config: &self.config,
            registry: &self.registry,
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
            journal: &self.journal,
        }
    }

    pub fn extras(&self) -> crate::app::Extras<'_> {
        crate::app::Extras {
            config: &self.config,
            executor: &self.executor,
            registry: &self.registry,
            scheduler: &self.scheduler,
            reaping: &self.reaping,
        }
    }

    pub fn leases(&self) -> crate::app::Leases<'_> {
        crate::app::Leases {
            config: &self.config,
            registry: &self.registry,
            state: &self.state,
            journal: &self.journal,
            reaping: &self.reaping,
        }
    }

    pub fn sync_engine(&self) -> SyncEngine {
        SyncEngine::new(self.machinery())
    }
}
