use crate::config::grammar::Origin;
use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::adopt::Adopter;
use crate::app::profile::ProfileManager;
use crate::app::run::Runner;
use crate::app::scheduler::notify::NotificationManager;
use crate::app::scheduler::SchedulerManager;
use crate::app::shell::EphemeralShell;
use crate::app::shim_manager::ShimManager;
use crate::app::sync::resolver::StateResolver;
use crate::app::sync::SyncEngine;
use crate::app::undo::UndoManager;
use crate::backends::{create_default_registry, BackendRegistry};
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

    pub fn undo_manager(&self) -> UndoManager {
        UndoManager::new(self.snapshot_manager.clone(), self.state.clone())
    }

    pub fn runner(&self) -> Runner {
        Runner::new(self.registry.clone(), self.config.clone())
    }

    pub async fn shim_manager(&self) -> Result<ShimManager> {
        ShimManager::with_bin_dir(self.config.bin_dir.clone()).await
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
        let edit = crate::model::Editor::new(&layout, &vocab, self.host_facts().await?)
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

    /// SEC3: the `link:` lines this run would place outside the home directory for the first
    /// time, as (line, destination) pairs.
    ///
    /// "First time" is asked of the destination, not of a ledger: `locks/extras.toml` keys a
    /// link by its *source*, so a line whose `@target` is edited to a system path is the same
    /// ledger entry it always was and would never be asked about. A destination that is not
    /// there yet is the run that creates it.
    pub fn outside_home_links(
        state: &crate::model::DesiredState,
        exists: &dyn Fn(&std::path::Path) -> bool,
    ) -> Vec<(String, std::path::PathBuf)> {
        use crate::config::grammar::Statement;

        state
            .extras
            .iter()
            .filter_map(|(stmt, _)| {
                let Statement::Link(name, opts) = stmt else {
                    return None;
                };
                // An unresolvable target is the install path's error to report, with its own
                // message; swallowing it here would turn it into a silent skip.
                let resolved = crate::backends::link::resolve_target(opts.one("target")?).ok()?;
                (crate::backends::link::is_outside_home(&resolved) && !exists(&resolved))
                    .then_some((format!("link:{}", name), resolved))
            })
            .collect()
    }

    /// Ask about those destinations before anything is applied.
    ///
    /// SEC3: `@target` is deliberately unconfined — an arbitrary destination is the link
    /// backend's purpose — so this asks rather than refuses, and no config key turns it off.
    /// What it buys is a beat between a pasted spec line and a system path.
    pub fn confirm_outside_home_links(&self, state: &crate::model::DesiredState) -> Result<()> {
        let targets = Self::outside_home_links(state, &|p| p.exists() || p.is_symlink());
        if targets.is_empty() {
            return Ok(());
        }

        println!("\nThese lines place files outside your home directory:");
        for (line, dest) in &targets {
            println!("  {}  ->  {}", line, dest.display());
        }

        if self.config.dry_run {
            println!("[DRY-RUN] a real run would ask you to confirm these destinations.");
            return Ok(());
        }
        if self.config.yes {
            return Ok(());
        }

        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return Err(Error::Other(format!(
                "refusing to place {} file(s) outside your home directory without \
                 confirmation in a non-interactive shell.\n\n\
                 What to do:\n  \
                 linix status        see every destination first\n  \
                 linix sync --yes    place them",
                targets.len()
            )));
        }

        let ok = dialoguer::Confirm::new()
            .with_prompt("Place these files?")
            .default(false)
            .interact()
            .map_err(|e| Error::Other(format!("could not ask for confirmation: {}", e)))?;
        if ok {
            Ok(())
        } else {
            Err(Error::Other("cancelled — nothing was changed.".to_string()))
        }
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
                Statement::Shim(name, opts) => {
                    // U19: a shim lands in `~/.local/bin`, which is this account's PATH and
                    // nobody else's. LiNix has no machine-wide shim directory yet, so a line
                    // asking for one is refused by name rather than quietly deploying a
                    // per-user shim under a declaration that says every account (P7).
                    if crate::model::scope::Scope::resolve(
                        opts.one("scope"),
                        crate::model::scope::Scope::User,
                    ) == crate::model::scope::Scope::System
                    {
                        return Err(Error::Validation(format!(
                            "{}: `shim:{}` asks for scope=system, and LiNix deploys shims only                              into this account's `~/.local/bin`. A per-user shim under a line                              that says every account would be the wrong answer quietly, so                              this is refused. Drop `@scope=system`.",
                            origin, name
                        )));
                    }
                    if self.config.dry_run {
                        info!("[DRY-RUN] would deploy shim `{}`", name);
                        continue;
                    }
                    info!("deploying `{}` ({})", name, origin);
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
                    info!("applying `{}` ({})", name, origin);
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
                Statement::Setting(name, opts) => {
                    let Some(b) = self.registry.get("setting") else {
                        warn!(
                            "{}: no settings adapter here — skipping `setting:{}`.",
                            origin, name
                        );
                        continue;
                    };
                    if self.config.dry_run {
                        info!("[DRY-RUN] would apply setting `{}`", name);
                        continue;
                    }
                    let Some(inst) = b.as_installable() else {
                        continue;
                    };
                    info!("Setting: applying `{}` ({})", name, origin);
                    let spec = spec_from_extra("setting", name, opts);
                    inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
                        .await?;
                }
                // dependents() yields only these four variants.
                _ => {}
            }
        }
        Ok(())
    }

    /// Provision the declared `schedule:` lines onto the OS scheduler (S21) — II.7's schedule
    /// phase, after packages and dependents. Each line is mapped to a `ScheduleConfig` (which
    /// validates it carries `cron` and `run`) and handed to the `SchedulerManager`. Declarative
    /// and idempotent: re-registering the same task each sync is how the system state is kept
    /// equal to what the `schedules` file says.
    pub async fn apply_schedules(&self, state: &crate::model::DesiredState) -> Result<()> {
        for (name, opts, origin) in state.schedules() {
            let cfg = crate::model::schedule::schedule_config(
                name,
                opts,
                origin,
                &self.config.guard.never_unattended,
            )?;
            if self.config.dry_run {
                info!(
                    "[DRY-RUN] would schedule `{}`: `{}` on `{}`",
                    name, cfg.command, cfg.cron
                );
                continue;
            }
            info!(
                "Schedule: provisioning `{}` ({}) — `{}` on `{}`",
                name, origin, cfg.command, cfg.cron
            );
            self.scheduler.provision(&self.executor, &cfg).await?;
        }
        Ok(())
    }

    /// Resolve one `exec:` line to the script LiNix would run, its content hash, and what the
    /// two ledgers say about it. Shared by the preview and the run so a plan cannot describe a
    /// decision the sync then makes differently.
    ///
    /// The path is taken relative to the config repo when it is not absolute — the script
    /// travels with the configuration that declares it, the way a `link:` source does.
    fn exec_plan(
        &self,
        script: &str,
        opts: &crate::config::grammar::Options,
        hooks: &crate::core::hook_lock::HookLedger,
        runs: &crate::core::ExecLedger,
    ) -> Result<(std::path::PathBuf, String, crate::model::exec::Decision)> {
        use crate::core::hook_lock::{exec_id, hash_script};

        let declared = std::path::Path::new(script);
        let path = if declared.is_absolute() {
            declared.to_path_buf()
        } else {
            self.config.config_root().join(declared)
        };
        let body = std::fs::read_to_string(&path).map_err(|e| {
            Error::Validation(format!(
                "`exec:{}` — cannot read the script at {} ({}). An `exec:` names a file the \
                 config carries; its contents are what LiNix hashes and runs.",
                script,
                path.display(),
                e
            ))
        })?;
        let hash = hash_script(&body);
        let decision = crate::model::exec::Decision::of(
            &hooks.verdict(&exec_id(script), &hash),
            runs.count(&hash),
            crate::core::Ceiling::read(opts.one("runs")),
        );
        Ok((path, hash, decision))
    }

    /// Print what each declared `exec:` will do, before anything happens (XIII.3's exit
    /// condition): the content hash, how many times that content has run here, and the
    /// decision that follows. Uses the same `exec_plan` the run uses, so the preview cannot
    /// describe one thing and the sync do another.
    ///
    /// A script that cannot be read is reported here rather than propagated: this is the
    /// preview, and the run raises the same problem as a real error a moment later.
    pub fn print_exec_plan(&self, state: &crate::model::DesiredState) {
        use crate::core::hook_lock::HookLedger;

        if !state.has_execs() {
            return;
        }
        let locks = self.config.layout().locks_dir();
        let (Ok(hooks), Ok(runs)) = (
            HookLedger::load(&HookLedger::path_in(&locks)),
            crate::core::ExecLedger::load(&crate::core::ExecLedger::path_in(&locks)),
        ) else {
            return;
        };
        println!("Scripts:");
        for (script, opts, origin) in state.execs() {
            match self.exec_plan(script, opts, &hooks, &runs) {
                Ok((_, hash, decision)) => {
                    println!("  exec:{}  ({})", script, origin);
                    println!("    {}", decision.describe(&hash));
                }
                Err(e) => println!("  exec:{}  ({}) — {}", script, origin, e),
            }
        }
    }

    /// The `[[bootstrap]]` rows this repo carries, if the file is approved (7c/U10).
    pub fn bootstrap_rows(&self) -> Vec<crate::model::bootstrap::BootstrapDef> {
        let layout = self.config.layout();
        let Some(body) = crate::backends::onboarder::read_approved_definitions(
            &layout.adapter_bootstrap_file(),
            &layout.locks_dir(),
        ) else {
            return Vec::new();
        };
        match toml::from_str::<crate::model::bootstrap::BootstrapFile>(&body) {
            Ok(f) => f.bootstrap,
            Err(e) => {
                warn!("ignoring adapters/bootstrap.toml: {}", e);
                Vec::new()
            }
        }
    }

    /// Offer to obtain a package manager the configuration declares and this machine lacks
    /// (7c). **Ask, then do** — the command is printed in full and confirmed before it runs.
    ///
    /// A bootstrap is usually a vendor's `curl | sh`, which is exactly the thing that must not
    /// run because a config file said so and nobody looked. It is approved through II.12 (the
    /// file) *and* confirmed at the moment it would run (the command), because those answer
    /// different questions: whether this repo may carry the instruction, and whether you want
    /// it executed on this machine now.
    ///
    /// Missing managers with no bootstrap row are left alone — that is the ordinary state of a
    /// machine that simply does not use them, not a fault to report.
    pub async fn offer_bootstrap(&self, state: &crate::model::DesiredState) -> Result<()> {
        use std::io::IsTerminal;

        let rows = self.bootstrap_rows();
        if rows.is_empty() {
            return Ok(());
        }
        let os = std::env::consts::OS;

        // Only managers this configuration actually asks for. A row for a manager nothing
        // declares is an offer nobody needs.
        let mut wanted: Vec<String> = state.packages.keys().cloned().collect();
        wanted.sort();
        wanted.dedup();

        for manager in wanted {
            // Registered AND runnable means there is nothing to obtain.
            if self
                .registry
                .get(&manager)
                .map(|b| b.is_available())
                .unwrap_or(false)
            {
                continue;
            }
            let Some(row) = crate::model::bootstrap::for_manager(&rows, &manager, os) else {
                continue;
            };

            println!(
                "\n`{}` is declared in your configuration and is not installed here.\n\
                 Your repo says it is obtained with:\n\n    {}\n",
                manager,
                row.command_line()
            );

            if self.config.dry_run {
                println!("[DRY-RUN] a real run would ask before running that.");
                continue;
            }
            if !self.config.yes {
                if !std::io::stdin().is_terminal() {
                    // Never in an unattended run: an installer that arrives with a pulled repo
                    // and executes because nobody was there to say no is the whole risk.
                    println!(
                        "Not asking in a non-interactive shell — run `linix sync` yourself, or \
                         install `{}` by hand.",
                        manager
                    );
                    continue;
                }
                let proceed = dialoguer::Confirm::new()
                    .with_prompt(format!("Run that to install {}?", manager))
                    .default(false)
                    .interact()
                    .unwrap_or(false);
                if !proceed {
                    println!("Left `{}` alone.", manager);
                    continue;
                }
            }

            let (program, args) = row.run.split_first().expect("a usable row has a command");
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            info!("obtaining `{}`...", manager);
            match self.executor.run(program, &refs, false).await {
                Ok(_) => info!("`{}` installed. The sync will use it.", manager),
                // Reported, not fatal: the rest of the sync is still worth doing, and the
                // packages that needed this manager will fail by name a moment later.
                Err(e) => warn!("could not obtain `{}`: {}", manager, e),
            }
        }
        Ok(())
    }

    /// Run the declared `exec:` scripts (XIII.3) — II.7's verb phase, after the packages and
    /// dependents a script is likely to depend on.
    ///
    /// Three things this does that a naive "run the command" would not: it refuses a script
    /// II.12 has not approved (a repo that can run code is the hook question with a different
    /// file name, and `-y` cannot approve); it runs a given *content* only as many times as its
    /// `@runs=` ceiling allows, so a settled sync executes nothing; and it records the run only
    /// when the script actually succeeded — a failed script has not happened, so the next sync
    /// must try it again.
    pub async fn apply_execs(&self, state: &crate::model::DesiredState) -> Result<()> {
        use crate::core::hook_lock::HookLedger;

        // No early return when nothing is declared: deleting the LAST `exec:` line is a real
        // change, and a teardown that only runs when something is still declared can never
        // undo the last one (S20 taught this for extras; it is the same shape here).
        let locks = self.config.layout().locks_dir();
        let hooks = HookLedger::load(&HookLedger::path_in(&locks))?;
        let runs_path = crate::core::ExecLedger::path_in(&locks);
        let mut runs = crate::core::ExecLedger::load(&runs_path)?;

        for (script, opts, origin) in state.execs() {
            let (path, hash, decision) = self.exec_plan(script, opts, &hooks, &runs)?;
            if let crate::model::exec::Decision::NeedsApproval(verdict) = &decision {
                // A refusal, not a warning: this is code from the configuration, and II.12's
                // whole point is that nothing runs it until a human has looked.
                return Err(Error::Validation(format!(
                    "{}: {}",
                    origin,
                    crate::core::hook_lock::refusal(
                        &crate::core::hook_lock::exec_id(script),
                        "exec script",
                        verdict
                    )
                )));
            }
            if !decision.will_run() {
                debug!("exec:{} — {}", script, decision.describe(&hash));
                continue;
            }
            if self.config.dry_run {
                info!("[DRY-RUN] would run exec:{} ({})", script, origin);
                continue;
            }
            info!("running exec:{} ({})", script, origin);
            self.run_exec_script(&path).await?;
            // Recorded only on success. A script that failed did not happen, and the next sync
            // must be free to try it again.
            runs.record_run(
                &hash,
                chrono::Utc::now().to_rfc3339(),
                script,
                opts.one("undo"),
            );
            runs.save(&runs_path)?;
        }

        self.undo_departed_execs(state, &mut runs, &runs_path).await?;
        Ok(())
    }

    /// Run the `@undo=` of every `exec:` whose line has gone away, then forget it (U3).
    ///
    /// **The undo is read from the ledger, not from the files**, because by the time it is
    /// needed the declaration has been deleted — that is what removal means. Reading the
    /// current config would find nothing and do nothing, which is the `link:` source-deletion
    /// mistake wearing a different hat.
    ///
    /// A script that declared no `@undo=` is simply forgotten: LiNix cannot invent an inverse,
    /// and pretending to would be worse than saying nothing. `plan` says so in those words.
    async fn undo_departed_execs(
        &self,
        state: &crate::model::DesiredState,
        runs: &mut crate::core::ExecLedger,
        runs_path: &std::path::Path,
    ) -> Result<()> {
        let declared = self.declared_exec_paths()?;
        // An unreadable configuration yields an empty set, which must never be read as "every
        // script departed" — that would run every undo on the machine because of a stray brace.
        if declared.is_empty() && state.has_execs() {
            return Ok(());
        }
        let departed = runs.departed(&declared);
        if departed.is_empty() {
            return Ok(());
        }
        for (hash, record) in departed {
            let name = record.script.as_deref().unwrap_or(&hash);
            let Some(undo) = record.undo.as_deref().filter(|u| !u.trim().is_empty()) else {
                debug!("`exec:{}` is no longer declared; it had no `undo`.", name);
                if !self.config.dry_run {
                    runs.forget(&hash);
                }
                continue;
            };
            if self.config.dry_run {
                info!("[DRY-RUN] would undo `exec:{}` with: {}", name, undo);
                continue;
            }
            info!("`exec:{}` is no longer declared — running its undo.", name);
            match self.run_shell_command(undo).await {
                Ok(()) => runs.forget(&hash),
                // Kept in the ledger on failure, so the next sync tries again rather than
                // forgetting an undo that never happened.
                Err(e) => warn!(
                    "could not undo `exec:{}` ({}); it stays recorded and the next sync will \
                     try again.",
                    name, e
                ),
            }
        }
        runs.save(runs_path)
    }

    /// Every `exec:` script path the configuration contains — read from the FILES, ignoring
    /// `when` and ignoring which profiles are active.
    ///
    /// **This is deliberately not `state.execs()`.** That answers *does this machine want it
    /// right now*, which is a different question with a dangerous difference: a `when` that
    /// went false would read as a deleted line and run its `@undo=` — the enrol script
    /// un-enrolling itself on the sync after it succeeded, which is the flapping failure
    /// XIII.3 spends a section warning about. Deactivating a profile is likewise not a
    /// deletion. Only removing the line from the file is, and only the file can say so.
    ///
    /// A file that cannot be parsed contributes nothing rather than being treated as empty:
    /// concluding "every exec: has departed" from a syntax error would run every undo on the
    /// machine because of a stray brace.
    fn declared_exec_paths(&self) -> Result<std::collections::BTreeSet<String>> {
        use crate::config::grammar::Statement;

        let mut out = std::collections::BTreeSet::new();
        let modules = self.config.layout().modules_dir();
        let Ok(entries) = std::fs::read_dir(&modules) else {
            return Ok(out);
        };
        let known = |name: &str| self.registry.get(name).is_some();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("txt") {
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(doc) = crate::config::grammar::parse_document(&path, &body, &known) else {
                // Unparseable: the resolver reports it properly elsewhere. Here it must not be
                // read as "this file declares nothing".
                warn!(
                    "{}: could not be parsed while looking for `exec:` lines; leaving its \
                     scripts recorded.",
                    path.display()
                );
                return Ok(std::collections::BTreeSet::new());
            };
            for (stmt, _, _) in doc.every_statement() {
                if let Statement::Exec(script, _) = stmt {
                    out.insert(script.clone());
                }
            }
        }
        Ok(out)
    }

    /// Run a command line through the platform's shell. Used only for `@undo=`, which is
    /// written as a command rather than a script path.
    async fn run_shell_command(&self, command: &str) -> Result<()> {
        #[cfg(windows)]
        let (program, args) = (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                command.to_string(),
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = ("sh", vec!["-c".to_string(), command.to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.executor.run(program, &refs, false).await.map(|_| ())
    }

    /// Execute one script through the platform's interpreter. `sh` on Unix and PowerShell on
    /// Windows, because a repo that must ship two spellings of every script is a repo that
    /// cannot be shared — which is the reason the file travels with the config at all.
    async fn run_exec_script(&self, path: &std::path::Path) -> Result<()> {
        let script = path.to_string_lossy().to_string();
        #[cfg(windows)]
        let (program, args) = (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                script,
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = ("sh", vec![script]);

        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.executor.run(program, &refs, false).await.map(|_| ())
    }

    /// Undo the extras that were applied but are no longer declared (S20). Extras had no
    /// record of what was put in place, so deleting a `service:`/`repo:`/`shim:`/`link:`/
    /// `schedule:` line left it in effect forever — `sync` could not even *detect* the
    /// removal. The applied-extras ledger (`locks/extras.toml`) closes that: this diffs the
    /// currently-declared extras against what the last sync recorded, undoes the difference,
    /// and records the new set. It is the extras' half of "removing a line removes the thing".
    ///
    /// Best-effort per item: a backend that cannot undo one extra must not block the rest, so
    /// each failure warns and the run continues. The ledger is still updated to the declared
    /// set — a drifted extra we could not undo is reported, not retried forever.
    /// The extras a sync would undo: applied last time, declared nowhere now. `status` and
    /// `reconcile_extras` ask the same question, so they ask it in the same place — a preview
    /// computed a second way is a preview free to disagree with the run.
    pub async fn extras_drift(
        &self,
        state: &crate::model::DesiredState,
    ) -> Result<Vec<String>> {
        use crate::core::extras_lock::ExtrasLedger;

        let path = ExtrasLedger::path_in(&self.config.config_root().join("locks"));
        let ledger = ExtrasLedger::load(&path)?;
        Ok(ledger.drift(&declared_extras(state)))
    }

    pub async fn reconcile_extras(&self, state: &crate::model::DesiredState) -> Result<()> {
        use crate::core::extras_lock::{split_key, ExtrasLedger};

        let declared = declared_extras(state);

        let path = ExtrasLedger::path_in(&self.config.config_root().join("locks"));
        let ledger = ExtrasLedger::load(&path)?;
        let drift = ledger.drift(&declared);

        // Nothing drifted and the record already matches — no work and, crucially, no write, so
        // an ordinary no-op sync does not churn `locks/extras.toml` on every run.
        if drift.is_empty() && ledger.applied() == &declared {
            return Ok(());
        }

        // An undo that failed leaves the extra in place, so its key stays in the ledger and
        // the next sync tries again. Dropping it would turn one warning into a service or
        // timer LiNix has permanently forgotten it owns.
        let mut still_applied = std::collections::BTreeSet::new();
        for key in &drift {
            let Some((kind, id)) = split_key(key) else {
                continue;
            };
            if self.config.dry_run {
                info!("[DRY-RUN] would undo removed extra `{}`", key);
                continue;
            }
            info!("`{}` is no longer declared — undoing it.", key);
            if let Err(e) = self.undo_extra(kind, id).await {
                warn!(
                    "could not undo `{}` ({}); it is still in place and the next sync will \
                     try again.",
                    key, e
                );
                still_applied.insert(key.clone());
            }
        }

        // Record what is declared now (even in dry-run? no — a dry run changes nothing, so
        // the ledger must not move, or the next real run would miss the drift).
        if !self.config.dry_run {
            let mut ledger = ledger;
            let mut recorded = declared;
            recorded.append(&mut still_applied);
            ledger.record(recorded);
            ledger.save(&path)?;
        }
        Ok(())
    }

    /// Execute the undo for one drifted extra, dispatched on its kind (S20). Each arm uses the
    /// same removal path the imperative command would.
    async fn undo_extra(&self, kind: &str, id: &str) -> Result<()> {
        match kind {
            "shim" => self.shim_manager().await?.remove_shim(id).await,
            "schedule" => self.scheduler.deprovision(&self.executor, id).await,
            "service" | "link" | "setting" => {
                let Some(b) = self.registry.get(kind) else {
                    return Err(Error::BackendNotFound(format!(
                        "the `{}` backend is not available to undo `{}:{}`",
                        kind, kind, id
                    )));
                };
                let Some(inst) = b.as_installable() else {
                    return Ok(());
                };
                inst.remove(std::slice::from_ref(&id.to_string()), b.sudo_for_write())
                    .await
                    .map(|_| ())
            }
            "repo" => {
                // A repo key is `repo:<backend>:<spec>`; `id` here is `<backend>:<spec>`.
                let Some((backend, spec)) = id.split_once(':') else {
                    return Err(Error::Config(format!("malformed repo key `repo:{}`", id)));
                };
                let Some(b) = self.registry.get(backend) else {
                    return Err(Error::BackendNotFound(format!(
                        "the `{}` backend is not available to undo `repo:{}:{}`",
                        backend, backend, spec
                    )));
                };
                let Some(mgr) = b.as_repo_manager() else {
                    return Err(Error::Unsupported(format!(
                        "`{}` does not manage repositories",
                        backend
                    )));
                };
                mgr.remove_repo(spec, b.sudo_for_write()).await.map(|_| ())
            }
            other => {
                warn!("no undo known for extra kind `{}`.", other);
                Ok(())
            }
        }
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
        let editor = crate::model::Editor::new(&layout, &vocab, facts);
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
        let edits = crate::model::Editor::new(&layout, &vocab, facts)
            .retarget_backend(&files, target_pkg, new_backend)
            .map_err(Error::from)?;
        for e in &edits {
            info!("{}", e.describe("Moved"));
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
        let edits = crate::model::Editor::new(&layout, &vocab, facts)
            .remove_from(&files, target_pkg)
            .map_err(Error::from)?;
        for e in &edits {
            info!("{}", e.describe("Removed"));
        }
        Ok(edits)
    }

    #[instrument(skip(self))]
    pub async fn resolve_spec(&self, spec_str: &str) -> Result<Vec<PackageSpec>> {
        StateResolver::new(&self.config, self.registry.clone(), false)
            .await
            .resolve_spec(spec_str)
            .await
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
        let _ = self.snapshot_manager.auto_snapshot(crate::core::snapshot::SnapshotLabel::PreUpgrade).await?;
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
        let backends = self.registry.available();
        let name = package_name.to_string();
        // A package lives in one backend, but which one is not known until asked, so all are
        // asked at once and the first answer wins. Serial, this waited on every backend that
        // did not have it before reaching the one that did.
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
        // The managed check touches the state lock once, after the process work is done,
        // rather than holding it across every backend's query.
        let state = self.state.lock().await;
        Ok(listed
            .into_iter()
            .flatten()
            .filter(|pkg| !state.is_managed(&pkg.backend, &pkg.name))
            .collect())
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

    /// Remove any managed packages whose `@expires` datetime has passed, across their
    /// backends, and persist the updated state. Runs as post-command maintenance so a dated
    /// line takes effect on time rather than waiting for the next explicit `sync`. No-op in
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
                "lease on {}:{} expired, but it is protected — leaving it installed. \
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
            crate::app::sync::guard::GuardScope::ExpirySweep,
        )
        .await
        {
            warn!(
                "expired-lease sweep refused, leaving them installed.\n{}",
                e
            );
            return Ok(());
        }

        info!(
            "{} package(s) have expired leases — reclaiming.",
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
                            "failed to remove expired {}:{}: {}",
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
            "{} temporary uninstall(s) are due for restoration.",
            due.len()
        );
        self.restore_suspensions(due, "temporarily-removed").await
    }

    /// Restore every package suspended under a given ephemeral shell session (called when
    /// that shell exits). Same warn-and-move-on contract as the timed sweep.
    pub async fn restore_session_suspensions(&self, session_id: &str) -> Result<()> {
        let owned = { self.state.lock().await.get_session_suspensions(session_id) };
        self.restore_suspensions(owned, "session-suspended on shell exit")
            .await
    }

    /// Reinstall a set of suspended packages and clear each suspension — whether the reinstall
    /// succeeds or fails (a suspension LiNix cannot honour is dropped, not retried forever).
    /// One implementation shared by the timed sweep and the shell-exit restore, which used to
    /// carry byte-identical copies of this loop (E11); `occasion` is the only thing that
    /// differed, and it only ever changed the log wording.
    async fn restore_suspensions(
        &self,
        items: Vec<crate::core::state::Suspension>,
        occasion: &str,
    ) -> Result<()> {
        for s in items {
            match self.restore_package(&s.backend, &s.name).await {
                Ok(()) => {
                    info!("Restored {} {}:{}", occasion, s.backend, s.name);
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
                        "could not restore {} {}:{} ({}); dropping the suspension.",
                        occasion, s.backend, s.name, e
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

/// Every declared extra key: the dependents (repo/shim/service/link/setting) and the schedules.
fn declared_extras(state: &crate::model::DesiredState) -> std::collections::BTreeSet<String> {
    state
        .extras
        .iter()
        .filter_map(|(s, _)| crate::core::extras_lock::extra_key(s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::grammar::{Options, Statement};


    fn link(name: &str, target: &str) -> (Statement, Origin) {
        let mut opts = Options::default();
        opts.insert("target", target);
        (
            Statement::Link(name.to_string(), opts),
            Origin::new("modules/files.txt", 1),
        )
    }

    fn state_with(extras: Vec<(Statement, Origin)>) -> crate::model::DesiredState {
        crate::model::DesiredState {
            extras,
            ..Default::default()
        }
    }

    #[test]
    fn only_a_new_link_outside_home_is_asked_about() {
        #[cfg(windows)]
        let system = r"C:\ProgramData\linix\hosts";
        #[cfg(not(windows))]
        let system = "/etc/cron.d/backup";

        let state = state_with(vec![
            link("dotfiles/gitconfig", "~/.gitconfig"),
            link("cron/backup", system),
        ]);

        let asked = App::outside_home_links(&state, &|_| false);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].0, "link:cron/backup");
        assert_eq!(asked[0].1, std::path::PathBuf::from(system));

        // The destination is already there: it was agreed to on the run that placed it, and a
        // re-converge that asks again is a prompt on every sync.
        assert!(App::outside_home_links(&state, &|_| true).is_empty());
    }
}
