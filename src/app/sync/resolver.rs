use crate::backends::BackendRegistry;
use crate::app::vocab::Vocab;
use crate::config::grammar::{statement, Gates, GrammarError, Origin, Statement};
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
        }
    }

    /// Resolve the model against these already-resolved variables instead of running the
    /// provider (used by `apply` to reuse a saved plan's frozen variables).
    pub fn with_vars(mut self, vars: crate::model::vars::Vars) -> Self {
        self.vars_override = Some(vars);
        self
    }

    /// The `priority` file: which package managers this setup uses, and in what order.
    ///
    /// A missing file is an error and not a detected default. LiNix cannot pick your
    /// package managers for you — inheriting them from whatever happens to be installed is
    /// the thing `priority` exists to stop (V.15), and a default nobody chose is a default
    /// nobody can safely change (P5).
    pub async fn priority_for_host(&self) -> Result<Priority> {
        self.priority(&HostFacts::current()).await
    }

    async fn priority(&self, facts: &HostFacts) -> Result<Priority> {
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
        Priority::parse(&file, &body, facts).map_err(Error::from)
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
        let facts = HostFacts::current();
        let priority = self.priority(&facts).await?;
        let known = Vocab::new(&self.registry, self.config, &priority);
        crate::model::Resolver::new(&self.layout, &known, &priority)
            .with_facts(facts)
            .with_vars_source(self.config.vars.source.clone())
            .load_vars_with_origins()
            .map_err(Error::from)
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
        let priority = self.priority(&facts).await?;
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
            None => {
                let priority = self.priority(&facts).await?;
                let known = Vocab::new(&self.registry, self.config, &priority);
                crate::model::Resolver::new(&self.layout, &known, &priority)
                    .with_facts(facts.clone())
                    .with_vars_source(self.config.vars.source.clone())
                    .load_vars()?
            }
        };
        if !vars.is_empty() {
            debug!("{} variable(s) resolved", vars.len());
        }
        Ok(facts.with_vars(vars))
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
        let answers = self.probe_bare_names(&reached.statements, &priority).await?;

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

    /// Ask each backend in `priority` order whether it has this bare name (II.7 step 4).
    ///
    /// Each distinct name is asked once however many lines mention it: the answer is about
    /// the name and the machine, not about the line.
    async fn probe_bare_names(
        &self,
        statements: &[(Statement, Origin, Gates)],
        priority: &Priority,
    ) -> Result<HashMap<String, String>> {
        let mut questions: Vec<(String, Option<String>, Origin)> = Vec::new();
        for (stmt, origin, _) in statements {
            let Statement::Package(decl) = stmt else {
                continue;
            };
            if decl.backend.is_some() {
                continue;
            }
            let name = decl.selector.as_str().to_string();
            if questions.iter().any(|(n, _, _)| *n == name) {
                continue;
            }
            let constraint = decl.options.one("version").map(str::to_string);
            questions.push((name, constraint, origin.clone()));
        }

        let mut answers = HashMap::new();
        for (name, constraint, origin) in questions {
            let mut found = None;
            for backend in priority.order() {
                if self
                    .remote_package_exists(backend, &name, constraint.as_deref())
                    .await
                {
                    found = Some(backend.clone());
                    break;
                }
            }
            match found {
                Some(backend) => {
                    debug!("bare `{}` resolved to `{}`.", name, backend);
                    answers.insert(name, backend);
                }
                // No backend has it, so there is no honest answer to give. The old code
                // fell back to a default backend, which turned a typo into a request to
                // install a package that does not exist, reported by whichever backend
                // happened to be first (P3).
                None => {
                    return Err(Error::from(
                        GrammarError::new(
                            origin,
                            format!("no package manager in your `priority` list has `{}`.", name),
                        )
                        .with_hint(format!(
                            "tried {} in order. Check the spelling, or name the backend on the \
                             line if it comes from somewhere else.",
                            priority.order().join(", ")
                        )),
                    ))
                }
            }
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
        let facts = HostFacts::current();
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
                let answers = self.probe_bare_names(&stmts, &priority).await?;
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

    async fn remote_package_exists(
        &self,
        backend_name: &str,
        package_name: &str,
        constraint: Option<&str>,
    ) -> bool {
        let backend_cap = match self.registry.get(backend_name) {
            Some(b) if b.is_available() => b,
            _ => return false,
        };

        if let Some(searchable) = backend_cap.as_searchable() {
            if let Ok(true) = searchable.remote_has(package_name).await {
                if let Some(req) = constraint {
                    match searchable.remote_info(package_name).await {
                        Ok(Some(pkg)) => {
                            if let Some(ver) = pkg.version.as_deref() {
                                return self.satisfies_constraint(ver, req);
                            }
                        }
                        _ => return false,
                    }
                }
                return true;
            }

            // `remote_has` returning false is not proof of absence — a backend may not
            // implement it — so an inconclusive answer falls through to a real search.
            if let Ok(results) = searchable.search(package_name).await {
                return results.iter().any(|pkg| {
                    if pkg.name == package_name {
                        match constraint {
                            Some(req) => pkg
                                .version
                                .as_deref()
                                .is_some_and(|v| self.satisfies_constraint(v, req)),
                            None => true,
                        }
                    } else {
                        false
                    }
                });
            }
        }

        false
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
