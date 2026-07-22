use crate::backends::BackendRegistry;
use crate::app::vocab::Vocab;
use crate::config::grammar::{statement, Candidates, Gates, GrammarError, Origin, Statement};
use crate::config::parser::HostFacts;
use crate::config::Config;
use crate::core::{Error, PackageSpec, Result, Validator};
use crate::model::resolve::{to_spec, Provenance, BARE};
use crate::model::{DesiredState, Layout, Priority};
use semver::{Version, VersionReq};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, instrument, warn};
use version_compare::{compare as loose_compare, Cmp};

/// Whether the statements handed to the prober are the whole model.
///
/// Only then does a name's absence mean it is no longer declared. A single `linix run jq` is
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
    /// When true, a package with no entry in locks/versions.json is an error rather than a free
    /// resolve — the whole point of a locked run is that nothing floats.
    locked: bool,
    /// "backend:package" -> version.
    locks: HashMap<String, String>,
    /// Pre-resolved variables to use instead of running the provider (Part IX, IX.6). Set when
    /// applying a saved plan: re-running a clock/shell/network provider at apply time could
    /// disagree with what the plan froze, so `apply` resolves the model against the plan's own
    /// variables. `None` resolves them fresh, which is what every non-plan path does.
    vars_override: Option<crate::model::vars::Vars>,
    /// Whether this resolution may freeze an unpinned name's backend into this host's
    /// `locks/bare.HOST.toml`.
    ///
    /// Recording is a decision, so only a run that goes on to change the machine makes it.
    /// Off unless a caller says otherwise: forgetting to ask means a command reads without
    /// leaving a mark, which is the harmless direction to be wrong in.
    may_record_locks: bool,
}

impl<'a> StateResolver<'a> {
    pub async fn new(config: &'a Config, registry: Arc<BackendRegistry>, locked: bool) -> Self {
        let mut locks = HashMap::new();

        if locked {
            let lock_path = config.config_root().join("locks").join("versions.json");
            debug!(
                "Locked mode active. Probing for locks at {:?}",
                lock_path
            );

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
            } else {
                warn!("Locked mode requested but locks/versions.json is missing.");
            }
        }

        Self {
            config,
            registry,
            layout: config.layout(),
            locked,
            locks,
            vars_override: None,
            may_record_locks: false,
        }
    }

    /// Resolve the model against these already-resolved variables instead of running the
    /// provider (used by `apply` to reuse a saved plan's frozen variables).
    pub fn with_vars(mut self, vars: crate::model::vars::Vars) -> Self {
        self.vars_override = Some(vars);
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
    /// A missing file is an error and not a detected default. LiNix cannot pick your
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
                return Err(Error::Config(format!(
                    "no `priority` file at {}.\n  \
                     `priority` lists the package managers LiNix may use, one per line, best \
                     first — for example:\n\n    apt\n    cargo\n\n  \
                     Listed means LiNix uses it. Not listed means LiNix does not touch it at \
                     all.",
                    file.display()
                )))
            }
            Err(e) => return Err(Error::from(e)),
        };
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

    /// Resolve the variables against the given facts — the one implementation, so `linix vars`
    /// prints what a `when` will see rather than a second opinion about it.
    async fn resolve_vars_against(
        &self,
        facts: &HostFacts,
    ) -> Result<(crate::model::vars::Vars, crate::model::vars::VarOrigins)> {
        let priority = self.vars_vocabulary().await?;
        let known = Vocab::new(&self.registry, self.config, &priority);
        crate::model::Resolver::new(&self.layout, &known, &priority)
            .with_facts(facts.clone())
            .with_vars_source(self.config.vars.source.clone())
            .load_vars_with_origins()
            .map_err(Error::from)
    }

    /// Resolve just the variables (Part IX), without planning the whole model — for `linix vars`.
    /// The same resolution `resolve_model` performs, so what this prints is what a `when` sees.
    pub async fn resolve_vars(&self) -> Result<crate::model::vars::Vars> {
        self.resolve_vars_with_origins().await.map(|(v, _)| v)
    }

    /// [`resolve_vars`], plus where each variable was set — for `linix vars` and `why`, which
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
        let known = Vocab::new(&self.registry, self.config, &priority);
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
        let known = Vocab::new(&self.registry, self.config, &priority);
        Ok(crate::model::Resolver::new(&self.layout, &known, &priority)
            .with_facts(facts)
            .parse_everything())
    }

    #[instrument(skip(self))]
    pub async fn resolve_desired_state(&self) -> Result<HashMap<String, Vec<PackageSpec>>> {
        Ok(self.resolve_model().await?.packages)
    }

    /// II.7, end to end: `active` -> profiles -> the modules they reach -> the desired state.
    ///
    /// The map the seam carries holds `absent:` lines too, marked `present: false`; the
    /// planner splits them out. Everything below the seam — `src/backends/`, `src/core/`,
    /// `src/parsers/` — is untouched by any of this.
    pub async fn resolve_model(&self) -> Result<DesiredState> {
        let facts = self.facts_for_host().await?;
        let priority = self.priority(&facts).await?;
        let known = Vocab::new(&self.registry, self.config, &priority);

        debug!("resolving desired state for host '{}'", facts.host);

        // Steps 1-3 read the files. Probing needs the network, so it happens out here,
        // between reading and merging: a bare `ripgrep` and an explicit `cargo:ripgrep` are
        // one package, and they only meet if the answer is known before the merge (V.16).
        let mut reached = crate::model::Resolver::new(&self.layout, &known, &priority)
            .with_facts(facts.clone())
            .statements()?;
        self.resolve_aliases(&mut reached.statements);
        self.expand_regexes(&mut reached.statements, &priority).await?;
        let answers = self
            .probe_bare_names(&reached.statements, &priority, Coverage::WholeModel)
            .await?;

        let mut state = crate::model::Resolver::new(&self.layout, &known, &priority)
            .with_facts(facts)
            .with_bare(answers)
            .collect(reached)?;

        for specs in state.packages.values_mut() {
            for spec in specs.iter_mut() {
                Validator::validate_package_name_for(&spec.name, &spec.backend)?;
            }
        }

        self.apply_locks(&mut state)?;

        // II.16: an expired line lingers, because LiNix must not rewrite your files. It
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
        if !statements
            .iter()
            .any(|(s, ..)| matches!(s, Statement::Package(d) | Statement::Absent(d)
                if matches!(d.selector, crate::config::grammar::Selector::Regex(_))))
        {
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
                    debug!("`{}:re:{}` is frozen to {} name(s).", backend, pattern, frozen.len());
                    frozen.to_vec()
                }
                None => {
                    let found = self.match_catalogue(&backend, pattern, &origin, priority).await?;
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
                one.options.insert("__from_regex".to_string(), pattern.clone());
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

        let re = regex::Regex::new(pattern).map_err(|e| {
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
        debug!("`{}:re:{}` matched {} name(s).", backend, pattern, names.len());
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
        for (stmt, origin, _) in statements {
            let Statement::Package(decl) = stmt else {
                continue;
            };
            if decl.backend.is_some() {
                continue;
            }
            let name = decl.selector.as_str().to_string();
            if let Some(seen) = questions.iter().find(|q| q.name == name) {
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
            let constraint = decl.options.one("version").map(str::to_string);
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
        let lock_path =
            crate::core::BareLock::path_in(&self.layout.locks_dir());
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
        for question in questions {
            let Question { name, candidates, constraint, origin } = question;
            // A candidate `priority` does not list is not a candidate at all: `priority` says
            // which managers LiNix may use on this host, whatever a line asks for (V.15).
            let chain: Vec<String> = candidates
                .order(&listed)
                .into_iter()
                .filter(|b| priority.allows(b))
                .collect();

            if let Some(backend) = lock.get(&name).map(str::to_string) {
                // Honoured only when the line still accepts it and this machine still has
                // it. The lock exists to stop an unedited line quietly changing meaning — it
                // was never a licence to demand a manager that is not here.
                let usable = self.registry.get(&backend).is_some_and(|b| b.is_available());
                if chain.contains(&backend) && usable {
                    debug!("`{}` is locked to `{}`.", name, backend);
                    answers.insert(name, backend);
                    continue;
                }
                warn!(
                    "`{}` was locked to `{}`, which {}. Asking again.",
                    name,
                    backend,
                    if usable { "this line no longer accepts" } else { "this machine does not have" }
                );
            }

            let mut found = None;
            let mut silent: Vec<String> = Vec::new();
            for backend in &chain {
                match self.ask(backend, &name, constraint.as_deref()).await {
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
                        format!(
                            "no package manager this line accepts has `{}`.",
                            name
                        ),
                    )
                    .with_hint(if chain.is_empty() {
                        format!(
                            "{} — and none of them is in your `priority` file, so LiNix may \
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
                    })
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
        if !self.locked {
            return Ok(());
        }
        for (backend, specs) in state.packages.iter_mut() {
            for spec in specs.iter_mut() {
                if !spec.present {
                    continue;
                }
                let key = format!("{}:{}", backend, spec.name);
                let Some(locked) = self.locks.get(&key) else {
                    return Err(Error::Validation(format!(
                        "Locked Mode Error: '{}' is missing from locks/versions.json.",
                        key
                    )));
                };
                if let Some(pinned) = spec.options.get("version") {
                    if pinned != locked {
                        return Err(Error::Validation(format!(
                            "Integrity Failure: {} version mismatch. Manifest: {}, Lock: {}.",
                            key, pinned, locked
                        )));
                    }
                }
                spec.options.insert("version".to_string(), locked.clone());
            }
        }
        Ok(())
    }

    /// Parse one package from a command line (`linix run jq`, a shell request).
    ///
    /// The same grammar and the same probe as a line in a module. P1: an imperative command
    /// is a shortcut for editing a file, so it must not be a second dialect.
    pub async fn parse_and_probe_spec(&self, line: &str) -> Result<PackageSpec> {
        let facts = self.facts_for_host().await?;
        let priority = self.priority(&facts).await?;
        let known = Vocab::new(&self.registry, self.config, &priority);

        let origin = Origin::argument();
        let stmt = statement::parse(&origin, line.trim(), &known)?;
        let (mut decl, present) = match stmt {
            Statement::Package(d) => (d, true),
            Statement::Absent(d) => (d, false),
            _ => return Err(Error::Config(format!("`{}` is not a package.", line.trim()))),
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
                let stmts = vec![(Statement::Package(decl.clone()), origin.clone(), Gates::new())];
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

        let has = match searchable.remote_has(package_name).await {
            Ok(true) => true,
            // `false` here is not proof of absence — a backend may not implement it — so
            // an inconclusive answer falls through to a real search.
            Ok(false) => match searchable.search(package_name).await {
                Ok(results) => results.iter().any(|pkg| pkg.name == package_name),
                Err(e) => return Verdict::CouldNotTell(e.to_string()),
            },
            Err(e) => return Verdict::CouldNotTell(e.to_string()),
        };
        if !has {
            return Verdict::Lacks;
        }

        let Some(req) = constraint else {
            return Verdict::Has;
        };
        match searchable.remote_info(package_name).await {
            // It has the package but will not say which version. The manager is the one
            // that enforces the pin at install time; refusing here would send the name to
            // a manager that merely talks about versions more.
            Ok(Some(pkg)) => match pkg.version.as_deref() {
                Some(ver) if !self.satisfies_constraint(ver, req) => Verdict::Lacks,
                _ => Verdict::Has,
            },
            Ok(None) => Verdict::Lacks,
            Err(e) => Verdict::CouldNotTell(e.to_string()),
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
        use crate::backends::generic::{
            GenericBackendCore, GenericSearchable, ManagerConfig, ManualListing,
        };
        use crate::core::executor::{DryRunOutput, MockExecutor};
        use crate::core::{BackendCapabilities, CommandExecutor, Package};
        use std::process::Output as StdOutput;
        use dashmap::DashMap;
        use std::collections::HashMap as Map;
        use std::path::PathBuf;

        fn one_per_line(output: &str) -> Vec<Package> {
            crate::parsers::parse_bare_names(output, "test")
        }

        fn manager(name: &str, exec: CommandExecutor) -> Arc<BackendCapabilities> {
            let config = ManagerConfig {
                name: name.into(),
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
                orphan_args: None,
                repo_add_args: None,
                repo_remove_args: None,
                depends_args: None,
                repo_list_args: None,
                version_pin: None,
                needs_root: false,
                is_exclusive: false,
                flag_map: Map::new(),
            };
            let core = Arc::new(GenericBackendCore {
                name: name.into(),
                executor: exec,
                config,
                parser: Arc::new(crate::parsers::LambdaParser {
                    installed_fn: one_per_line,
                    search_fn: one_per_line,
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
            let exec = CommandExecutor::with_layer(
                false,
                false,
                mock,
                vfs,
                Arc::new(DashMap::new()),
            );
            let mut reg = BackendRegistry::new();
            reg.register(manager("first", exec.duplicate()));
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
            let (backend, recorded) = settle(&bare_jq(), registry(DryRunOutput::new().into(), found()))
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
        // one LiNix has ever heard of: the grammar refuses a prefix that names nothing,
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
        let scopes = curl.options.get("__scopes").unwrap();
        assert!(scopes.contains("module:dev"), "{}", scopes);
        assert!(scopes.contains("profile:Work"), "{}", scopes);
        // And `__source` stays the human answer to "where is this line?".
        assert!(curl.options["__source"].contains("dev.txt:1"));
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
        let scopes = jq.options.get("__scopes").unwrap();
        assert!(scopes.contains("module:base"), "{}", scopes);
        assert!(scopes.contains("profile:Work"), "{}", scopes);
    }

    #[tokio::test]
    async fn an_unreached_broken_module_is_never_parsed() {
        // II.3: LiNix only parses what the active profiles reach. `linix check` is the
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
