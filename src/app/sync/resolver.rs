use crate::app::sync::pins::UnmeetablePin;
use crate::app::vocab::Vocab;
use crate::backends::BackendRegistry;
use crate::config::grammar::{statement, Candidates, Gates, GrammarError, Origin, Statement};
use crate::config::parser::HostFacts;
use crate::config::Config;
use crate::core::LockFile;
use crate::core::{Error, PackageSpec, Result, Validator};
use crate::model::resolve::{to_spec, Provenance, BARE};
use crate::model::{DesiredState, Layout, Priority};
use semver::{Version, VersionReq};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, instrument, warn};
use version_compare::{compare as loose_compare, Cmp};

/// This invocation's resolved variables, per repo and provider setting (IX.6).
///
/// See [`StateResolver::resolve_vars_against`] for why this exists at all.
#[allow(clippy::type_complexity)]
static VARS_MEMO: once_cell::sync::Lazy<
    dashmap::DashMap<
        (u64, std::path::PathBuf, String),
        Arc<tokio::sync::Mutex<Option<(crate::model::vars::Vars, crate::model::vars::VarOrigins)>>>,
    >,
> = once_cell::sync::Lazy::new(dashmap::DashMap::new);

/// Which resolution we are on. IX.6 says variables resolve once **per invocation**, and one
/// process is not always one invocation.
static RESOLUTION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Declare that a fresh resolution is starting, so the variables are resolved again.
///
/// `watch` is the case that makes this necessary: it runs reconcile passes in a loop inside one
/// process, and a provider that reads the clock or the network is *supposed* to answer
/// differently on the next tick. Memoising for the life of the process would freeze a `when
/// $hour` at whatever hour the daemon started, which is a worse bug than the one the memo fixes.
pub fn new_resolution() {
    RESOLUTION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    VARS_MEMO.clear();
}

/// Whether the statements handed to the prober are the whole model.
///
/// Only then does a name's absence mean it is no longer declared. A single `shall run jq` is
/// one line, and pruning the bare-name lock against it would forget every other name on the
/// machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Coverage {
    WholeModel,
    OneLine,
}

/// What one manager said when asked whether it has a name.
///
/// `Lacks` and `CouldNotTell` both send the name on to the next candidate; they differ in
/// what may be *written down* afterwards. An answer nobody could give is not a no, and
/// freezing a lower manager on the strength of it is how an unedited line comes to mean a
/// different package the day an index goes stale (V.7c).
enum Verdict {
    Has,
    Lacks,
    CouldNotTell(String),
}

/// Give an error the file and line the declaration came from, in the shape the grammar's own
/// refusals already use.
///
/// A refusal about an invisible character is the one a reader can least find by looking, so it
/// is the one that most needs a location — and it was the only class arriving without one. The
/// `__source` tag is how far the origin travels past the parser; a spec that somehow has none
/// keeps the undecorated message rather than gaining a made-up location.
fn located(e: Error, source: Option<&String>) -> Error {
    let Some(origin) = source else { return e };
    GrammarError::new(
        origin
            .parse::<Origin>()
            .unwrap_or_else(|_| Origin::argument()),
        e.to_string(),
    )
    .into()
}

/// A candidate list as the line wrote it, for an error to quote back.
fn describe_candidates(candidates: &Candidates) -> String {
    match candidates {
        Candidates::Priority => "every manager in `priority`".to_string(),
        Candidates::Named(names) => format!("`{}`", names.join(",")),
        Candidates::NamedThenPriority(names) => {
            format!("`{},list`", names.join(","))
        }
    }
}

pub struct StateResolver<'a> {
    config: &'a Config,
    registry: Arc<BackendRegistry>,
    layout: Layout,
    /// Strict mode (`sync --locked`): a package with no entry in locks/versions.json is an
    /// error rather than a free resolve. For reproducing a machine exactly, where a package
    /// nobody locked is a gap in the reproduction rather than a detail.
    locked: bool,
    /// Whether a recorded version wins over whatever the manager offers today. **On by
    /// default** (owner ruling, 2026-07-24): a sync converges to what was decided, not to what
    /// was published since. `--upgrade` turns it off for the run that means to move forward.
    /// Unlike `locked`, a package with no recorded version is not an error here — it simply
    /// resolves, which is what a machine that has never run `shall lock` does for everything.
    prefer_locks: bool,
    /// "backend:package" -> version.
    locks: HashMap<String, String>,
    /// Pre-resolved variables to use instead of running the provider (Part IX, IX.6). Set when
    /// applying a saved plan: re-running a clock/shell/network provider at apply time could
    /// disagree with what the plan froze, so `apply` resolves the model against the plan's own
    /// variables. `None` resolves them fresh, which is what every non-plan path does.
    vars_override: Option<crate::model::vars::Vars>,
    /// Resolve as if `active` held this body. Set by `profile show`, which asks "what would
    /// this profile give me" and must not answer by editing the machine's `active` file.
    active_override: Option<String>,
    /// Whether this resolution may freeze an unpinned name's backend into this host's
    /// `locks/bare.HOST.toml`.
    ///
    /// Recording is a decision, so only a run that goes on to change the machine makes it.
    /// Off unless a caller says otherwise: forgetting to ask means a command reads without
    /// leaving a mark, which is the harmless direction to be wrong in.
    may_record_locks: bool,
    /// The one cap on remote lookups this resolver has in flight.
    ///
    /// There are two nested fan-outs — every bare name at once, and within each name every
    /// candidate manager at once — and bounding them separately multiplies. One gate held by
    /// the leaf that actually talks to a registry is the number a user set.
    remote_gate: Arc<tokio::sync::Semaphore>,
}

impl<'a> StateResolver<'a> {
    pub async fn new(config: &'a Config, registry: Arc<BackendRegistry>, locked: bool) -> Self {
        // Read unconditionally: recorded versions are preferred on every ordinary run now, so
        // this file is no longer only a strict-mode input. A missing file is the ordinary state
        // of a machine that has not run `shall lock`, never an error.
        let mut locks = HashMap::new();
        let lock_path = config.config_root().join("locks").join("versions.json");
        if tokio::fs::try_exists(&lock_path).await.unwrap_or(false) {
            if let Ok(data) = fs::read_to_string(&lock_path).await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                    if let Some(obj) = json.get("locks").and_then(|l| l.as_object()) {
                        for (key, val) in obj {
                            if let Some(v_str) = val.as_str() {
                                locks.insert(key.clone(), v_str.to_string());
                            }
                        }
                    }
                }
            }
        } else if locked {
            warn!("Locked mode requested but locks/versions.json is missing.");
        }

        Self {
            config,
            registry,
            layout: config.layout(),
            locked,
            prefer_locks: true,
            locks,
            vars_override: None,
            active_override: None,
            may_record_locks: false,
            remote_gate: Arc::new(tokio::sync::Semaphore::new(config.network_parallel.max(1))),
        }
    }

    /// `sync --upgrade`: ignore what was recorded and take what the managers offer now. Moving
    /// a version forward is a decision, so it is asked for (owner ruling, 2026-07-24).
    pub fn upgrading(mut self) -> Self {
        self.prefer_locks = false;
        self
    }

    /// Resolve the model against these already-resolved variables instead of running the
    /// provider (used by `apply` to reuse a saved plan's frozen variables).
    pub fn with_vars(mut self, vars: crate::model::vars::Vars) -> Self {
        self.vars_override = Some(vars);
        self
    }

    /// Resolve as if `active` held `body`, leaving the file on disk untouched.
    ///
    /// `profile show` answers "what would this profile give me", and it used to answer by
    /// **writing the profile's name into the real `active` file**, resolving, and writing the old
    /// contents back. A read-only command changed what the machine was set to for the length of a
    /// resolve — and a `^C`, a panic, or a second write that failed left it changed, so the next
    /// `sync` converged to a profile the user had asked only to look at.
    pub fn as_if_active(mut self, body: String) -> Self {
        self.active_override = Some(body);
        self
    }

    /// Say that this resolution belongs to a run that will act on it, so a bare name it
    /// settles may be recorded. `reconcile` is the only caller: everything else is looking.
    pub fn recording_locks(mut self) -> Self {
        self.may_record_locks = true;
        self
    }

    /// The `priority` file: which package managers this setup uses, and in what order.
    ///
    /// A missing file is an error and not a detected default. Shall cannot pick your
    /// package managers for you — inheriting them from whatever happens to be installed is
    /// the thing `priority` exists to stop (V.15), and a default nobody chose is a default
    /// nobody can safely change (P5).
    pub async fn priority_for_host(&self) -> Result<Priority> {
        let facts = self.facts_for_host().await?;
        self.priority(&facts).await
    }

    /// The `priority` file's text, or the error that teaches what the file is for.
    async fn priority_body(&self) -> Result<(std::path::PathBuf, String)> {
        let file = self.layout.priority_file();
        let body = match fs::read_to_string(&file).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // The command that writes this file goes first. This is the first thing a new
                // user sees, and it used to explain the format by hand and never mention
                // `shall init` — which exists to do exactly this, detects the managers on the
                // machine, and is one word long. Explaining how to hand-write a file the
                // program will write for you is a dead end however well the format is
                // described.
                return Err(Error::Config(format!(
                    "no `priority` file at {}.\n  \
                     Run `shall init` — it writes this file with the package managers it \
                     finds on this machine, along with the rest of the repo.\n\n  \
                     To write it by hand instead: `priority` lists the managers Shall may \
                     use, one per line, best first — for example:\n\n    apt\n    cargo\n\n  \
                     Listed means Shall uses it. Not listed means Shall does not touch it at \
                     all.",
                    file.display()
                )));
            }
            Err(e) => return Err(Error::from(e)),
        };
        // **A file that names no backend is the same state as no file, and got the opposite
        // treatment.** A *missing* `priority` produced the message above — the file, the fix,
        // and an example. An *empty* one was accepted without a word, and empty does not mean
        // "no backends": `UniversalSearch` reads an empty enabled set as *every available
        // backend*, on the stated premise that only a missing file can produce one. That
        // premise was false, so emptying the file quietly inverted the sentence the file's own
        // header prints — "Not listed = Shall does not use it at all" (B8).
        //
        // Asked of the parser rather than of the text, so a file of nothing but comments, and
        // one holding only an empty `when` block, are the same answer as a file of nothing. A
        // body that will not parse falls through untouched: the real parser below reports that
        // far better than a guess here could.
        if Priority::every_backend(&file, &body).is_ok_and(|p| p.is_empty()) {
            return Err(Error::Config(format!(
                "the `priority` file at {} names no package manager.\n  \
                 Shall does not fall back to whatever is installed — that is the point of the \
                 file — so there is nothing it may use and every declaration would be \
                 refused.\n\n  \
                 Run `shall init` to have it written from the managers on this machine, or \
                 list them one per line, best first:\n\n    apt\n    cargo\n",
                file.display()
            )));
        }
        Ok((file, body))
    }

    async fn priority(&self, facts: &HostFacts) -> Result<Priority> {
        let (file, body) = self.priority_body().await?;
        Priority::parse(&file, &body, facts).map_err(Error::from)
    }

    /// The backend vocabulary the `vars` file is parsed with, before any variable exists.
    ///
    /// Never an order and never a filter: `priority`'s `when` blocks are evaluated against the
    /// resolved facts by [`StateResolver::priority`], which is the answer everything else uses.
    async fn vars_vocabulary(&self) -> Result<Priority> {
        let (file, body) = self.priority_body().await?;
        Priority::every_backend(&file, &body).map_err(Error::from)
    }

    /// Resolve the variables against the given facts — the one implementation, so `shall vars`
    /// prints what a `when` will see rather than a second opinion about it.
    ///
    /// **Resolved once per invocation (IX.6), and now actually once.** Every resolver entry
    /// point comes through here, and `StateResolver` is constructed at 39 sites — so a single
    /// `shall check` ran the user's `vars.sh` three times, measured, and any `http()` variable
    /// was fetched three times over three fresh connections. That is not only slow: a vars
    /// provider is a program the user wrote, and running it three times runs its side effects
    /// three times. `HostFacts::with_vars` has claimed "resolved once per invocation and
    /// carried, never recomputed" since it was written; this is what makes the sentence true.
    ///
    /// Keyed by the repo and the provider setting rather than global, because one process can
    /// legitimately hold more than one config — that is exactly what the test suite is.
    async fn resolve_vars_against(
        &self,
        facts: &HostFacts,
    ) -> Result<(crate::model::vars::Vars, crate::model::vars::VarOrigins)> {
        let key = (
            RESOLUTION.load(std::sync::atomic::Ordering::SeqCst),
            self.layout.config_root().to_path_buf(),
            format!("{:?}", self.config.vars.source),
        );
        let slot = VARS_MEMO
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(None)))
            .clone();
        // Held across the resolution, so two concurrent askers do not both run the provider.
        let mut slot = slot.lock().await;
        if let Some(resolved) = slot.as_ref() {
            return Ok(resolved.clone());
        }
        let resolved = self.resolve_vars_uncached(facts).await?;
        *slot = Some(resolved.clone());
        Ok(resolved)
    }

    async fn resolve_vars_uncached(
        &self,
        facts: &HostFacts,
    ) -> Result<(crate::model::vars::Vars, crate::model::vars::VarOrigins)> {
        let priority = self.vars_vocabulary().await?;
        let known = self.vocab(&priority);
        let layout = self.layout.clone();
        let facts = facts.clone();
        let source = self.config.vars.source.clone();
        // Synchronous, and not cheaply so: this runs the user's external vars provider as a
        // subprocess, every embedded `sh()` as another, and every `http()` as a network round
        // trip. On a runtime worker thread that is a worker parked for the length of somebody
        // else's script, which is why it goes to the blocking pool instead.
        tokio::task::spawn_blocking(move || {
            crate::model::Resolver::new(&layout, &known, &priority)
                .with_facts(facts)
                .with_vars_source(source)
                .load_vars_with_origins()
        })
        .await
        .map_err(|e| Error::Other(format!("resolving variables: {}", e)))?
        .map_err(Error::from)
    }

    /// Resolve just the variables (Part IX), without planning the whole model — for `shall vars`.
    /// The same resolution `resolve_model` performs, so what this prints is what a `when` sees.
    pub async fn resolve_vars(&self) -> Result<crate::model::vars::Vars> {
        self.resolve_vars_with_origins().await.map(|(v, _)| v)
    }

    /// [`resolve_vars`], plus where each variable was set — for `shall vars` and `why`, which
    /// have to say not just a variable's value but the line or provider that produced it (W11/W12).
    pub async fn resolve_vars_with_origins(
        &self,
    ) -> Result<(crate::model::vars::Vars, crate::model::vars::VarOrigins)> {
        self.resolve_vars_against(&HostFacts::current()).await
    }

    /// The variables as of the last successful sync (HEAD), for W13's change note. Line-file
    /// provider only: a script or program has no committed values to diff, and a clock/network
    /// var would read as "changed" every run, which is noise, not a cause. `None` when there is
    /// no baseline — no git repo, no commit yet, or a non-line-file provider.
    pub async fn vars_at_last_sync(
        &self,
        git: &crate::core::GitManager,
    ) -> Result<Option<crate::model::vars::Vars>> {
        use crate::model::vars_provider::Kind;
        let Some(selected) = self.vars_provider()? else {
            return Ok(None);
        };
        if selected.kind != Kind::LineFile {
            return Ok(None);
        }
        let name = selected
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("vars");
        let Some(body) = git.show_at_head(name)? else {
            return Ok(None);
        };
        let facts = HostFacts::current();
        let priority = self.vars_vocabulary().await?;
        let known = self.vocab(&priority);
        let (vars, _) = crate::model::Resolver::new(&self.layout, &known, &priority)
            .with_facts(facts)
            .with_vars_source(self.config.vars.source.clone())
            .resolve_linefile_body(&selected.path, &body)
            .map_err(Error::from)?;
        Ok(Some(vars))
    }

    /// The active provider file and kind, or `None` when the repo has no `vars` provider.
    pub fn vars_provider(&self) -> Result<Option<crate::model::vars_provider::Selected>> {
        crate::model::vars_provider::select(self.layout.config_root(), &self.config.vars.source)
            .map_err(Error::from)
    }

    /// The facts every `when` in your files is evaluated against: what this machine is,
    /// plus this run's variables.
    ///
    /// IX.6: variables are resolved exactly once per invocation. Anything that reads a `when`
    /// without them sees `$role` as an unknown key and refuses a file that is correct — which
    /// is what `activate`, `deactivate` and `uninstall` all did before W8.
    /// The parser's backend vocabulary, carrying the `groups` file (U18) so `tools:rg` expands
    /// to its chain. One place, so every resolution path — the model, the vars pass, the
    /// whole-repo parse — sees the same groups; a group that worked in `sync` and not in `check`
    /// would be the "two of everything" this repo removes.
    ///
    /// A malformed or cyclic `groups` file is a warning and no groups, not a stopped
    /// resolution: a broken groups file should not take down a config whose lines mostly do not
    /// use one, and the lines that do will fail as "not a backend" — pointing at the real fix.
    fn vocab(&self, priority: &Priority) -> Vocab {
        let groups = match crate::model::groups::Groups::load(&self.layout.groups_file()) {
            Ok(g) => g,
            Err(e) => {
                warn!("ignoring the `groups` file: {}", e);
                crate::model::groups::Groups::default()
            }
        };
        Vocab::new(&self.registry, self.config, priority).with_groups(groups)
    }

    /// This host's backend vocabulary, for anything that reads or writes a line.
    ///
    /// **The only public way to get one.** `App` built its own — `Vocab::new` with no
    /// `.with_groups()` — and handed it to `declare`, `undeclare`, `retarget` and `declares`.
    /// So `tools:rg` expanded to its chain when `sync` parsed the file and was "not a backend"
    /// when `install` wrote the same line: one program, two vocabularies, disagreeing about
    /// what a name means.
    pub async fn vocabulary(&self) -> Result<Vocab> {
        let facts = self.facts_for_host().await?;
        let priority = self.priority(&facts).await?;
        Ok(self.vocab(&priority))
    }

    pub async fn facts_for_host(&self) -> Result<HostFacts> {
        let facts = HostFacts::current();
        let vars = match &self.vars_override {
            Some(frozen) => frozen.clone(),
            None => self.resolve_vars_against(&facts).await?.0,
        };
        if !vars.is_empty() {
            debug!("{} variable(s) resolved", vars.len());
        }
        Ok(facts.with_vars(vars))
    }

    /// Every parse error in `modules/` and `profiles/`, reached by an active profile or not
    /// (II.3, for `check`).
    pub async fn parse_everything(&self) -> Result<Vec<GrammarError>> {
        let facts = self.facts_for_host().await?;
        let priority = self.priority(&facts).await?;
        let known = self.vocab(&priority);
        Ok(crate::model::Resolver::new(&self.layout, &known, &priority)
            .with_facts(facts)
            .parse_everything())
    }

    #[instrument(skip(self))]
    pub async fn resolve_desired_state(&self) -> Result<HashMap<String, Vec<PackageSpec>>> {
        Ok(self.resolve_model().await?.packages)
    }

    /// The backends this host manages, for a plan that may reap (II.6).
    ///
    /// **The only place a [`HostBackends`] is made.** It lives here and not on `App` because
    /// `App` is not reachable from `app/profile.rs` or `app/shell/`, and both of those planned
    /// removals without the list precisely because asking for it meant reaching for something
    /// they did not have.
    ///
    /// An unreadable `priority` yields the empty list, which allows every backend — the same
    /// answer as before, and the honest one: a host that could not say which managers are its
    /// own has not excluded any. Every caller that reaps resolves the config first, so a
    /// `priority` this cannot read has already failed the run.
    pub async fn host_backends(&self) -> crate::app::sync::planner::HostBackends {
        crate::app::sync::planner::HostBackends::from_priority(
            self.priority_for_host()
                .await
                .map(|p| p.order().to_vec())
                .unwrap_or_default(),
        )
    }

    /// II.7, end to end: `active` -> profiles -> the modules they reach -> the desired state.
    ///
    /// The map the seam carries holds `absent:` lines too, marked `present: false`; the
    /// planner splits them out. Everything below the seam — `src/backends/`, `src/core/`,
    /// `src/parsers/` — is untouched by any of this.
    pub async fn resolve_model(&self) -> Result<DesiredState> {
        let facts = self.facts_for_host().await?;
        let priority = self.priority(&facts).await?;
        let known = self.vocab(&priority);

        debug!("resolving desired state for host '{}'", facts.host);

        // Steps 1-3 read the files. Probing needs the network, so it happens out here,
        // between reading and merging: a bare `ripgrep` and an explicit `cargo:ripgrep` are
        // one package, and they only meet if the answer is known before the merge (V.16).
        // Reading `active`, every profile and every module is dozens of synchronous file reads;
        // collecting is the same again. Both go to the blocking pool so the runtime keeps its
        // workers.
        let mut reached = {
            let (layout, known, facts) = (self.layout.clone(), known.clone(), facts.clone());
            let active = self.active_override.clone();
            let priority = priority.clone();
            tokio::task::spawn_blocking(move || {
                let mut r =
                    crate::model::Resolver::new(&layout, &known, &priority).with_facts(facts);
                if let Some(body) = active {
                    r = r.as_if_active(body);
                }
                r.statements()
            })
            .await
            .map_err(|e| Error::Other(format!("reading the model: {}", e)))??
        };
        // U33: run any `generate:` command and splice its declarations into the stream BEFORE
        // aliases, regexes, bare-name probing and collect — so generated lines get exactly the
        // same treatment (and the same guard and removal preview) as typed ones. Off by default,
        // and a failed generator is a failed resolution.
        self.expand_generators(&mut reached.statements, &known, &facts)
            .await?;
        self.resolve_aliases(&mut reached.statements);
        self.expand_regexes(&mut reached.statements, &priority)
            .await?;
        let answers = self
            .probe_bare_names(&reached.statements, &priority, Coverage::WholeModel)
            .await?;

        let mut state = {
            let (layout, known, priority) = (self.layout.clone(), known.clone(), priority.clone());
            tokio::task::spawn_blocking(move || {
                crate::model::Resolver::new(&layout, &known, &priority)
                    .with_facts(facts)
                    .with_bare(answers)
                    .collect(reached)
            })
            .await
            .map_err(|e| Error::Other(format!("collecting the model: {}", e)))??
        };

        for specs in state.packages.values_mut() {
            for spec in specs.iter_mut() {
                // Located, like every refusal the grammar makes about the same file. The
                // character validator's refusals are the ones a user can least find by
                // looking — the character at fault is a bidi override, a NUL or an escape —
                // and they were the only ones arriving without a file and a line.
                Validator::validate_package_name_for(&spec.name, &spec.backend).map_err(|e| {
                    located(e, spec.options.one("__source").map(String::from).as_ref())
                })?;
            }
        }

        self.apply_locks(&mut state)?;

        // II.16: an expired line lingers, because Shall must not rewrite your files. It
        // gets mentioned by the exact file and line, never vaguely.
        for (key, origin) in &state.lapsed {
            warn!("`{}` at {} has expired and no longer counts.", key, origin);
        }

        debug!(
            "{} declared present, {} declared absent.",
            state.present().count(),
            state.absent().count()
        );
        Ok(state)
    }

    /// Run every `generate:` command and splice its output into the statement stream (U33).
    ///
    /// The dangerous half of Lisp, kept on the safe side of XIII.32's line by four rules, none
    /// waived:
    /// - **Off by default.** With `allow_generators` unset, a `generate:` line is a refusal,
    ///   naming the config key and `shall lock`. The computing-config surface is dormant unless
    ///   turned on deliberately.
    /// - **The ledger gates it.** A generator runs code the repo carries; it is approved by
    ///   `shall lock` (content-addressed, like `exec:`), and an unapproved or changed command
    ///   stops resolution. `-y` cannot approve it.
    /// - **A failure is a failed resolution.** A non-zero exit is an error, never an empty set —
    ///   an empty declaration set is a mass-removal input (VI.0), and "the generator broke" must
    ///   never be read as "nothing is declared".
    /// - **The output is shown, not trusted.** Spliced in before probing and collection, so the
    ///   generated lines pass the same conflict check, guard and removal preview as typed ones.
    async fn expand_generators(
        &self,
        statements: &mut Vec<(Statement, Origin, Gates)>,
        known: &dyn crate::config::grammar::BackendNames,
        facts: &HostFacts,
    ) -> Result<()> {
        use crate::core::hook_lock::{generate_id, hash_script, refusal, HookLedger};

        if !statements
            .iter()
            .any(|(s, _, _)| matches!(s, Statement::Generate(..)))
        {
            return Ok(());
        }

        // Split the stream: keep everything that is not a generator, process the generators.
        let mut kept: Vec<(Statement, Origin, Gates)> = Vec::new();
        let mut gens: Vec<(String, Origin, Gates)> = Vec::new();
        for (stmt, origin, gates) in std::mem::take(statements) {
            match stmt {
                Statement::Generate(cmd, _) => gens.push((cmd, origin, gates)),
                other => kept.push((other, origin, gates)),
            }
        }

        let ledger = HookLedger::load(&HookLedger::path_in(&self.layout.locks_dir()))?;

        // Two passes on purpose. Every refusal — generators off, an unreadable script, an
        // unapproved hash — is checked first and in declaration order, so which one a user is
        // told about does not depend on which script happened to finish first. Only then do the
        // approved scripts run, and they run at once: each is a subprocess, they are
        // independent of one another, and the merge happens afterwards anyway.
        let mut approved: Vec<(String, Origin, Gates, std::path::PathBuf)> = Vec::new();
        for (cmd, origin, gates) in gens {
            if !self.config.allow_generators {
                return Err(Error::Refused(format!(
                    "{}: `generate:{}` is off by default.\n  \
                     A generator runs a command and treats its stdout as declarations — the one \
                     place the config computes its state instead of stating it.\n  \
                     Set `allow_generators = true` to enable it, then `shall lock` to approve the \
                     command.",
                    origin, cmd
                )));
            }

            // Content-addressed approval, like `exec:`: hash the script's bytes so an edit
            // re-requires approval. A command that is not a readable file is refused rather than
            // run as a bare shell word.
            let declared = std::path::Path::new(&cmd);
            let path = if declared.is_absolute() {
                declared.to_path_buf()
            } else {
                self.config.config_root().join(declared)
            };
            let body = std::fs::read_to_string(&path).map_err(|e| {
                Error::Validation(format!(
                    "{}: `generate:{}` — cannot read the command at {} ({}). A generator names a \
                     script the config carries; its contents are hashed and run.",
                    origin,
                    cmd,
                    path.display(),
                    e
                ))
            })?;
            let verdict = ledger.verdict(&generate_id(&cmd), &hash_script(&body));
            if !verdict.is_approved() {
                return Err(Error::Refused(format!(
                    "{}: {}",
                    origin,
                    refusal(&generate_id(&cmd), "generate command", &verdict)
                )));
            }

            approved.push((cmd, origin, gates, path));
        }

        let outputs: Vec<std::io::Result<std::process::Output>> = {
            use futures::stream::StreamExt;
            futures::stream::iter(approved.iter().map(|(cmd, _, _, path)| {
                // Supervised: a `generate:` command is somebody's script, it runs on every
                // sync, and before this it had no bound and no owner — one that blocked on a
                // prompt blocked every sync on the machine with nothing said, and one abandoned
                // by a failed sync kept running.
                let mut command = tokio::process::Command::new(path);
                command.current_dir(self.config.config_root());
                let label = format!("generate:{cmd}");
                async move {
                    crate::core::supervise::supervised_output(command, &label, false)
                        .await
                        .map_err(|e| std::io::Error::other(e.to_string()))
                }
            }))
            .buffered(self.config.max_parallel.max(1))
            .collect()
            .await
        };

        for ((cmd, origin, gates, _path), output) in approved.into_iter().zip(outputs) {
            let output = output.map_err(|e| {
                Error::Other(format!(
                    "{}: could not run `generate:{}` ({})",
                    origin, cmd, e
                ))
            })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(Error::Other(format!(
                    "{}: `generate:{}` failed (exit {}). A generator that fails is a failed \
                     resolution — its output is not trusted as an empty declaration set.\n{}",
                    origin,
                    cmd,
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                )));
            }

            // Parse the output through the ONE grammar parser, so a generated line is a line —
            // same statements, same errors, same downstream treatment. Origins name the
            // generator, so `eval`/`why` can say a declaration came from one.
            let out = crate::utils::text::sanitize(&String::from_utf8_lossy(&output.stdout));
            let synthetic = std::path::PathBuf::from(format!("generate:{}", cmd));
            let doc = crate::config::grammar::parse_document(&synthetic, &out, known)?;
            for (s, o, own) in doc.statements_with_gating(facts)? {
                // The generated line inherits the `generate:` line's own gates, then its own.
                let mut g = gates.clone();
                g.extend(own);
                let _ = &origin; // origin is preserved via the synthetic path in `o`
                kept.push((s, o, g));
            }
        }

        *statements = kept;
        Ok(())
    }

    /// Rewrite an aliased backend to its real name before anything reads it.
    ///
    /// Here rather than in the model: an alias is a nickname this config gives a backend,
    /// and the model should only ever see the real name — otherwise `priority` would have
    /// to know every nickname too.
    fn resolve_aliases(&self, statements: &mut [(Statement, Origin, Gates)]) {
        if self.config.aliases.is_empty() {
            return;
        }
        for (stmt, ..) in statements.iter_mut() {
            let decl = match stmt {
                Statement::Package(d) | Statement::Absent(d) => d,
                _ => continue,
            };
            if let Some(b) = &decl.backend {
                if let Some(real) = self.config.aliases.get(b) {
                    decl.backend = Some(real.clone());
                }
            }
        }
    }

    /// Replace every `re:` line with the packages it matches (II.15).
    ///
    /// Here, beside the bare-name probe: both turn one written line into what it actually
    /// names, both need the backends, and both must happen before the merge or two lines that
    /// resolve to the same package never meet.
    ///
    /// **A frozen pattern is not re-expanded** — `locks/regex.toml` is the switch, and a
    /// pattern re-matched every run grows the machine a package the day somebody else uploads
    /// one that fits, with nothing in your files changed and nothing to review.
    async fn expand_regexes(
        &self,
        statements: &mut Vec<(Statement, Origin, Gates)>,
        priority: &Priority,
    ) -> Result<()> {
        if !statements.iter().any(|(s, ..)| {
            matches!(s, Statement::Package(d) | Statement::Absent(d)
                if matches!(d.selector, crate::config::grammar::Selector::Regex(_)))
        }) {
            return Ok(());
        }

        let lock_path = crate::core::RegexLock::path_in(&self.layout.locks_dir());
        let mut lock = crate::core::RegexLock::load(&lock_path)?;
        let mut declared: Vec<String> = Vec::new();
        let mut lock_changed = false;
        let mut expanded: Vec<(Statement, Origin, Gates)> = Vec::new();

        for (stmt, origin, gates) in statements.drain(..) {
            let (decl, present) = match &stmt {
                Statement::Package(d) => (d, true),
                Statement::Absent(d) => (d, false),
                _ => {
                    expanded.push((stmt, origin, gates));
                    continue;
                }
            };
            let crate::config::grammar::Selector::Regex(pattern) = &decl.selector else {
                expanded.push((stmt, origin, gates));
                continue;
            };

            // The grammar refuses a prefixless `re:` at parse time, so this cannot be reached
            // through a file. Skipping rather than re-erroring keeps one rule in one place;
            // the pattern falls through as a package name and the validator says so.
            let Some(backend) = decl.backend.clone() else {
                expanded.push((stmt, origin, gates));
                continue;
            };

            let names = match lock.get(&backend, pattern) {
                Some(frozen) => {
                    debug!(
                        "`{}:re:{}` is frozen to {} name(s).",
                        backend,
                        pattern,
                        frozen.len()
                    );
                    frozen.to_vec()
                }
                None => {
                    let found = self
                        .match_catalogue(&backend, pattern, &origin, priority)
                        .await?;
                    lock_changed |= lock.record(&backend, pattern, found.clone());
                    found
                }
            };
            declared.push(crate::core::regex_lock::key(&backend, pattern));

            // Zero matches is an error, not an empty expansion: a pattern that matches nothing
            // is a typo every time, and silently declaring nothing is the failure this whole
            // design exists to remove (P3).
            if names.is_empty() {
                return Err(Error::from(
                    GrammarError::new(
                        origin,
                        format!("`{}:re:{}` matches no package.", backend, pattern),
                    )
                    .with_hint("check the pattern, or the manager's package index is empty."),
                ));
            }

            for name in names {
                let mut one = decl.clone();
                one.selector = crate::config::grammar::Selector::Name(name);
                // The line that produced it, so `why` can say a pattern put this here rather
                // than leaving a package nobody can find in any file.
                one.options.set("__from_regex".to_string(), pattern.clone());
                let stmt = if present {
                    Statement::Package(one)
                } else {
                    Statement::Absent(one)
                };
                expanded.push((stmt, origin.clone(), gates.clone()));
            }
        }

        lock_changed |= lock.retain_declared(&declared);
        if lock_changed {
            lock.save(&lock_path)?;
        }
        *statements = expanded;
        Ok(())
    }

    /// Every name in a manager's catalogue that the pattern matches.
    async fn match_catalogue(
        &self,
        backend: &str,
        pattern: &str,
        origin: &Origin,
        priority: &Priority,
    ) -> Result<Vec<String>> {
        if !priority.allows(backend) {
            return Err(Error::from(priority.reject(backend, origin)));
        }
        let listing = self
            .registry
            .get(backend)
            .and_then(|b| b.as_enumerable().cloned())
            .ok_or_else(|| {
                Error::from(
                    GrammarError::new(
                        origin.clone(),
                        format!("`{}` cannot list every package it could install.", backend),
                    )
                    .with_hint(
                        "`re:` needs a manager that can produce its whole catalogue — the \
                         system managers can, the language registries cannot. Name the \
                         packages instead.",
                    ),
                )
            })?;

        let re = crate::utils::regex_cache::compiled(pattern).map_err(|e| {
            Error::from(GrammarError::new(
                origin.clone(),
                format!("`re:{}` is not a valid regular expression: {}", pattern, e),
            ))
        })?;
        let names: Vec<String> = listing
            .available_names()
            .await?
            .into_iter()
            .filter(|n| re.is_match(n))
            .collect();
        debug!(
            "`{}:re:{}` matched {} name(s).",
            backend,
            pattern,
            names.len()
        );
        Ok(names)
    }

    /// Ask each of a name's candidate managers, in order, whether it has that name
    /// (II.7 step 4).
    ///
    /// Each distinct name is asked once however many lines mention it: the answer is about
    /// the name and the machine, not about the line.
    async fn probe_bare_names(
        &self,
        statements: &[(Statement, Origin, Gates)],
        priority: &Priority,
        coverage: Coverage,
    ) -> Result<HashMap<String, String>> {
        struct Question {
            name: String,
            candidates: Candidates,
            constraint: Option<String>,
            origin: Origin,
        }

        let mut questions: Vec<Question> = Vec::new();
        // Name → its position, so "have I already asked this?" is a lookup rather than a scan
        // over every declaration seen so far. The manifest is the thing users grow.
        let mut asked: HashMap<String, usize> = HashMap::new();
        for (stmt, origin, _) in statements {
            let Statement::Package(decl) = stmt else {
                continue;
            };
            if decl.backend.is_some() {
                continue;
            }
            let name = decl.selector.as_str().to_string();
            if let Some(seen) = asked.get(&name).map(|i| &questions[*i]) {
                // Two lines asking the same name to come from different places have no one
                // answer, and picking either silently would make the other line a lie.
                if seen.candidates != decl.candidates {
                    return Err(Error::from(
                        GrammarError::new(
                            origin.clone(),
                            format!(
                                "`{}` is declared with two different backend lists — {} here, \
                                 {} in {}.",
                                name,
                                describe_candidates(&decl.candidates),
                                describe_candidates(&seen.candidates),
                                seen.origin,
                            ),
                        )
                        .with_hint(
                            "a name resolves to one manager on one machine, so both lines \
                             have to agree on where it may come from.",
                        ),
                    ));
                }
                continue;
            }
            // The character check the model runs after `collect` cannot help here: this probe
            // is what hands the name to each candidate manager, so it happens *before* that
            // check has anything to look at. On Windows a manager that ships as a `.cmd` shim
            // reaches `cmd`, and one crafted bare name in a shared module wrote files on every
            // machine that so much as evaluated it (B-1). Located, because a name that came
            // from a file is answerable only by its line.
            Validator::refuse_command_metacharacters(&name, "a package name").map_err(|e| {
                Error::from(GrammarError::new(origin.clone(), e.to_string()).with_hint(
                    "a bare name is asked of every manager that could own it, so it becomes \
                     a command line before Shall knows whose it is.",
                ))
            })?;
            let constraint = decl.options.one("version").map(str::to_string);
            asked.insert(name.clone(), questions.len());
            questions.push(Question {
                name,
                candidates: decl.candidates.clone(),
                constraint,
                origin: origin.clone(),
            });
        }

        // II.6/II.15: the lock is the switch. A recorded name keeps its backend without asking
        // anyone — which is the point, since re-deriving the answer against whatever is
        // installed today is how an unedited line comes to mean a different package. Deleting
        // the entry is how you ask again.
        let lock_path = crate::core::BareLock::path_in(&self.layout.locks_dir());
        let mut lock = crate::core::BareLock::load(&lock_path)?;
        let mut lock_changed = match coverage {
            Coverage::WholeModel => {
                let declared: Vec<String> = questions.iter().map(|q| q.name.clone()).collect();
                lock.retain_declared(&declared)
            }
            Coverage::OneLine => false,
        };

        let listed: Vec<String> = priority.order().to_vec();

        let mut answers = HashMap::new();
        let mut to_ask: Vec<(Question, Vec<String>)> = Vec::new();

        // The lock answers what it can without troubling anyone.
        for question in questions {
            // A candidate `priority` does not list is not a candidate at all: `priority` says
            // which managers Shall may use on this host, whatever a line asks for (V.15).
            let chain: Vec<String> = question
                .candidates
                .order(&listed)
                .into_iter()
                .filter(|b| priority.allows(b))
                .collect();

            if let Some(backend) = lock.get(&question.name).map(str::to_string) {
                // Honoured only when the line still accepts it and this machine still has
                // it. The lock exists to stop an unedited line quietly changing meaning — it
                // was never a licence to demand a manager that is not here.
                let usable = self
                    .registry
                    .get(&backend)
                    .is_some_and(|b| b.is_available());
                if chain.contains(&backend) && usable {
                    debug!("`{}` is locked to `{}`.", question.name, backend);
                    answers.insert(question.name, backend);
                    continue;
                }
                warn!(
                    "`{}` was locked to `{}`, which {}. Asking again.",
                    question.name,
                    backend,
                    if usable {
                        "this line no longer accepts"
                    } else {
                        "this machine does not have"
                    }
                );
            }
            to_ask.push((question, chain));
        }

        // Every remaining name's chain, at once.
        //
        // `ask_the_chain` was already careful to ask one name's candidates concurrently; the
        // loop around it was not, so every bare name waited on the previous one's registry
        // round trips. A manifest with sixty bare names serialised sixty chains of network
        // lookups, one after another, for answers that have nothing to do with each other.
        //
        // Determinism is unaffected: the verdicts come back in declaration order, and the lock
        // records below are applied in that order, so the file this writes is byte-identical
        // to what the serial version wrote.
        let all_verdicts: Vec<Vec<Verdict>> = {
            use futures::stream::StreamExt;
            futures::stream::iter(
                to_ask
                    .iter()
                    .map(|(q, chain)| self.ask_the_chain(chain, &q.name, q.constraint.as_deref())),
            )
            .buffered(self.config.network_parallel.max(1))
            .collect()
            .await
        };

        for ((question, chain), verdicts) in to_ask.into_iter().zip(all_verdicts) {
            let Question {
                name,
                candidates,
                origin,
                ..
            } = question;
            let mut found = None;
            let mut silent: Vec<String> = Vec::new();
            for (backend, verdict) in chain.iter().zip(verdicts) {
                match verdict {
                    Verdict::Has => {
                        found = Some(backend.clone());
                        break;
                    }
                    Verdict::Lacks => {}
                    Verdict::CouldNotTell(why) => silent.push(why),
                }
            }
            match found {
                // Recorded only when every manager ahead of the winner actually said no.
                // If one of them could not answer, this pick is the best available guess
                // and not a decision: leaving it out of the lock is what makes the next
                // sync ask again, and move the package once the silent manager is back.
                Some(backend) if silent.is_empty() => {
                    debug!("`{}` resolved to `{}`.", name, backend);
                    lock_changed |= lock.record(&name, &backend);
                    answers.insert(name, backend);
                }
                Some(backend) => {
                    warn!(
                        "`{}` is being taken from `{}` only because {}. Not recorded — the \
                         next sync asks again, and moves `{}` if the manager that could not \
                         answer turns out to have it.",
                        name,
                        backend,
                        silent.join("; "),
                        name,
                    );
                    answers.insert(name, backend);
                }
                // Every candidate was asked and none has it — except that some could not
                // be asked, and "not found" would then be a lie.
                None if !silent.is_empty() => {
                    let grammar = GrammarError::new(
                        origin,
                        format!(
                            "no package manager this line accepts has `{}` — and {}",
                            name,
                            silent.join("; "),
                        ),
                    )
                    .with_hint(
                        "this may not be a misspelling. A manager that cannot reach its \
                         package index says nothing, which reads the same as a manager \
                         that does not have it — fix that manager and run again.",
                    );
                    return Err(Error::Unresolvable {
                        message: grammar.to_string(),
                        name,
                    });
                }
                // No candidate has it, so there is no honest answer to give. The old code
                // fell back to a default backend, which turned a typo into a request to
                // install a package that does not exist, reported by whichever backend
                // happened to be first (P3).
                None => {
                    let grammar = GrammarError::new(
                        origin,
                        format!("no package manager this line accepts has `{}`.", name),
                    )
                    .with_hint(if chain.is_empty() {
                        format!(
                            "{} — and none of them is in your `priority` file, so Shall may \
                             not use any of them here.",
                            describe_candidates(&candidates)
                        )
                    } else {
                        format!(
                            "tried {} in order. Check the spelling, or name a manager on the \
                             line if it comes from somewhere else.",
                            chain.join(", ")
                        )
                    });
                    return Err(Error::Unresolvable {
                        message: grammar.to_string(),
                        name,
                    });
                }
            }
        }

        // Written only when it changed: an unchanged lock rewritten every run would make
        // every sync a commit (V.30 commits on success, and there would always be something).
        // And only by a run that acts: a preview that froze the backend it guessed at made
        // the real install afterwards use that guess.
        if lock_changed && self.may_record_locks && !self.config.dry_run {
            lock.save(&lock_path)?;
        }
        Ok(answers)
    }

    /// Locked mode: nothing floats. A package with no lock entry is an error, and a
    /// hand-written pin that disagrees with the lock is reported rather than quietly
    /// resolved one way.
    fn apply_locks(&self, state: &mut DesiredState) -> Result<()> {
        // `--upgrade`: the run that means to move forward ignores what was recorded.
        if !self.locked && !self.prefer_locks {
            return Ok(());
        }
        for (backend, specs) in state.packages.iter_mut() {
            for spec in specs.iter_mut() {
                if !spec.present {
                    continue;
                }
                let key = format!("{}:{}", backend, spec.name);
                let Some(locked) = self.locks.get(&key) else {
                    if self.locked {
                        return Err(Error::Validation(format!(
                            "Locked Mode Error: '{}' is missing from locks/versions.json.",
                            key
                        )));
                    }
                    // Nothing recorded for this one: it resolves freely, which is what every
                    // package on a machine that has never run `shall lock` does.
                    continue;
                };
                if let Some(pinned) = spec.options.one("version") {
                    if pinned != locked {
                        // A hand-written pin that disagrees with the lock is never quietly
                        // resolved one way: under strict mode it is an error, and otherwise
                        // the line wins, because a version you typed is a decision and the
                        // lock is only a record of one.
                        if self.locked {
                            return Err(Error::Validation(format!(
                                "Integrity Failure: {} version mismatch. Manifest: {}, Lock: {}.",
                                key, pinned, locked
                            )));
                        }
                        debug!(
                            "{}: the line pins {} and the lock records {}; the line wins.",
                            key, pinned, locked
                        );
                    }
                    continue;
                }
                // **Recorded everywhere, replayed only where it can be replayed** (`Q53`).
                //
                // A lockfile does two jobs and they are not the same job: *reproduce* needs the
                // manager to accept a version, *detect drift* only needs it to report one. Job 2
                // works on every manager, so `lock` records on every manager — and feeding that
                // record back as an install argument is what killed the macOS run, where brew's
                // observed `tokei 14.0.0` became `tokei@14.0.0`, a formula that does not exist.
                //
                // Not injecting here is also what makes the planner's refusal correct: after
                // this, a version on a manager that cannot pin can only have come from a line
                // somebody typed, so it can be refused as a decision rather than guessed about.
                if !self.registry.pins_version(backend) {
                    if self.locked {
                        warn!(
                            "{} is recorded at {} and `{}` cannot install an exact version, so \
                             this run reproduces whatever it offers today.",
                            key, locked, backend
                        );
                    } else {
                        debug!(
                            "{}: {} is recorded but `{}` cannot replay a version; the record \
                             stays a drift reference.",
                            key, locked, backend
                        );
                    }
                    continue;
                }
                spec.options.set("version".to_string(), locked.clone());
            }
        }
        // `--locked` promises an exact machine, so a pin it cannot replay is fatal here rather
        // than a named skip: a run whose whole purpose is "reproduce this" must not report
        // success over a package it resolved freely. Without the flag the planner refuses the
        // package by name and the rest of the manifest proceeds — same fact, two severities, one
        // implementation in `sync::pins` so the rule cannot hold on one command and not its
        // neighbour.
        if self.locked {
            let unmeetable = crate::app::sync::pins::unmeetable(
                &self.registry,
                state
                    .packages
                    .iter()
                    .flat_map(|(b, specs)| specs.iter().map(move |s| (b.as_str(), s))),
            );
            if !unmeetable.is_empty() {
                return Err(Error::Validation(format!(
                    "Locked Mode Error: {} declared version(s) cannot be reproduced.\n  {}",
                    unmeetable.len(),
                    unmeetable
                        .iter()
                        .map(UnmeetablePin::message)
                        .collect::<Vec<_>>()
                        .join("\n  ")
                )));
            }
        }
        Ok(())
    }

    /// Parse one package from a command line (`shall run jq`, a shell request).
    ///
    /// The same grammar and the same probe as a line in a module. P1: an imperative command
    /// is a shortcut for editing a file, so it must not be a second dialect.
    /// The static half of `parse_and_probe_spec`: does this line name a backend Shall uses?
    ///
    /// Answers without asking any manager anything, so it is cheap enough to run before a
    /// write. A bare name has no backend to check and passes; a `repo:` names one the same
    /// way a package does, and `collect` refuses it in the file, so it is refused here too.
    pub async fn validate_line(&self, line: &str) -> Result<()> {
        let facts = self.facts_for_host().await?;
        let priority = self.priority(&facts).await?;
        let known = self.vocab(&priority);

        let origin = Origin::argument();
        let named = match statement::parse(&origin, line.trim(), &known)? {
            Statement::Package(d) | Statement::Absent(d) => d.backend,
            Statement::Repo { backend, .. } => Some(backend),
            _ => None,
        };
        let Some(b) = named else { return Ok(()) };
        let b = self.config.aliases.get(&b).cloned().unwrap_or(b);
        if !priority.allows(&b) {
            return Err(Error::from(priority.reject(&b, &origin)));
        }
        Ok(())
    }

    /// The `(backend, name)` a query string denotes when it is a typed resource statement whose
    /// prefix is also a backend — `service:com.apple.X`, `link:/etc/foo`, `setting:S/K`.
    ///
    /// Three prefixes are both grammar keywords and registered backends, and `shall list`
    /// prints them as those two columns. A user who copies a row out of a listing therefore
    /// hands a string the *declaration* parser reads as a resource, and a caller that only
    /// understands packages sees nothing at all. That is R-4: `list` reported
    /// `service:com.apple.SafariHistoryServiceAgent` and `info` about that exact name answered
    /// "is not installed on this machine".
    ///
    /// Through the same parser as everything else, and the pair comes off the variant rather
    /// than off a second split.
    pub async fn queried_resource(&self, line: &str) -> Result<Option<(String, String)>> {
        let facts = self.facts_for_host().await?;
        let priority = self.priority(&facts).await?;
        let known = self.vocab(&priority);
        let stmt = statement::parse(&Origin::argument(), line.trim(), &known)?;
        Ok(stmt
            .listed_as()
            .map(|(backend, name)| (backend.to_string(), name.to_string())))
    }

    /// The backend a declaration *names*, if it names one: `Some("cargo")` for `cargo:ripgrep`,
    /// `None` for a bare `ripgrep`.
    ///
    /// Through the same parser [`Self::parse_and_probe_spec`] uses, never a `split_once(':')` —
    /// a second place that decides what a prefix means is the bug `CLAUDE.md` names and C13
    /// records six times over.
    ///
    /// Callers need this to tell *the user named a manager* from *Shall picked one*, which are
    /// different questions with different right answers. `info ripgrep` reported a package the
    /// machine has as absent because `priority` picked `choco`, `choco` had nothing, and the
    /// answer from the manager that did have it was never asked for (N-3).
    pub async fn declared_backend(&self, line: &str) -> Result<Option<String>> {
        let facts = self.facts_for_host().await?;
        let priority = self.priority(&facts).await?;
        let known = self.vocab(&priority);
        let stmt = statement::parse(&Origin::argument(), line.trim(), &known)?;
        let decl = match stmt {
            Statement::Package(d) | Statement::Absent(d) => d,
            _ => return Ok(None),
        };
        // An alias is a name for a backend, so resolve it here or a caller checking the answer
        // against the registry would reject a spelling the rest of the program accepts.
        Ok(decl
            .backend
            .map(|b| self.config.aliases.get(&b).cloned().unwrap_or(b)))
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
                "`{}` is not a backend Shall uses\n  \
                 add `{}` to your `priority` file, or check the spelling. Not listed means \
                 Shall does not use it at all.",
                name, name
            ))),
            Some(b) => {
                if b.is_available() {
                    Ok(true)
                } else {
                    warn!(
                        "`{}` is a manager Shall knows, but it is not installed on this \
                         machine — so there is nothing for it to report. `shall check health` \
                         says which managers are ready here.",
                        name
                    );
                    Ok(false)
                }
            }
        }
    }

    /// Refuse a `backend:name` argument whose prefix is not a backend (Q9).
    ///
    /// Q9 ruled that every verb taking a backend name refuses an unknown one, and listed the
    /// four that take it as a `--backend` flag — "checked from the code rather than from the one
    /// that was reported". The `backend:name` *spec* form was not in that enumeration, so the
    /// ruling was applied to half its surface: `shall hold nosuchbackend:foo` recorded a hold
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

    pub async fn parse_and_probe_spec(&self, line: &str) -> Result<PackageSpec> {
        let facts = self.facts_for_host().await?;
        let priority = self.priority(&facts).await?;
        let known = self.vocab(&priority);

        let origin = Origin::argument();
        let stmt = statement::parse(&origin, line.trim(), &known)?;
        let (mut decl, present) = match stmt {
            Statement::Package(d) => (d, true),
            Statement::Absent(d) => (d, false),
            _ => {
                return Err(Error::Config(format!(
                    "`{}` is not a package.",
                    line.trim()
                )))
            }
        };

        if let Some(b) = &decl.backend {
            if let Some(real) = self.config.aliases.get(b) {
                decl.backend = Some(real.clone());
            }
        }

        let backend = match &decl.backend {
            Some(b) => {
                if !priority.allows(b) {
                    return Err(Error::from(priority.reject(b, &origin)));
                }
                b.clone()
            }
            None => {
                let stmts = vec![(
                    Statement::Package(decl.clone()),
                    origin.clone(),
                    Gates::new(),
                )];
                let answers = self
                    .probe_bare_names(&stmts, &priority, Coverage::OneLine)
                    .await?;
                answers
                    .get(decl.selector.as_str())
                    .cloned()
                    .unwrap_or_else(|| BARE.to_string())
            }
        };

        // No scopes: this came from a command line, so it is in no module and no profile.
        // `--module dev` must not match it, and it has nothing to be untrue about.
        let spec = to_spec(
            &backend,
            &decl.selector,
            &decl.options,
            present,
            priority.options(&backend),
            Provenance {
                origin: &origin,
                scopes: &[],
                gates: &[],
            },
        );
        Validator::validate_package_name_for(&spec.name, &spec.backend)?;
        Ok(spec)
    }

    /// One command-line spec, plus everything its `@requires` chain pulls in.
    ///
    /// Lives here because [`parse_and_probe_spec`](Self::parse_and_probe_spec) does, and
    /// callers that own only a registry and a config — `App` and `Runner` — each kept an
    /// identical copy of this walk.
    pub async fn resolve_spec(&self, spec_str: &str) -> Result<Vec<PackageSpec>> {
        let mut resolved = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        let mut seen = std::collections::HashSet::new();

        queue.push_back(self.parse_and_probe_spec(spec_str).await?);

        while let Some(spec) = queue.pop_front() {
            let key = format!("{}:{}", spec.backend, spec.name);
            if !seen.insert(key) {
                continue;
            }

            Validator::validate_package_name_for(&spec.name, &spec.backend)?;
            for req in &spec.requires {
                queue.push_back(self.parse_and_probe_spec(req).await?);
            }
            resolved.push(spec);
        }
        Ok(resolved)
    }

    /// Ask one manager whether it has a name, keeping "it does not" apart from "it could
    /// not say".
    ///
    /// A manager this machine does not have, and one with no way to search at all, both
    /// answer `Lacks`: those are settled facts about the machine, and asking again next
    /// run would get the same answer. Only a command that failed is `CouldNotTell`.
    /// Every manager in the chain's verdict, **in the chain's order**, asked in as few rounds
    /// as the answer allows.
    ///
    /// The winner is still *the first manager in `priority` that has the name* — the caller
    /// walks this list in order and takes the first `Has`, exactly as the serial loop did. What
    /// changes is the waiting. Measured on Windows with 23 managers in `priority`, a bare name
    /// no manager carries:
    ///
    /// ```text
    /// shall eval, bare name nothing claims     18.1 s  (103 s cold)
    /// shall eval, the same fixture qualified    0.31 s
    /// ```
    ///
    /// Every manager has to be asked when nobody has the name, so the cost is not the number of
    /// questions — it is asking them one at a time, each a network round trip.
    ///
    /// **The first is still asked alone.** That is the case `priority` exists to make cheap: a
    /// bare name usually comes from the manager at the top of the list, and speculating across
    /// the rest would spend twenty-two extra queries — one of them against the GitHub API, whose
    /// rate limit is what R-3 was about — to save nothing. Only once the priority winner has
    /// said no is the rest of the chain asked at once, which is the shape where the fan-out was
    /// going to happen anyway.
    ///
    /// `ask` is read-only (one `lookup` per candidate), which is what makes the order
    /// of asking a question about latency rather than about behaviour.
    async fn ask_the_chain(
        &self,
        chain: &[String],
        name: &str,
        constraint: Option<&str>,
    ) -> Vec<Verdict> {
        let Some((first, rest)) = chain.split_first() else {
            return Vec::new();
        };

        let head = self.ask(first, name, constraint).await;
        if matches!(head, Verdict::Has) || rest.is_empty() {
            // Done, and nobody else was troubled. The caller stops at the first `Has`, so the
            // verdicts it never reads are the ones never asked for.
            return vec![head];
        }

        use futures::stream::{FuturesOrdered, StreamExt};
        let cap = self.config.max_parallel.max(1);
        let mut queued = rest
            .iter()
            .map(|backend| self.ask(backend, name, constraint));
        let mut ordered = FuturesOrdered::new();
        for _ in 0..cap {
            match queued.next() {
                Some(fut) => ordered.push_back(fut),
                None => break,
            }
        }
        let mut out = vec![head];
        while let Some(verdict) = ordered.next().await {
            out.push(verdict);
            if let Some(fut) = queued.next() {
                ordered.push_back(fut);
            }
        }
        out
    }

    async fn ask(
        &self,
        backend_name: &str,
        package_name: &str,
        constraint: Option<&str>,
    ) -> Verdict {
        let Some(backend_cap) = self.registry.get(backend_name).filter(|b| b.is_available()) else {
            return Verdict::Lacks;
        };
        let Some(searchable) = backend_cap.as_searchable() else {
            return Verdict::Lacks;
        };

        // The one cap on remote work, held by the leaf that does it. Both fan-outs above —
        // over names and over candidates — queue here rather than multiplying.
        let Ok(_permit) = self.remote_gate.acquire().await else {
            return Verdict::CouldNotTell("the resolver was shutting down".to_string());
        };

        // One query per candidate. Presence and version come out of the same answer, so a
        // rejected candidate costs one search instead of two and a pinned name one instead of
        // three.
        let found = match searchable.lookup(package_name).await {
            Ok(Some(pkg)) => pkg,
            Ok(None) => return Verdict::Lacks,
            Err(e) => return Verdict::CouldNotTell(e.to_string()),
        };

        let Some(req) = constraint else {
            return Verdict::Has;
        };
        // It has the package but will not say which version. The manager is the one that
        // enforces the pin at install time; refusing here would send the name to a manager that
        // merely talks about versions more.
        match found.version.as_deref() {
            Some(ver) if !self.satisfies_constraint(ver, req) => Verdict::Lacks,
            _ => Verdict::Has,
        }
    }

    fn satisfies_constraint(&self, version: &str, constraint: &str) -> bool {
        if constraint == "latest" || constraint == "*" || constraint.is_empty() {
            return true;
        }

        // SemVer first, then literal, then loose: package managers ship versions SemVer
        // cannot parse (epochs, distro suffixes), and those must still be comparable.
        if let Ok(req) = VersionReq::parse(constraint) {
            if let Ok(ver) = Version::parse(version) {
                return req.matches(&ver);
            }
        }

        if version == constraint {
            return true;
        }

        match loose_compare(version, constraint) {
            Ok(Cmp::Eq) => true,
            Ok(Cmp::Gt) if constraint.starts_with('>') => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tempfile::{tempdir, TempDir};

    /// A real repo on disk, in the II.1 layout.
    struct Repo {
        _tmp: TempDir,
        config: Config,
    }

    fn repo(files: &[(&str, &str)]) -> Repo {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("cfg");
        for (path, body) in files {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, body).unwrap();
        }
        let config = Config {
            // `root` is the repo root the layout hangs off (modules/, profiles/, active).
            config_root: root.clone(),
            ..Config::default()
        };
        Repo { _tmp: tmp, config }
    }

    /// The property `ask_the_chain` rests on, asserted against a case built to break it.
    ///
    /// A bare name goes to **the first manager in `priority` that has it**. Asking the chain
    /// concurrently is only a question about latency while the answers come back in the chain's
    /// order — swap `FuturesOrdered` for `FuturesUnordered` and the winner becomes whichever
    /// manager replied first, which on a slow morning is a different package.
    ///
    /// So the futures here finish in exactly the reverse of the order they are queued, and the
    /// window is smaller than the set, which is the arrangement that makes a completion-ordered
    /// stream visibly wrong.
    #[tokio::test]
    async fn the_chain_answers_in_the_chains_order_however_the_answers_arrive() {
        use futures::stream::{FuturesOrdered, StreamExt};
        use std::time::Duration;

        let delays = [50u64, 40, 30, 20, 10, 0];
        let cap = 2;

        let mut queued = delays.iter().enumerate().map(|(i, ms)| async move {
            tokio::time::sleep(Duration::from_millis(*ms)).await;
            i
        });
        let mut ordered = FuturesOrdered::new();
        for _ in 0..cap {
            match queued.next() {
                Some(f) => ordered.push_back(f),
                None => break,
            }
        }
        let mut out = Vec::new();
        while let Some(i) = ordered.next().await {
            out.push(i);
            if let Some(f) = queued.next() {
                ordered.push_back(f);
            }
        }

        assert_eq!(
            out,
            vec![0, 1, 2, 3, 4, 5],
            "the verdicts came back in completion order, so the caller's `zip` would pair each \
             verdict with the wrong manager and a bare name would resolve to whichever manager \
             was quickest to answer"
        );
    }

    async fn resolve(r: &Repo) -> Result<HashMap<String, Vec<PackageSpec>>> {
        let registry = Arc::new(BackendRegistry::new());
        StateResolver::new(&r.config, registry, false)
            .await
            .resolve_desired_state()
            .await
    }

    fn names(map: &HashMap<String, Vec<PackageSpec>>, backend: &str) -> Vec<String> {
        let mut v: Vec<String> = map
            .get(backend)
            .map(|specs| {
                specs
                    .iter()
                    .filter(|s| s.present)
                    .map(|s| s.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    mod silent_managers {
        use super::*;
        use crate::backends::generic::SearchSource;
        use crate::backends::generic::{
            GenericBackendCore, GenericSearchable, ManagerConfig, ManualListing,
        };
        use crate::core::executor::{DryRunOutput, MockExecutor};
        use crate::core::{BackendCapabilities, CommandExecutor, Package};
        use dashmap::DashMap;
        use std::path::PathBuf;
        use std::process::Output as StdOutput;

        fn one_per_line(output: &str) -> crate::parsers::ParseResult {
            crate::parsers::parse_bare_names(output, "test")
        }

        fn one_per_line_search(output: &str) -> Vec<Package> {
            crate::parsers::parse_bare_names(output, "test").unwrap_or_default()
        }

        fn manager(name: &str, exec: CommandExecutor) -> Arc<BackendCapabilities> {
            let config = ManagerConfig {
                name: name.into(),
                binary: None,
                remove_binary: None,
                install_args: vec![],
                remove_args: vec![],
                list_args: vec![],
                manual: ManualListing::AllInstalled,
                essential_args: None,
                search_args: vec!["search".into()],
                search_binary: None,
                enumerate_args: None,
                enumerate_binary: None,
                list_binary: None,
                upgrade_args: vec![],
                update_args: None,
                purge_args: None,
                orphan_dry_run: None,
                repo_add_args: None,
                repo_remove_args: None,
                depends: None,
                clean_cache: None,
                repo_list_args: None,
                repo_binary: None,
                repo_list_binary: None,
                repo_remove_binary: None,
                repo_list_shape: crate::backends::generic::RepoListing::Columns,
                version_pin: None,
                needs_root: false,
                is_exclusive: false,
                install_source_option: None,
                extra_probes: None,
                upgrade_reinstall_args: None,
                property_probes: Vec::new(),
                machine_list: None,
                outdated: None,
                search_source: SearchSource::Command,
            };
            let core = Arc::new(GenericBackendCore {
                name: name.into(),
                executor: exec,
                config,
                parser: Arc::new(crate::parsers::LambdaParser {
                    installed_fn: one_per_line,
                    search_fn: one_per_line_search,
                }),
            });
            Arc::new(
                BackendCapabilities::builder(core.clone())
                    .with_searchable(Arc::new(GenericSearchable { core }))
                    .build(),
            )
        }

        /// Two managers named on `priority`, each answering a `search jq` however the
        /// test says.
        fn registry(first: StdOutput, second: StdOutput) -> Arc<BackendRegistry> {
            let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
            let mock = Arc::new(MockExecutor::new(vfs.clone()));
            mock.set_command_exists("first", true);
            mock.set_command_exists("second", true);
            mock.set_response("first search jq", Ok(first));
            mock.set_response("second search jq", Ok(second));
            let exec =
                CommandExecutor::with_layer(false, false, mock, vfs, Arc::new(DashMap::new()));
            let mut reg = BackendRegistry::new();
            reg.register(manager("first", exec.clone()));
            reg.register(manager("second", exec));
            Arc::new(reg)
        }

        fn found() -> StdOutput {
            let mut out: StdOutput = DryRunOutput::new().into();
            out.stdout = b"jq\n".to_vec();
            out
        }

        async fn settle(r: &Repo, reg: Arc<BackendRegistry>) -> Result<(String, Option<String>)> {
            let state = StateResolver::new(&r.config, reg, false)
                .await
                .recording_locks()
                .resolve_model()
                .await?;
            let backend = state
                .packages
                .iter()
                .find(|(_, specs)| specs.iter().any(|s| s.name == "jq"))
                .map(|(b, _)| b.clone())
                .expect("jq resolved somewhere");
            let lock = crate::core::BareLock::load(&crate::core::BareLock::path_in(
                &r.config.layout().locks_dir(),
            ))
            .unwrap();
            let recorded = lock.get("jq").map(str::to_string);
            Ok((backend, recorded))
        }

        fn bare_jq() -> Repo {
            repo(&[
                ("priority", "first\nsecond\n"),
                ("active", "Work\n"),
                ("profiles/Work", "use base\n"),
                ("modules/base.txt", "jq\n"),
            ])
        }

        #[tokio::test]
        async fn a_manager_that_said_no_lets_the_pick_be_recorded() {
            let (backend, recorded) =
                settle(&bare_jq(), registry(DryRunOutput::new().into(), found()))
                    .await
                    .unwrap();
            assert_eq!(backend, "second");
            assert_eq!(recorded.as_deref(), Some("second"));
        }

        /// The ruling: a manager that could not answer has not said no, so the name still
        /// falls through — but nothing is written down, and the next sync asks again.
        #[tokio::test]
        async fn a_manager_that_could_not_answer_leaves_no_lock() {
            let (backend, recorded) = settle(
                &bare_jq(),
                registry(DryRunOutput::faulted("E: package lists are empty"), found()),
            )
            .await
            .unwrap();
            assert_eq!(backend, "second");
            assert_eq!(recorded, None, "a guess must not be frozen");
        }

        /// And when nothing has it either, "no such package" would be a lie.
        #[tokio::test]
        async fn nothing_found_past_a_silent_manager_says_so() {
            let err = settle(
                &bare_jq(),
                registry(
                    DryRunOutput::faulted("E: package lists are empty"),
                    DryRunOutput::new().into(),
                ),
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(err.contains("could not answer"), "{}", err);
            assert!(err.contains("`first`"), "{}", err);
            assert!(err.contains("not be a misspelling"), "{}", err);
        }
    }

    #[tokio::test]
    async fn the_seam_carries_what_the_active_profiles_reach() {
        let r = repo(&[
            ("priority", "apt\ncargo\n"),
            ("active", "Work\n"),
            ("profiles/Work", "use editors\n"),
            ("modules/editors.txt", "apt:neovim\ncargo:ripgrep\n"),
            ("modules/gaming.txt", "apt:steam\n"),
        ]);
        let map = resolve(&r).await.unwrap();
        assert_eq!(names(&map, "apt"), ["neovim"]);
        assert_eq!(names(&map, "cargo"), ["ripgrep"]);
        // Nothing is active unless a profile names it: `gaming` was never reached.
        assert!(!names(&map, "apt").contains(&"steam".to_string()));
    }

    #[tokio::test]
    async fn a_missing_priority_file_is_an_error_that_names_it() {
        // Not a detected default. Which package managers this machine uses is a thing you
        // declare, and guessing it is what V.15 exists to stop.
        let r = repo(&[("active", "Work\n"), ("profiles/Work", "apt:curl\n")]);
        let err = resolve(&r).await.unwrap_err().to_string();
        assert!(err.contains("priority"), "{}", err);
        assert!(err.contains("one per line"), "{}", err);
    }

    #[tokio::test]
    async fn a_backend_missing_from_priority_is_refused_by_name() {
        let r = repo(&[
            ("priority", "apt\n"),
            ("active", "Work\n"),
            ("profiles/Work", "use base\n"),
            ("modules/base.txt", "snap:foo\n"),
        ]);
        let err = resolve(&r).await.unwrap_err().to_string();
        // Two refusals guard this, and which one fires depends on whether the backend is
        // one Shall has ever heard of: the grammar refuses a prefix that names nothing,
        // `priority` refuses a real backend you did not list (V.15). Both must name the
        // backend, point at `priority`, and say where the line is — an error that cannot
        // be located cannot be fixed.
        assert!(err.contains("snap"), "{}", err);
        assert!(err.contains("priority"), "{}", err);
        assert!(err.contains("base.txt:1"), "{}", err);
        // Never silently dropped, which is what the old resolver did with a backend it
        // did not recognise.
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn an_absent_line_crosses_the_seam_marked_absent() {
        // The map is the seam, so `absent:` shares it and carries `present: false`. The
        // planner splits them; nothing may read the map as a plain wish list.
        let r = repo(&[
            ("priority", "apt\n"),
            ("active", "Work\n"),
            ("profiles/Work", "use base\n"),
            ("modules/base.txt", "apt:curl\nabsent:apt:libreoffice\n"),
        ]);
        let map = resolve(&r).await.unwrap();
        let apt = map.get("apt").unwrap();
        assert_eq!(names(&map, "apt"), ["curl"]);
        let absent: Vec<&str> = apt
            .iter()
            .filter(|s| !s.present)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(absent, ["libreoffice"]);
    }

    #[tokio::test]
    async fn a_contradiction_across_two_modules_is_an_error_naming_both() {
        // Part IV requires this proof, through the seam and not just in the model.
        let r = repo(&[
            ("priority", "apt\n"),
            ("active", "Work\n"),
            ("profiles/Work", "use a\nuse b\n"),
            ("modules/a.txt", "apt:jq@version=1.6\n"),
            ("modules/b.txt", "apt:jq@version=1.7\n"),
        ]);
        let err = resolve(&r).await.unwrap_err().to_string();
        assert!(err.contains("a.txt"), "{}", err);
        assert!(err.contains("b.txt"), "{}", err);
    }

    #[tokio::test]
    async fn a_package_is_scoped_to_its_module_and_to_the_profile_that_reaches_it() {
        // What `upgrade --module dev` and `upgrade --profile Work` match on.
        let r = repo(&[
            ("priority", "apt\n"),
            ("active", "Work\n"),
            ("profiles/Work", "use dev\n"),
            ("modules/dev.txt", "apt:curl\n"),
        ]);
        let map = resolve(&r).await.unwrap();
        let curl = &map.get("apt").unwrap()[0];
        let scopes = curl.options.all("__scopes");
        assert!(scopes.iter().any(|s| s == "module:dev"), "{scopes:?}");
        assert!(scopes.iter().any(|s| s == "profile:Work"), "{scopes:?}");
        // And `__source` stays the human answer to "where is this line?".
        assert!(curl.options.one("__source").unwrap().contains("dev.txt:1"));
    }

    #[tokio::test]
    async fn a_module_reached_through_another_module_keeps_its_own_scope() {
        let r = repo(&[
            ("priority", "apt\n"),
            ("active", "Work\n"),
            ("profiles/Work", "use dev\n"),
            ("modules/dev.txt", "use base\napt:curl\n"),
            ("modules/base.txt", "apt:jq\n"),
        ]);
        let map = resolve(&r).await.unwrap();
        let jq = map
            .get("apt")
            .unwrap()
            .iter()
            .find(|s| s.name == "jq")
            .unwrap();
        let scopes = jq.options.all("__scopes");
        assert!(scopes.iter().any(|s| s == "module:base"), "{scopes:?}");
        assert!(scopes.iter().any(|s| s == "profile:Work"), "{scopes:?}");
    }

    #[tokio::test]
    async fn an_unreached_broken_module_is_never_parsed() {
        // II.3: Shall only parses what the active profiles reach. `shall check` is the
        // command that parses everything.
        let r = repo(&[
            ("priority", "apt\n"),
            ("active", "Work\n"),
            ("profiles/Work", "use base\n"),
            ("modules/base.txt", "apt:curl\n"),
            ("modules/broken.txt", "!!! not a statement !!!\n"),
        ]);
        assert!(resolve(&r).await.is_ok());
    }
}
