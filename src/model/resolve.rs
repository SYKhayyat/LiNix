use super::conflict::{reconcile, Declared};
use super::dated::dating_of;
use super::layout::Layout;
use super::modules::{expand, expand_args, ModuleLoader};
use super::priority::Priority;
use super::profiles::{read_active, ProfileLoader, SetOp};
use crate::config::grammar::{
    statement, BackendNames, Gate, Gates, GrammarError, Options, Origin, PackageDecl, Result,
    Selector, Statement,
};
use crate::config::parser::HostFacts;
use crate::core::PackageSpec;
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The desired state (SPEC II.7 step 7).
///
/// **The seam.** Everything upstream produces this; everything downstream consumes it.
/// `src/backends/`, `src/core/` and `src/parsers/` — most of the codebase — never notice
/// the model changed underneath them.
#[derive(Debug, Clone, Default)]
pub struct DesiredState {
    /// Every declaration, by backend. `absent:` lines are in here too, carrying
    /// `present: false` — one map, because the map type is the seam.
    pub packages: HashMap<String, Vec<PackageSpec>>,
    /// Repositories, shims, links, services and schedules, in declaration order.
    pub extras: Vec<(Statement, Origin)>,
    /// Dated lines whose date has passed. They linger — **LiNix must not rewrite your
    /// files** — so `sync` mentions them, naming the exact file and line (II.16).
    pub lapsed: Vec<(String, Origin)>,
    /// The variables this state resolved against (Part IX). Carried so a saved plan can freeze
    /// them: a provider may read the clock or shell out, so re-resolving at apply time could
    /// disagree with what the plan previewed — the plan uses these, not a fresh resolution.
    pub vars: crate::model::vars::Vars,
}

impl DesiredState {
    /// What must be installed.
    pub fn present(&self) -> impl Iterator<Item = &PackageSpec> {
        self.packages.values().flatten().filter(|p| p.present)
    }

    /// What must not exist (II.2's `absent:`).
    pub fn absent(&self) -> impl Iterator<Item = &PackageSpec> {
        self.packages.values().flatten().filter(|p| !p.present)
    }

    pub fn total_present(&self) -> usize {
        self.present().count()
    }

    /// Each `re:` pattern in force and how many packages it expanded to (II.15).
    ///
    /// A pattern is the one line whose meaning you cannot read off the line — `apt:re:^lib`
    /// was measured at 30,207 packages — so the count is the only way to know what you wrote.
    pub fn regex_expansions(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for spec in self.packages.values().flatten() {
            if let Some(pattern) = spec.options.get("__from_regex") {
                *counts
                    .entry(format!("{}:re:{}", spec.backend, pattern))
                    .or_default() += 1;
            }
        }
        counts.into_iter().collect()
    }

    /// The extras `sync` applies AFTER packages (II.7's dependent phase): shims, services
    /// and links. A shim wraps a tool that must already be installed; a service enables a
    /// unit a package just laid down; a link writes a config a package expects — each one
    /// depends on the package plan having run, which is what makes them the *dependent*
    /// phase and not part of the package map. `repo:` is phase 1 (before packages) and
    /// `schedule:` belongs to the scheduler, so neither is here.
    pub fn dependents(&self) -> impl Iterator<Item = &(Statement, Origin)> {
        self.extras.iter().filter(|(s, _)| {
            matches!(
                s,
                Statement::Shim(..)
                    | Statement::Service(..)
                    | Statement::Link(..)
                    | Statement::Setting(..)
            )
        })
    }

    /// Whether there is any dependent extra to apply — so `sync` knows it has work even
    /// when the package plan is empty (a config that only declares a `service:` line).
    pub fn has_dependents(&self) -> bool {
        self.dependents().next().is_some()
    }

    /// The `schedule:` lines (S21), as `(name, options, origin)`. The scheduler provisions
    /// these onto systemd/launchd/Task Scheduler — a phase of its own, separate from the
    /// package plan and the dependents. The file-context rule in `collect` guarantees every
    /// one of these came from the `schedules` file.
    pub fn schedules(&self) -> impl Iterator<Item = (&str, &Options, &Origin)> {
        self.extras.iter().filter_map(|(s, o)| match s {
            Statement::Schedule(name, opts) => Some((name.as_str(), opts, o)),
            _ => None,
        })
    }

    /// The `exec:` lines whose `when` is true right now (XIII.3).
    ///
    /// Reaching this list at all *is* the first of XIII.3's three states: a line whose `when`
    /// is false was dropped during resolution, and the whole point of the three-state rule is
    /// that its absence means "nothing runs and nothing is undone", not "remove it". They are
    /// deliberately not `dependents()` — a verb has no teardown.
    pub fn execs(&self) -> impl Iterator<Item = (&str, &Options, &Origin)> {
        self.extras.iter().filter_map(|(s, o)| match s {
            Statement::Exec(script, opts) => Some((script.as_str(), opts, o)),
            _ => None,
        })
    }

    /// The `dotfiles:` trees this machine wants (7n). Like `execs()`, reaching this list means
    /// the `when` was true; unlike it, a tree IS a noun — its files are undone when it goes.
    pub fn dotfile_trees(&self) -> impl Iterator<Item = (&str, &Options, &Origin)> {
        self.extras.iter().filter_map(|(s, o)| match s {
            Statement::Dotfiles(path, opts) => Some((path.as_str(), opts, o)),
            _ => None,
        })
    }

    /// The `firewall:` lines this machine wants (Part XI).
    pub fn firewall_rules(&self) -> impl Iterator<Item = (&str, &Options, &Origin)> {
        self.extras.iter().filter_map(|(s, o)| match s {
            Statement::Firewall(name, opts) => Some((name.as_str(), opts, o)),
            _ => None,
        })
    }

    pub fn has_firewall_rules(&self) -> bool {
        self.firewall_rules().next().is_some()
    }

    pub fn has_dotfile_trees(&self) -> bool {
        self.dotfile_trees().next().is_some()
    }

    pub fn has_execs(&self) -> bool {
        self.execs().next().is_some()
    }

    /// Is there anything for a sync to do beyond the package plan?
    ///
    /// **One place, deliberately.** `sync`'s "nothing to do" exit used to enumerate the
    /// statement kinds it knew about, so every new kind was a chance to forget one — and it was
    /// forgotten three times: extras (S20), `exec:`, and `dotfiles:`, each shipping an early
    /// return that skipped the new phase entirely. Adding a statement kind now means adding it
    /// here, once, where the compiler and the reader both look.
    pub fn has_non_package_work(&self) -> bool {
        self.has_dependents()
            || self.schedules().next().is_some()
            || self.has_execs()
            || self.has_dotfile_trees()
            || self.has_firewall_rules()
    }
}

/// What the active profiles reach: the statements, and which profile and module each file's
/// lines belong to.
///
/// The scopes are collected here because this is the only place that knows them. Once the
/// statements are flattened, "profile `Work` reaches module `dev`" is gone, and `linix
/// upgrade --profile Work` has no way to ask.
pub struct Reached {
    pub statements: Vec<(Statement, Origin, Gates)>,
    scopes: HashMap<PathBuf, BTreeSet<String>>,
}

impl Reached {
    /// The scopes a line belongs to, as `module:dev` / `profile:Work`.
    fn of(&self, origin: &Origin) -> Vec<String> {
        self.scopes
            .get(&origin.file)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn record(&mut self, file: &Path, scope: String) {
        self.scopes.entry(file.to_path_buf()).or_default().insert(scope);
    }
}

/// Resolves `active` -> the desired state (SPEC II.7).
pub struct Resolver<'a> {
    layout: &'a Layout,
    backends: &'a dyn BackendNames,
    priority: &'a Priority,
    facts: HostFacts,
    now: DateTime<Utc>,
    bare: HashMap<String, String>,
    /// `[vars] source` from preferences (Part IX): which provider is active when the repo holds
    /// more than one. `None` selects the sole provider file, or none.
    vars_source: Option<String>,
}

impl<'a> Resolver<'a> {
    pub fn new(layout: &'a Layout, backends: &'a dyn BackendNames, priority: &'a Priority) -> Self {
        Self {
            layout,
            backends,
            priority,
            facts: HostFacts::current(),
            now: Utc::now(),
            bare: HashMap::new(),
            vars_source: None,
        }
    }

    /// The active `vars` provider (Part IX), from `[vars] source` in preferences.
    pub fn with_vars_source(mut self, source: Option<String>) -> Self {
        self.vars_source = source;
        self
    }

    /// The prober's answers: bare name -> the backend it resolved to.
    ///
    /// Supplying them before `collect` is what makes `ripgrep` and `apt:ripgrep` meet each
    /// other. Keyed on the resolved backend, they are one declaration and reconcile decides
    /// between them; keyed on `BARE`, they would be two, and both would be installed.
    pub fn with_bare(mut self, answers: HashMap<String, String>) -> Self {
        self.bare = answers;
        self
    }

    pub fn with_facts(mut self, facts: HostFacts) -> Self {
        self.facts = facts;
        self
    }

    /// Fixing "now" makes dated lines testable and makes one run internally consistent: a
    /// line must not lapse halfway through resolution.
    pub fn at(mut self, now: DateTime<Utc>) -> Self {
        self.now = now;
        self
    }

    /// II.7, steps 1-7.
    pub fn resolve(&self) -> Result<DesiredState> {
        let statements = self.statements()?;
        self.collect(statements)
    }

    /// Read and resolve the `vars` file (IX.2), so `$name` has a value before any `when` that
    /// mentions one is evaluated.
    ///
    /// Runs against facts carrying no variables, which is what makes a `when` *inside* `vars`
    /// a detected-fact condition only. Letting one variable's `when` test another would make
    /// the file's meaning depend on the order its blocks were read, and there is no order that
    /// is obviously right.
    pub fn load_vars(&self) -> Result<crate::model::vars::Vars> {
        self.load_vars_with_origins().map(|(v, _)| v)
    }

    /// [`load_vars`], plus where each variable was set — for the tooling that explains a variable
    /// (`linix vars`, `why`; W11/W12). The value path calls [`load_vars`] and drops the origins,
    /// so no producer runs twice: both share the one provider dispatch below.
    pub fn load_vars_with_origins(
        &self,
    ) -> Result<(crate::model::vars::Vars, crate::model::vars::VarOrigins)> {
        use crate::model::vars_provider::{self, Kind};
        let Some(selected) = vars_provider::select(self.layout.config_root(), &self.vars_source)?
        else {
            return Ok(Default::default());
        };
        match selected.kind {
            // The line file declares values and executes nothing, so it is not hashed.
            Kind::LineFile => self.load_vars_linefile(&selected.path),
            Kind::External => {
                self.verify_provider_approved(&selected.path)?;
                vars_provider::run_external_with_origins(&selected.path, &self.facts)
            }
            Kind::Embedded => {
                self.verify_provider_approved(&selected.path)?;
                crate::model::vars_embedded::resolve_with_origins(&selected.path, &self.facts)
            }
        }
    }

    /// A `vars` provider that executes goes through the hook ledger before it runs (V.55).
    /// It resolves at step 0 — before any plan, on `status`/`plan`, and under `watch --pull`
    /// on a pulled repo — so an unapproved or changed provider is a refusal here, exactly as
    /// a changed hook stops a sync. `-y` cannot approve; `linix lock` does.
    fn verify_provider_approved(&self, path: &Path) -> Result<()> {
        use crate::core::hook_lock::{hash_script, refusal, vars_id, HookLedger};
        let origin = Origin::new(
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("vars provider")
                .to_string(),
            0,
        );
        let body = std::fs::read_to_string(path).map_err(|e| {
            GrammarError::new(origin.clone(), format!("could not read the vars provider: {}", e))
        })?;
        let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();
        let id = vars_id(filename);
        let locks = self.layout.config_root().join("locks");
        let ledger = HookLedger::load(&HookLedger::path_in(&locks))
            .map_err(|e| GrammarError::new(origin.clone(), e.to_string()))?;
        let verdict = ledger.verdict(&id, &hash_script(&body));
        if verdict.is_approved() {
            return Ok(());
        }
        Err(GrammarError::new(
            origin,
            refusal(&id, "vars provider", &verdict),
        ))
    }

    /// The line-file provider (`vars`): parse it, enforce IX.3, resolve to typed values and their
    /// origins.
    fn load_vars_linefile(
        &self,
        file: &Path,
    ) -> Result<(crate::model::vars::Vars, crate::model::vars::VarOrigins)> {
        let Ok(body) = std::fs::read_to_string(file) else {
            return Ok(Default::default());
        };
        self.resolve_linefile_body(file, &body)
    }

    /// Resolve a line-file `vars` body that came from somewhere other than the working tree — the
    /// last commit, for W13's "what did this edit change" note. Same parse, same IX.3 check, same
    /// resolution as [`load_vars_linefile`]; only the source of the text differs.
    pub fn resolve_linefile_body(
        &self,
        file: &Path,
        body: &str,
    ) -> Result<(crate::model::vars::Vars, crate::model::vars::VarOrigins)> {
        let doc = crate::config::grammar::parse_document(file, body, self.backends)?;

        // IX.3 is a property of the FILE, not of this machine: a name defined only inside a
        // `when` block is an error everywhere, including on the box where that block happens
        // to match. Checked against every definition the file contains, gated or not, so the
        // answer does not depend on which machine runs it.
        let every = doc.every_statement();
        let top_level: std::collections::HashSet<&str> = every
            .iter()
            .filter(|(_, _, conditional)| !conditional)
            .filter_map(|(s, _, _)| match s {
                Statement::Var { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        for (stmt, origin, conditional) in &every {
            if let (Statement::Var { name, .. }, true) = (stmt, *conditional) {
                if !top_level.contains(name.as_str()) {
                    return Err(GrammarError::new(
                        (*origin).clone(),
                        format!("`{}` is only defined inside a `when` block", name),
                    )
                    .with_hint(
                        "give it a default at the top level. Every variable is defined on \
                         every machine, so a typo is always an error and never a block that \
                         quietly never fires.",
                    ));
                }
            }
        }

        let mut defs = Vec::new();
        for (stmt, origin, gates) in doc.statements_with_gating(&self.facts)? {
            match stmt {
                Statement::Var { name, value } => defs.push(crate::model::vars::Definition {
                    name,
                    value,
                    origin,
                    conditional: !gates.is_empty(),
                }),
                other => {
                    return Err(GrammarError::new(
                        origin,
                        format!("the `vars` file takes `NAME = VALUE` lines, not `{}`", set_key(&other)),
                    )
                    .with_hint("declarations live in `modules/`; this file only names values."));
                }
            }
        }
        crate::model::vars::resolve_with_origins(&defs)
    }

    /// II.7 steps 1-3: `active` -> profiles -> the modules they reach, parsed and gated.
    ///
    /// Split from `collect` because resolving a bare name needs the network and this does
    /// not. The caller probes the bare names these statements carry, then hands the answers
    /// back to `collect` via `with_bare` — so the merge in `collect` sees real backends.
    pub fn statements(&self) -> Result<Reached> {
        // 1. Read `active` -> the profile set. Against this run's facts, which carry the
        // resolved variables (IX.6/W8), so `when $role == travel { Travel }` in `active` sees
        // `$role` — the single most useful place for a variable, and the one that used to fail
        // with "unknown when key" because `active` re-detected varless facts of its own.
        let active_file = self.layout.active_file();
        let body = std::fs::read_to_string(&active_file).unwrap_or_default();
        let active: Vec<(String, Gates)> = read_active(&active_file, &body, &self.facts)?
            .into_iter()
            .filter(|e| e.on)
            .map(|e| {
                let gates = e
                    .gate
                    .map(|pred| vec![Gate::new(pred, Origin::new(&active_file, e.line))])
                    .unwrap_or_default();
                (e.name, gates)
            })
            .collect();

        let mut out = Reached {
            statements: Vec::new(),
            scopes: HashMap::new(),
        };

        // 2. Resolve profiles -> the module set. Profiles may reference profiles; modules
        //    may not.
        let profiles = ProfileLoader::new(self.layout, self.backends);
        let mut wanted_modules: Vec<(String, Gates, Vec<(String, String)>)> = Vec::new();
        // Which profiles want each module. A module two profiles both reach belongs to
        // both, and `upgrade --profile` for either must find it.
        let mut wanted_by: HashMap<String, Vec<String>> = HashMap::new();
        let mut direct: Vec<(Statement, Origin, Gates)> = Vec::new();
        let mut loader = ModuleLoader::new(self.layout, self.backends);
        let asked = Origin::new(&active_file, 0);

        for (name, activated_by) in &active {
            let r = profiles.resolve(name, &asked, &self.facts, &mut Vec::new(), activated_by)?;

            // A profile doing set math resolves to packages, not to modules: an
            // intersection of two modules' packages is not a module (V.46). So it is
            // materialised here and its result joins `direct`.
            if r.does_set_math() {
                let stmts = self.apply_set_math(&profiles, &mut loader, &r, &asked)?;
                for (_, origin, _) in &stmts {
                    out.record(&origin.file, format!("profile:{}", name));
                    self.record_module_scope(&mut out, origin);
                }
                direct.extend(stmts);
                continue;
            }

            for m in r.modules {
                match wanted_modules.iter_mut().find(|(n, ..)| *n == m.name) {
                    Some((_, gates, _)) if m.gates.len() < gates.len() => *gates = m.gates.clone(),
                    Some(_) => {}
                    None => wanted_modules.push((m.name.clone(), m.gates.clone(), m.args.clone())),
                }
                wanted_by.entry(m.name).or_default().push(name.clone());
            }
            // A profile's own package lines belong to the profile and to no module: a
            // module can never reach them, which is the cost II.4 accepts knowingly (V.3).
            for (_, origin, _) in &r.direct {
                out.record(&origin.file, format!("profile:{}", name));
            }
            direct.extend(r.direct);
        }

        // 3. Parse ONLY the modules reached. Apply `when`.
        for (m, reached_by, args) in &wanted_modules {
            let stmts = expand_args(
                &mut loader,
                m,
                &Origin::new(&active_file, 0),
                &self.facts,
                &mut Vec::new(),
                reached_by,
                args,
            )?;
            // Attributed by the file each line actually came from, so a module reached
            // through another module is scoped to itself and to the profile that led here.
            for (_, origin, _) in &stmts {
                self.record_module_scope(&mut out, origin);
                for p in wanted_by.get(m).into_iter().flatten() {
                    out.record(&origin.file, format!("profile:{}", p));
                }
            }
            out.statements.extend(stmts);
        }
        out.statements.extend(direct);

        // 3b (S21). Read the `schedules` file — the one place `schedule:` lines live (II.2).
        // Parsed and `when`-gated exactly like a module, so `when host == laptop { … }` works
        // here too. Absent file → no schedules, which is the norm. The file-context rule in
        // `collect` refuses a `schedule:` from anywhere else, so this is its only legal source.
        let schedules_file = self.layout.schedules_file();
        if let Ok(body) = std::fs::read_to_string(&schedules_file) {
            let doc = crate::config::grammar::parse_document(&schedules_file, &body, self.backends)?;
            out.statements.extend(doc.statements_with_gating(&self.facts)?);
        }

        self.expand_vars(&mut out.statements)?;
        Ok(out)
    }

    /// Parse every module and profile the folders hold, reached or not, and return every error
    /// found rather than the first.
    ///
    /// II.3: resolution parses only what the active profiles reach, and `check` parses
    /// everything. Without this, a module with a broken line is clean until the day someone
    /// activates the profile that reaches it — which is the day they are least able to read a
    /// parse error. Reached files are parsed again here rather than tracked and skipped: the
    /// bookkeeping to tell them apart is a second answer to "what did resolution read", and the
    /// files are small.
    ///
    /// Each one is walked as far as resolution would walk it — `use` followed into other
    /// modules and profiles — because **`check` catches cycles no active profile reaches**
    /// (II.7), and a loop is not visible in any one file.
    ///
    /// Gated by this host's facts, like everything else: a `when` arm for another machine is
    /// parsed here but not walked, and checking it would mean deciding which host to pretend
    /// to be.
    pub fn parse_everything(&self) -> Vec<GrammarError> {
        let mut errors = Vec::new();
        let mut loader = ModuleLoader::new(self.layout, self.backends);
        let asked = Origin::new(self.layout.modules_dir(), 0);

        let mut modules = loader.available();
        modules.sort();
        for name in modules {
            if let Err(e) = expand(
                &mut loader,
                &name,
                &asked,
                &self.facts,
                &mut Vec::new(),
                &Vec::new(),
            ) {
                errors.push(e);
            }
        }

        let profiles = ProfileLoader::new(self.layout, self.backends);
        let asked = Origin::new(self.layout.profiles_dir(), 0);
        for name in profiles.available() {
            if let Err(e) = profiles.resolve(
                &name,
                &asked,
                &self.facts,
                &mut Vec::new(),
                &Vec::new(),
            ) {
                errors.push(e);
            }
        }

        // Every module is walked as a root, so a loop is found once per member and reported
        // once per member — five copies of one five-module loop. Two reports of one loop are
        // rotations of each other: same hops, different starting point, so the hops are the
        // key and the arrow naming that starting point is not.
        let mut seen: Vec<Vec<&str>> = Vec::new();
        let mut once = Vec::new();
        for e in &errors {
            let mut key: Vec<&str> = e
                .what
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("^ back to"))
                .collect();
            key.sort_unstable();
            if !seen.contains(&key) {
                seen.push(key);
                once.push(e.clone());
            }
        }
        once
    }

    /// Substitute `$name` into the values of every statement this host reached.
    ///
    /// Done here, once, after `when` gating and before anything reads a value — so the prober,
    /// the merge and the backends all see the same expanded text and none of them has to know
    /// variables exist. A line the host never reached is never expanded, which is why an unused
    /// `when` arm cannot fail on a variable that is irrelevant to this machine.
    /// An empty variable set is not a reason to skip the walk: with no `vars` file at all,
    /// `$role` has to be the same error it is when the file exists and the name is misspelled.
    /// Returning early here left it as literal text, which becomes a path with a dollar in it
    /// and fails later, somewhere else, with no mention of the typo.
    fn expand_vars(&self, statements: &mut [(Statement, Origin, Gates)]) -> Result<()> {
        for (stmt, origin, _) in statements.iter_mut() {
            let vars = &self.facts.vars;
            match stmt {
                Statement::Package(d) | Statement::Absent(d) => {
                    for value in d.options.values_mut() {
                        *value = crate::model::vars::expand(value, vars, origin)?;
                    }
                }
                Statement::Shim(name, opts)
                | Statement::Service(name, opts)
                | Statement::Link(name, opts)
                | Statement::Setting(name, opts)
                | Statement::Exec(name, opts)
                | Statement::Dotfiles(name, opts)
                | Statement::Firewall(name, opts) => {
                    *name = crate::model::vars::expand(name, vars, origin)?;
                    for value in opts.values_mut() {
                        *value = crate::model::vars::expand(value, vars, origin)?;
                    }
                }
                Statement::Repo { spec, .. } => {
                    *spec = crate::model::vars::expand(spec, vars, origin)?;
                }
                // A schedule's `run` is a command line, where `$` belongs to the shell that
                // will run it. Set math and `use` name files, which are not values.
                Statement::Schedule(..)
                | Statement::Use(..)
                | Statement::Param { .. }
                | Statement::Exclude(_)
                | Statement::Intersect(_)
                | Statement::Subtract(_)
                | Statement::Expr(_)
                | Statement::Var { .. } => {}
            }
        }
        Ok(())
    }

    /// A line keeps its file, so a package that survives an intersection still knows which
    /// module holds it. That is what makes `upgrade --module` keep working through set math.
    fn record_module_scope(&self, out: &mut Reached, origin: &Origin) {
        if origin.file.parent() != Some(self.layout.modules_dir().as_path()) {
            return;
        }
        if let Some(stem) = origin.file.file_stem().and_then(|s| s.to_str()) {
            out.record(&origin.file, format!("module:{}", stem));
        }
    }

    /// Apply a profile's set math to what it reaches (II.4).
    ///
    /// Order is fixed and stated in II.4: everything is gathered first, then narrowed by
    /// each `intersect`, then everything subtracted is removed. **Subtraction always wins** —
    /// otherwise `use gaming` after `-steam` would quietly put steam back, and which line
    /// won would depend on the order you happened to write them in.
    fn apply_set_math(
        &self,
        profiles: &ProfileLoader<'_>,
        loader: &mut ModuleLoader<'a>,
        r: &super::profiles::Resolved,
        asked: &Origin,
    ) -> Result<Vec<(Statement, Origin, Gates)>> {
        let mut base: Vec<(Statement, Origin, Gates)> = Vec::new();
        for m in &r.modules {
            base.extend(expand_args(
                loader,
                &m.name,
                asked,
                &self.facts,
                &mut Vec::new(),
                &m.gates,
                &m.args,
            )?);
        }
        base.extend(r.direct.clone());

        let mut intersects: Vec<Vec<String>> = Vec::new();
        let mut subtract: Vec<String> = Vec::new();

        for (op, origin) in &r.ops {
            match op {
                SetOp::Expr(e) => {
                    let found = self.eval_expression(profiles, loader, e, origin)?;
                    base.extend(found);
                }
                SetOp::Intersect(reference) => {
                    let other = self.atom(profiles, loader, reference.name(), origin)?;
                    intersects.push(other.iter().map(|(s, ..)| set_key(s)).collect());
                }
                SetOp::Exclude(reference) => {
                    let other = self.atom(profiles, loader, reference.name(), origin)?;
                    subtract.extend(other.iter().map(|(s, ..)| set_key(s)));
                }
                SetOp::Subtract(pkg) => subtract.push(pkg.trim().to_string()),
            }
        }

        for keep in &intersects {
            base.retain(|(s, ..)| keep.iter().any(|k| same_package(k, &set_key(s))));
        }
        base.retain(|(s, ..)| !subtract.iter().any(|k| same_package(k, &set_key(s))));

        Ok(base)
    }

    /// Evaluate `(Work | gaming) & security` and return the statements that survive.
    ///
    /// `profile_expr` works over names, so each atom is resolved to its packages' keys, the
    /// expression is evaluated over those, and the winners are mapped back to the statements
    /// they came from — which is what keeps their file, and therefore their module.
    fn eval_expression(
        &self,
        profiles: &ProfileLoader<'_>,
        loader: &mut ModuleLoader<'a>,
        expr: &str,
        origin: &Origin,
    ) -> Result<Vec<(Statement, Origin, Gates)>> {
        let mut table: HashMap<String, (Statement, Origin, Gates)> = HashMap::new();
        let mut failure: Option<GrammarError> = None;

        let keys = crate::app::profile_expr::evaluate(expr, &mut |atom| {
            match self.atom(profiles, loader, atom, origin) {
                Ok(stmts) => stmts
                    .into_iter()
                    .map(|(s, o, g)| {
                        let k = set_key(&s);
                        table.entry(k.clone()).or_insert((s, o, g));
                        k
                    })
                    .collect(),
                Err(e) => {
                    failure.get_or_insert(e);
                    Vec::new()
                }
            }
        })
        .map_err(|e| {
            GrammarError::new(origin.clone(), format!("`{}` is not a set expression: {}", expr, e))
                .with_hint(
                    "set math is `|` union, `&` intersect, `\\` difference, and parentheses \
                     — for example `(Work | gaming) & security`.",
                )
        })?;

        if let Some(e) = failure {
            return Err(e);
        }
        Ok(keys.into_iter().filter_map(|k| table.remove(&k)).collect())
    }

    /// One name in an expression: a profile, a module, or a package written literally.
    fn atom(
        &self,
        profiles: &ProfileLoader<'_>,
        loader: &mut ModuleLoader<'a>,
        atom: &str,
        origin: &Origin,
    ) -> Result<Vec<(Statement, Origin, Gates)>> {
        let capitalized = atom.chars().next().is_some_and(char::is_uppercase);

        if capitalized {
            let r = profiles.resolve(atom, origin, &self.facts, &mut Vec::new(), &Vec::new())?;
            return self.apply_set_math(profiles, loader, &r, origin);
        }

        // A name that cannot be a module is not a broken module — it falls through to the
        // package parse below, which is where `(Work | apt:jq)` is meant to land.
        let is_module = super::ModuleName::new(atom)
            .map(|m| self.layout.module_file(&m).is_file())
            .unwrap_or(false);
        if is_module {
            return expand(
                loader,
                atom,
                origin,
                &self.facts,
                &mut Vec::new(),
                &Vec::new(),
            );
        }

        // Not a file, so it is a package named literally: `(Work | jq)`. Parsed, so a typo
        // is an error here rather than a package nobody has.
        let stmt = statement::parse(origin, atom, self.backends)?;
        Ok(vec![(stmt, origin.clone(), Vec::new())])
    }

    /// II.7 steps 4-7: resolve each line, conflicts are errors, dated lines get rule 6.
    pub fn collect(&self, reached: Reached) -> Result<DesiredState> {
        // Keyed `backend:name` so two declarations of one package meet each other. BTreeMap
        // so the plan and any error list in a stable order rather than a hash one.
        let mut merged: BTreeMap<String, Entry> = BTreeMap::new();
        let mut out = DesiredState::default();

        for (stmt, origin, gates) in reached.statements.iter().cloned() {
            let (decl, present) = match stmt {
                Statement::Package(d) => (d, true),
                Statement::Absent(d) => (d, false),
                // V.47/V.15: a `repo:` names a backend, and a backend not in `priority` is
                // one LiNix does not use — refused here so `sync`, `plan` and `check` all
                // say the same thing, in the file, rather than at the add command.
                Statement::Repo { backend, spec } if !self.priority.allows(&backend) => {
                    return Err(self.priority.reject(&backend, &origin).with_hint(format!(
                        "add `{}` to `priority` to use `repo:{}:{}`. Not listed means LiNix \
                         does not use it at all.",
                        backend, backend, spec
                    )));
                }
                // II.2 / S21: a `schedule:` line belongs in the `schedules` file, and only
                // there — one in a module or profile is an error, not silently parsed. A
                // schedule is a machine-wide fact ("run `clean` nightly"), not something a
                // profile toggles, so letting it hide in a module would make "what does this
                // machine run on a timer?" depend on what you activated.
                Statement::Schedule(ref name, _) if origin.file != self.layout.schedules_file() => {
                    return Err(GrammarError::new(
                        origin.clone(),
                        format!("`schedule:{}` is not in the `schedules` file", name),
                    )
                    .with_hint(
                        "move it to the `schedules` file in your config root. A schedule runs \
                         for the whole machine, so it does not live in a module or profile.",
                    ));
                }
                // IX.2: a variable belongs in the `vars` file, and only there. A `NAME =
                // VALUE` in a module would make what `$role` means depend on which profile
                // you activated, which is the opposite of a variable that is always defined.
                Statement::Var { ref name, .. } if origin.file != self.layout.vars_file() => {
                    return Err(GrammarError::new(
                        origin.clone(),
                        format!("`{}` is a variable, and is not in the `vars` file", name),
                    )
                    .with_hint(
                        "move it to the `vars` file in your config root. A variable is defined \
                         on every machine, so it cannot live behind a profile.",
                    ));
                }
                // A `param` is a module's own declaration and is consumed when the module is
                // `use`d (U32). Reaching resolution means it was written where nothing binds it —
                // a profile, the `active` file — where it would silently bind nothing.
                Statement::Param { ref name, .. } => {
                    return Err(GrammarError::new(
                        origin.clone(),
                        format!("`param {}` is only valid in a module", name),
                    )
                    .with_hint(
                        "a parameter is declared in the module it belongs to and bound by \
                         `use module(name=value)`. A profile has nothing to bind it.",
                    ));
                }
                other => {
                    out.extras.push((other, origin));
                    continue;
                }
            };

            let backend = self.backend_for(&decl, &origin)?;
            let key = format!("{}:{}", backend, decl.selector.as_str());

            if dating_of(&decl.options, self.now) == super::dated::Dating::Lapsed {
                out.lapsed.push((key.clone(), origin.clone()));
            }

            let incoming = Declared {
                options: decl.options.clone(),
                origin: origin.clone(),
                present,
            };

            match merged.remove(&key) {
                Some(mut e) => {
                    // Reconcile decides which declaration wins, but BOTH still declared it:
                    // the loser's scope is not thereby untrue, and dropping it would hide
                    // the package from `upgrade --module <the other one>`.
                    e.declared = reconcile(&key, e.declared, incoming, self.now)?;
                    e.origins.push(origin);
                    // Declared twice, once behind a condition and once not, it is here
                    // unconditionally — so the shortest chain is the true reason, whichever
                    // declaration won on options.
                    if gates.len() < e.gates.len() {
                        e.gates = gates;
                    }
                    merged.insert(key, e);
                }
                None => {
                    merged.insert(
                        key,
                        Entry {
                            declared: incoming,
                            backend,
                            selector: decl.selector.clone(),
                            origins: vec![origin],
                            gates,
                        },
                    );
                }
            }
        }

        for (_, e) in merged {
            // A line whose date has passed has no opinion at all — it is not "absent", it
            // simply stops counting (II.7 rule 6).
            if dating_of(&e.declared.options, self.now) == super::dated::Dating::Lapsed {
                continue;
            }
            let mut scopes: Vec<String> = Vec::new();
            for o in &e.origins {
                for s in reached.of(o) {
                    if !scopes.contains(&s) {
                        scopes.push(s);
                    }
                }
            }
            // A line with no prefix could not be checked at parse time, because the backend
            // that will answer it was not known yet. It is known now, and an option nobody
            // reads is a line that does nothing (VIII.4).
            crate::config::grammar::statement::validate_artifact_options(
                &e.declared.origin,
                Some(e.backend.as_str()),
                &e.declared.options,
            )?;

            let spec = to_spec(
                &e.backend,
                &e.selector,
                &e.declared.options,
                e.declared.present,
                self.priority.options(&e.backend),
                Provenance {
                    origin: &e.declared.origin,
                    scopes: &scopes,
                    gates: &e.gates,
                },
            );
            out.packages.entry(e.backend).or_default().push(spec);
        }

        out.vars = self.facts.vars.clone();
        Ok(out)
    }

    /// II.7 step 4: bare names use `priority`, then the lock.
    ///
    /// The probe itself (asking each backend "do you have ripgrep?") is the caller's job —
    /// it needs the live registry and the network. This resolves what the FILE says; an
    /// unlocked bare name is handed to the prober with the order to try.
    fn backend_for(&self, decl: &PackageDecl, origin: &Origin) -> Result<String> {
        match &decl.backend {
            Some(b) => {
                // V.15: not listed means LiNix does not use it at all, and saying so
                // catches typos and makes your backend set declared, not inherited.
                if !self.priority.allows(b) {
                    return Err(self.priority.reject(b, origin));
                }
                Ok(b.clone())
            }
            None => {
                if self.priority.is_empty() {
                    return Err(GrammarError::new(
                        origin.clone(),
                        format!("`{}` does not say which backend, and `priority` is empty", decl.selector.as_str()),
                    )
                    .with_hint(
                        "list your package managers in `priority`, or write the backend on the \
                         line: `apt:curl`.",
                    ));
                }
                // The bare name is the question, the lock is the answer (V.16). If the
                // prober has already answered, that answer is the backend; if not, the
                // question is passed on marked, so nothing downstream mistakes it for a
                // decision.
                match self.bare.get(decl.selector.as_str()) {
                    Some(b) => Ok(b.clone()),
                    None => Ok(BARE.to_string()),
                }
            }
        }
    }
}

/// The backend of a bare name, before probing. Never reaches a backend: `Resolver::resolve`
/// hands these to the prober, which replaces it with the lock's answer or `priority`'s.
pub const BARE: &str = "?";

/// How a statement is named in set math: as written, `apt:jq` or bare `jq`.
///
/// The statement's own identity ([`Statement::key`]) — set math has no separate notion of what
/// names a line, and when it kept one the two drifted apart every time a kind was added.
fn set_key(stmt: &Statement) -> String {
    stmt.key()
}

/// Whether two set-math keys name the same package.
///
/// A bare name matches any backend's line for it, so `-vim` takes vim out however it was
/// going to arrive. Two explicit backends must agree: `-apt:vim` leaves `cargo:vim` alone,
/// because you said which one.
fn same_package(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    fn name_of(s: &str) -> &str {
        s.split_once(':').map(|(_, n)| n).unwrap_or(s)
    }
    fn bare(s: &str) -> bool {
        !s.contains(':')
    }
    match (bare(a), bare(b)) {
        (true, false) => a == name_of(b),
        (false, true) => name_of(a) == b,
        _ => false,
    }
}

/// One package's declarations, mid-merge.
struct Entry {
    declared: Declared,
    backend: String,
    selector: Selector,
    /// Every line that declared it, winner and losers alike — the scopes it belongs to.
    origins: Vec<Origin>,
    /// The shortest chain of `when` conditions any of those lines needed.
    gates: Gates,
}

/// Where a declaration came from and what had to be true for it to count.
///
/// Three answers to three different questions, which is why they travel together and not as
/// one tag: `origin` is the line a human edits, `scopes` is what `--module`/`--profile` match
/// on, and `gates` is what `why` explains. One tag answering two of them is how
/// `upgrade --module dev` came to be matched against a filename.
pub struct Provenance<'a> {
    pub origin: &'a Origin,
    pub scopes: &'a [String],
    pub gates: &'a [Gate],
}

/// Build the seam's `PackageSpec` from one declaration. The only place this conversion
/// happens: an imperative `linix install jq` and a line in a module must produce the same
/// spec, or the two paths drift (P4).
pub fn to_spec(
    backend: &str,
    selector: &Selector,
    options: &Options,
    present: bool,
    backend_defaults: Option<&Options>,
    from: Provenance<'_>,
) -> PackageSpec {
    let mut properties: HashMap<String, String> = HashMap::new();

    // The line beats `priority`, and `priority` beats the built-in default (VIII.2, D9).
    // The backend's defaults go in first and the line's own options overwrite them whole:
    // a list is replaced, never extended, because half-overriding an ordered list produces
    // an order nobody wrote.
    if let Some(defaults) = backend_defaults {
        for (k, vs) in defaults.iter() {
            properties.insert(k.to_string(), vs.join(";"));
        }
    }
    for (k, vs) in options.iter() {
        // `requires` is a list; the rest are single values. Joined with `;` because that is
        // what the planner already splits on.
        properties.insert(k.to_string(), vs.join(";"));
    }

    // Which of the three levels answered, so `why` can say (D14). Kept beside the value
    // rather than recomputed later: the resolver is the only place that still knows.
    if options.contains("formats") {
        properties.insert("__formats_from".into(), "line".into());
    } else if backend_defaults.is_some_and(|d| d.contains("formats")) {
        properties.insert("__formats_from".into(), format!("priority ({})", backend));
    }
    // Two different questions, two tags. `__source` is where the line is, for the human
    // reading an error or "Added jq to modules/imperative.txt" (II.8). `__scopes` is what
    // it belongs to, for `--module` / `--profile` to match on. One tag answering both is
    // how `upgrade --module dev` came to be matched against a filename.
    properties.insert("__source".to_string(), from.origin.to_string());
    // The `when` conditions that admitted this line, kept only where one tests a variable:
    // that is the hop `why` cannot make on its own, because a variable's value is not in
    // the file the package is written in (W11).
    let gated_by: Vec<String> = from
        .gates
        .iter()
        .filter(|g| !crate::model::vars::referenced_names(&g.predicate).is_empty())
        .map(Gate::to_string)
        .collect();
    if !gated_by.is_empty() {
        properties.insert("__gated_by".to_string(), gated_by.join(";"));
    }
    if !from.scopes.is_empty() {
        properties.insert("__scopes".to_string(), from.scopes.join(";"));
    }
    if let Selector::Regex(p) = selector {
        properties.insert("__regex".to_string(), p.clone());
    }
    PackageSpec {
        name: selector.as_str().to_string(),
        backend: backend.to_string(),
        requires: options.all("requires").to_vec(),
        options: properties,
        present,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dated::parse_absolute;
    use tempfile::TempDir;

    fn known(name: &str) -> bool {
        matches!(name, "apt" | "cargo" | "snap")
    }

    fn facts() -> HostFacts {
        HostFacts {
            os: "linux".into(),
            arch: "x86_64".into(),
            host: "laptop".into(),
            family: "debian".into(),
            vars: Default::default(),
        }
    }

    struct Fx {
        _tmp: TempDir,
        layout: Layout,
        priority: Priority,
    }

    fn fx(active: &str, profiles: &[(&str, &str)], modules: &[(&str, &str)]) -> Fx {
        let tmp = TempDir::new().unwrap();
        let layout = Layout::new(tmp.path().join("cfg"), tmp.path().join("data"));
        std::fs::create_dir_all(layout.profiles_dir()).unwrap();
        std::fs::create_dir_all(layout.modules_dir()).unwrap();
        std::fs::write(layout.active_file(), active).unwrap();
        for (n, b) in profiles {
            std::fs::write(layout.profiles_dir().join(n), b).unwrap();
        }
        for (n, b) in modules {
            std::fs::write(layout.modules_dir().join(n), b).unwrap();
        }
        Fx {
            _tmp: tmp,
            layout,
            priority: Priority::from_backends(vec!["apt".into(), "cargo".into()]),
        }
    }

    fn resolve(f: &Fx) -> Result<DesiredState> {
        Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts())
            .at(parse_absolute("2026-07-16T12:00").unwrap())
            .resolve()
    }

    fn names(d: &DesiredState, backend: &str) -> Vec<String> {
        let mut v: Vec<String> = d
            .present()
            .filter(|p| p.backend == backend)
            .map(|p| p.name.clone())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn active_profiles_choose_which_modules_are_read() {
        let f = fx(
            "Work\n",
            &[("Work", "use editors\n")],
            &[
                ("editors.txt", "apt:neovim\n"),
                ("gaming.txt", "apt:steam\n"),
            ],
        );
        let d = resolve(&f).unwrap();
        assert_eq!(names(&d, "apt"), ["neovim"]);
    }

    #[test]
    fn nothing_is_active_unless_a_profile_names_it() {
        // The whole reason `group:` was already a no-op (V.4): the old resolver seeded
        // every .txt unconditionally, so a file was loaded before you named it.
        let f = fx("\n", &[], &[("editors.txt", "apt:neovim\n")]);
        let d = resolve(&f).unwrap();
        assert_eq!(d.total_present(), 0);
    }

    #[test]
    fn an_unreached_broken_module_is_never_parsed() {
        // II.3: LiNix only parses what the active profiles reach.
        let f = fx(
            "Work\n",
            &[("Work", "use editors\n")],
            &[
                ("editors.txt", "apt:neovim\n"),
                ("broken.txt", "!!! not a statement !!!\n"),
            ],
        );
        assert!(resolve(&f).is_ok());
    }

    #[test]
    fn two_modules_declaring_one_package_differently_is_an_error_naming_both() {
        // Part IV requires this exact proof.
        let f = fx(
            "Work\n",
            &[("Work", "use a\nuse b\n")],
            &[
                ("a.txt", "apt:jq@version=1.6\n"),
                ("b.txt", "apt:jq@version=1.7\n"),
            ],
        );
        let err = resolve(&f).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("a.txt"), "{}", msg);
        assert!(msg.contains("b.txt"), "{}", msg);
    }

    #[test]
    fn absent_lands_in_its_own_bucket() {
        let f = fx(
            "Work\n",
            &[("Work", "use base\n")],
            &[("base.txt", "apt:curl\nabsent:apt:libreoffice\n")],
        );
        let d = resolve(&f).unwrap();
        assert_eq!(names(&d, "apt"), ["curl"]);
        let absent: Vec<&str> = d.absent().map(|p| p.name.as_str()).collect();
        assert_eq!(absent, ["libreoffice"]);
    }

    #[test]
    fn an_explicit_backend_not_in_priority_is_refused() {
        // V.15, and the message II.6 specifies.
        let f = fx(
            "Work\n",
            &[("Work", "use base\n")],
            &[("base.txt", "snap:foo\n")],
        );
        let err = resolve(&f).unwrap_err();
        assert!(err.what.contains("isn't in your priority list"), "{}", err);
    }

    #[test]
    fn a_lapsed_line_stops_counting_but_is_reported_with_its_file_and_line() {
        // II.16: expired lines linger — LiNix must not rewrite your files — so it mentions
        // them, naming the exact file and line, never vaguely.
        let f = fx(
            "Work\n",
            &[("Work", "use base\n")],
            &[("base.txt", "apt:curl\napt:jq@expires=2026-01-01\n")],
        );
        let d = resolve(&f).unwrap();
        assert_eq!(names(&d, "apt"), ["curl"], "the lapsed jq stops counting");
        assert_eq!(d.lapsed.len(), 1);
        assert_eq!(d.lapsed[0].0, "apt:jq");
        assert!(d.lapsed[0].1.to_string().contains("base.txt:2"));
    }

    #[test]
    fn a_profile_can_hold_a_package_directly() {
        // II.4/V.3.
        let f = fx("Work\n", &[("Work", "apt:slack\n")], &[]);
        assert_eq!(names(&resolve(&f).unwrap(), "apt"), ["slack"]);
    }

    #[test]
    fn a_repo_carries_its_backend() {
        // V.47: a repo names the package manager that owns it.
        let f = fx(
            "Work\n",
            &[("Work", "use py\n")],
            &[("py.txt", "repo:apt:ppa:deadsnakes/ppa\napt:python3.12\n")],
        );
        let d = resolve(&f).unwrap();
        let repo = d.extras.iter().find_map(|(s, _)| match s {
            Statement::Repo { backend, spec } => Some((backend.clone(), spec.clone())),
            _ => None,
        });
        assert_eq!(repo, Some(("apt".into(), "ppa:deadsnakes/ppa".into())));
    }

    #[test]
    fn a_repo_for_a_backend_not_in_priority_is_refused() {
        // V.47/V.15. `snap` is not in this fixture's priority (apt, cargo).
        let f = fx(
            "Work\n",
            &[("Work", "use py\n")],
            &[("py.txt", "repo:snap:whatever\n")],
        );
        let err = resolve(&f).unwrap_err();
        assert!(err.what.contains("isn't in your priority list"), "{}", err);
    }

    #[test]
    fn extras_are_carried_in_declaration_order() {
        let f = fx(
            "Work\n",
            &[("Work", "use py\n")],
            &[("py.txt", "repo:apt:ppa:deadsnakes/ppa\napt:python3.12\n")],
        );
        let d = resolve(&f).unwrap();
        assert!(matches!(d.extras[0].0, Statement::Repo { .. }));
        assert_eq!(names(&d, "apt"), ["python3.12"]);
    }

    #[test]
    fn dependents_are_the_after_package_extras_only() {
        // II.7 phase 3: shims, services and links are the dependent phase. `repo:` is phase
        // 1, so it is excluded. (`schedule:` can no longer appear in a module at all — see
        // `a_schedule_outside_the_schedules_file_is_refused` — so it is not part of this
        // fixture.)
        let f = fx(
            "Work\n",
            &[("Work", "use svc\n")],
            &[(
                "svc.txt",
                "repo:apt:ppa:deadsnakes/ppa\n\
                 apt:nginx\n\
                 service:nginx@enabled=true\n\
                 link:~/.config/nginx.conf@target=~/.config/nginx.conf\n\
                 shim:rg\n",
            )],
        );
        let d = resolve(&f).unwrap();
        assert!(d.has_dependents());
        let kinds: Vec<&str> = d
            .dependents()
            .map(|(s, _)| match s {
                Statement::Service(..) => "service",
                Statement::Link(..) => "link",
                Statement::Shim(..) => "shim",
                _ => "other",
            })
            .collect();
        // Declaration order preserved; repo excluded.
        assert_eq!(kinds, ["service", "link", "shim"]);
    }

    #[test]
    fn a_schedule_outside_the_schedules_file_is_refused() {
        // S21: a `schedule:` line belongs in the `schedules` file, and only there. One in a
        // module is an error naming the file, not a silently-parsed extra.
        let f = fx(
            "Work\n",
            &[("Work", "use m\n")],
            &[("m.txt", "schedule:nightly@cron=0 2 * * *,run=clean\n")],
        );
        let err = resolve(&f).unwrap_err();
        assert!(
            err.what.contains("is not in the `schedules` file"),
            "{}",
            err
        );
    }

    #[test]
    fn a_schedule_in_the_schedules_file_is_read_and_is_not_a_dependent() {
        // S21 parts (1) and (3): the resolver reads the `schedules` file, and its lines are
        // schedules — not dependents, not packages.
        let f = fx("Work\n", &[("Work", "apt:nginx\n")], &[]);
        std::fs::write(
            f.layout.schedules_file(),
            "schedule:nightly@cron=0 2 * * *,run=clean\n",
        )
        .unwrap();
        let d = resolve(&f).unwrap();
        assert_eq!(d.schedules().count(), 1);
        let (name, _opts, _origin) = d.schedules().next().unwrap();
        assert_eq!(name, "nightly");
        // A schedule is never a dependent (II.7 phase 4, not phase 3).
        assert!(!d.has_dependents());
    }

    // ---- Part IX: vars ----

    fn load_vars(f: &Fx) -> Result<crate::model::vars::Vars> {
        Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts())
            .load_vars()
    }

    fn with_vars(f: &Fx, body: &str) {
        std::fs::write(f.layout.vars_file(), body).unwrap();
    }

    #[test]
    fn no_vars_file_is_no_variables_and_not_an_error() {
        let f = fx("Work
", &[("Work", "apt:curl
")], &[]);
        assert!(load_vars(&f).unwrap().is_empty());
    }

    #[test]
    fn a_matching_block_overrides_the_default() {
        let f = fx("Work
", &[("Work", "apt:curl
")], &[]);
        with_vars(&f, "role = desktop
when os == linux {
  role = travel
}
");
        assert_eq!(load_vars(&f).unwrap()["role"].to_string(), "travel");
    }

    #[test]
    fn a_block_that_does_not_match_leaves_the_default() {
        let f = fx("Work
", &[("Work", "apt:curl
")], &[]);
        with_vars(&f, "role = desktop
when os == plan9 {
  role = travel
}
");
        assert_eq!(load_vars(&f).unwrap()["role"].to_string(), "desktop");
    }

    #[test]
    fn a_variable_defined_only_in_a_block_is_an_error_even_where_the_block_misses() {
        // IX.3 is a property of the FILE. If this were checked only against the blocks that
        // matched, the same repo would be valid on the laptop and broken on the desktop —
        // and the error would appear on whichever machine happened not to define it.
        let f = fx("Work
", &[("Work", "apt:curl
")], &[]);
        with_vars(&f, "when os == plan9 {
  role = travel
}
");
        let err = load_vars(&f).unwrap_err();
        assert!(err.what.contains("only defined inside a `when` block"), "{}", err);
    }

    #[test]
    fn a_gated_line_can_use_a_variable() {
        let f = fx("Work
", &[("Work", "use m
")], &[("m.txt", "apt:curl
when $role == travel {
  apt:mosh
}
")]);
        with_vars(&f, "role = travel
");
        let vars = load_vars(&f).unwrap();
        let d = Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts().with_vars(vars))
            .at(parse_absolute("2026-07-16T12:00").unwrap())
            .resolve()
            .unwrap();
        assert_eq!(names(&d, "apt"), vec!["curl", "mosh"]);
    }

    #[test]
    fn check_reaches_a_module_no_profile_reaches() {
        // II.3. A broken file nobody activated is still broken, and the day it is activated is
        // the worst day to find out. Both errors are returned, not just the first.
        let f = fx(
            "Work\n",
            &[("Work", "use good\n")],
            &[
                ("good.txt", "apt:curl\n"),
                ("orphan.txt", "apt:nginx@requires=libfoo\n"),
                ("other.txt", "-vim\n"),
            ],
        );
        // Resolution is clean: nothing reaches the broken files.
        assert_eq!(names(&resolve(&f).unwrap(), "apt"), ["curl"]);

        let errors = Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts())
            .parse_everything();
        let found: Vec<String> = errors.iter().map(|e| e.origin.to_string()).collect();
        assert_eq!(errors.len(), 2, "{:?}", found);
        assert!(found.iter().any(|o| o.contains("orphan.txt")), "{:?}", found);
        assert!(found.iter().any(|o| o.contains("other.txt")), "{:?}", found);
    }

    #[test]
    fn a_package_carries_every_variable_condition_that_admitted_it() {
        // W11's gating half. The chain crosses three files — `active` turned the profile on,
        // the profile's block let the `use` through, the module's block let the line
        // through — and `why` needs all three, in that order.
        let f = fx(
            "when $role == travel {\n  Trip\n}\n",
            &[("Trip", "when $tier == full {\n  use m\n}\n")],
            &[("m.txt", "when $gpu == true {\n  apt:mosh\n}\napt:curl\n")],
        );
        with_vars(&f, "role = travel\ntier = full\ngpu = true\n");
        let vars = load_vars(&f).unwrap();
        let d = Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts().with_vars(vars))
            .at(parse_absolute("2026-07-16T12:00").unwrap())
            .resolve()
            .unwrap();

        let mosh = d.present().find(|p| p.name == "mosh").unwrap();
        let chain: Vec<&str> = mosh.options["__gated_by"].split(';').collect();
        assert_eq!(chain.len(), 3, "{:?}", chain);
        assert!(chain[0].starts_with("when $role == travel @ "), "{:?}", chain);
        assert!(chain[1].starts_with("when $tier == full @ "), "{:?}", chain);
        assert!(chain[2].starts_with("when $gpu == true @ "), "{:?}", chain);

        // `curl` is outside the module's block but still inside the two that led here.
        let curl = d.present().find(|p| p.name == "curl").unwrap();
        assert_eq!(curl.options["__gated_by"].split(';').count(), 2);
    }

    #[test]
    fn a_condition_that_tests_no_variable_is_not_recorded() {
        // The tag answers "which variable put this here?". A `when host == laptop` has no
        // second hop to explain, and listing it would bury the ones that do.
        let f = fx(
            "Work\n",
            &[("Work", "use m\n")],
            &[("m.txt", "when host == laptop {\n  apt:curl\n}\n")],
        );
        let d = resolve(&f).unwrap();
        let curl = d.present().find(|p| p.name == "curl").unwrap();
        assert!(!curl.options.contains_key("__gated_by"));
    }

    #[test]
    fn the_resolved_state_carries_its_variables_for_the_plan_to_freeze() {
        // IX.6: a saved plan freezes these so `apply` reuses them instead of re-running a
        // provider that might read the clock and disagree with what the plan previewed.
        let f = fx("Work
", &[("Work", "apt:curl
")], &[]);
        with_vars(&f, "role = travel
");
        let vars = load_vars(&f).unwrap();
        let d = Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts().with_vars(vars))
            .at(parse_absolute("2026-07-16T12:00").unwrap())
            .resolve()
            .unwrap();
        assert_eq!(d.vars["role"], crate::model::vars::Value::Str("travel".into()));
    }

    #[test]
    fn a_variable_expands_inside_a_declaration_value() {
        let f = fx(
            "Work
",
            &[("Work", "use m
")],
            &[("m.txt", "link:~/.config/${role}/init.lua
")],
        );
        with_vars(&f, "role = travel
");
        let vars = load_vars(&f).unwrap();
        let d = Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts().with_vars(vars))
            .at(parse_absolute("2026-07-16T12:00").unwrap())
            .resolve()
            .unwrap();
        let names: Vec<String> = d
            .dependents()
            .filter_map(|(s, _)| match s {
                Statement::Link(n, _) => Some(n.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["~/.config/travel/init.lua"]);
    }

    /// With no `vars` file the variable set is empty, and an empty set used to mean "skip
    /// expansion entirely" — so `$role` survived as literal text and became a path with a
    /// dollar in it, failing later and somewhere else. It has to be the same error here that
    /// a misspelled name is when the file does exist.
    #[test]
    fn an_unknown_variable_is_an_error_even_with_no_vars_file() {
        let f = fx(
            "Work\n",
            &[("Work", "use m\n")],
            &[("m.txt", "link:~/.config/$role/init.lua\n")],
        );
        let err = Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts())
            .at(parse_absolute("2026-07-16T12:00").unwrap())
            .resolve()
            .unwrap_err()
            .to_string();
        assert!(err.contains("role"), "{}", err);
    }

    #[test]
    fn a_variable_expands_inside_an_option_value() {
        let f = fx(
            "Work
",
            &[("Work", "use m
")],
            &[("m.txt", "apt:nginx@version=$pinned
")],
        );
        with_vars(&f, "pinned = 1.24.0
");
        let vars = load_vars(&f).unwrap();
        let d = Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts().with_vars(vars))
            .at(parse_absolute("2026-07-16T12:00").unwrap())
            .resolve()
            .unwrap();
        let spec = d.present().find(|p| p.name == "nginx").unwrap();
        assert_eq!(spec.options.get("version").map(String::as_str), Some("1.24.0"));
    }

    #[test]
    fn a_schedules_run_command_keeps_its_dollars_for_the_shell() {
        // `run` is a command line; `$` there belongs to the shell that will execute it.
        let f = fx("Work
", &[("Work", "apt:curl
")], &[]);
        with_vars(&f, "role = travel
");
        std::fs::write(
            f.layout.schedules_file(),
            "schedule:t@cron=0 2 * * *,run=sh -c 'echo $HOME'
",
        )
        .unwrap();
        let vars = load_vars(&f).unwrap();
        let d = Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts().with_vars(vars))
            .at(parse_absolute("2026-07-16T12:00").unwrap())
            .resolve()
            .unwrap();
        let (_, opts, _) = d.schedules().next().unwrap();
        assert_eq!(opts.one("run"), Some("sh -c 'echo $HOME'"));
    }

    #[test]
    fn a_variable_line_outside_the_vars_file_is_refused() {
        let f = fx("Work
", &[("Work", "use m
")], &[("m.txt", "role = travel
apt:curl
")]);
        let err = resolve(&f).unwrap_err();
        assert!(err.what.contains("is not in the `vars` file"), "{}", err);
    }

    #[test]
    fn a_package_line_in_the_vars_file_is_refused() {
        let f = fx("Work
", &[("Work", "apt:curl
")], &[]);
        with_vars(&f, "role = desktop
apt:nginx
");
        let err = load_vars(&f).unwrap_err();
        assert!(err.what.contains("NAME = VALUE"), "{}", err);
    }

    #[test]
    fn a_config_with_no_extras_has_no_dependents() {
        let f = fx("Work\n", &[("Work", "apt:curl\n")], &[]);
        assert!(!resolve(&f).unwrap().has_dependents());
    }

    #[test]
    fn a_bare_name_is_left_for_the_prober() {
        // V.16: the bare name is the question, the lock is the answer. Resolution reads
        // the file; it does not probe.
        let f = fx(
            "Work\n",
            &[("Work", "use base\n")],
            &[("base.txt", "ripgrep\n")],
        );
        let d = resolve(&f).unwrap();
        assert_eq!(names(&d, BARE), ["ripgrep"]);
    }

    /// Resolve with the prober's answers already in hand, as the caller does.
    fn resolve_probed(f: &Fx, answers: &[(&str, &str)]) -> Result<DesiredState> {
        let table: HashMap<String, String> = answers
            .iter()
            .map(|(n, b)| (n.to_string(), b.to_string()))
            .collect();
        let r = Resolver::new(&f.layout, &known, &f.priority)
            .with_facts(facts())
            .at(parse_absolute("2026-07-16T12:00").unwrap())
            .with_bare(table);
        let stmts = r.statements()?;
        r.collect(stmts)
    }

    #[test]
    fn the_probers_answer_becomes_the_backend() {
        let f = fx(
            "Work\n",
            &[("Work", "use base\n")],
            &[("base.txt", "ripgrep\n")],
        );
        let d = resolve_probed(&f, &[("ripgrep", "cargo")]).unwrap();
        assert_eq!(names(&d, "cargo"), ["ripgrep"]);
        assert_eq!(names(&d, BARE), [] as [String; 0]);
    }

    #[test]
    fn a_bare_name_and_an_explicit_one_are_one_package_once_probed() {
        // Probing has to happen BEFORE the merge. Keyed on `?`, `ripgrep` and `cargo:ripgrep`
        // never meet, and the run installs the same package twice — quietly, which is the
        // one thing this model exists to stop.
        let f = fx(
            "Work\n",
            &[("Work", "use a\nuse b\n")],
            &[("a.txt", "ripgrep\n"), ("b.txt", "cargo:ripgrep\n")],
        );
        let d = resolve_probed(&f, &[("ripgrep", "cargo")]).unwrap();
        assert_eq!(names(&d, "cargo"), ["ripgrep"], "one package, not two");
    }

    #[test]
    fn a_probed_bare_name_can_contradict_an_explicit_one() {
        // And having met, they obey rule 5 like any other pair.
        let f = fx(
            "Work\n",
            &[("Work", "use a\nuse b\n")],
            &[
                ("a.txt", "ripgrep@version=14.1.0\n"),
                ("b.txt", "cargo:ripgrep@version=13.0.0\n"),
            ],
        );
        let err = resolve_probed(&f, &[("ripgrep", "cargo")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("a.txt"), "{}", msg);
        assert!(msg.contains("b.txt"), "{}", msg);
    }

    // ------------------------------------------------------------ II.4 set math

    #[test]
    fn a_profile_can_subtract_one_package() {
        // `-vim`: this profile does not want vim, whatever module holds it.
        let f = fx(
            "Work\n",
            &[("Work", "use editors\n-vim\n")],
            &[("editors.txt", "apt:vim\napt:neovim\n")],
        );
        assert_eq!(names(&resolve(&f).unwrap(), "apt"), ["neovim"]);
    }

    #[test]
    fn a_bare_subtraction_takes_the_package_out_whatever_backend_holds_it() {
        let f = fx(
            "Work\n",
            &[("Work", "use editors\n-ripgrep\n")],
            &[("editors.txt", "cargo:ripgrep\napt:vim\n")],
        );
        let d = resolve(&f).unwrap();
        assert_eq!(names(&d, "cargo"), [] as [String; 0]);
        assert_eq!(names(&d, "apt"), ["vim"]);
    }

    #[test]
    fn an_explicit_subtraction_leaves_the_other_backends_line_alone() {
        // You said which one, so only that one goes.
        let f = fx(
            "Work\n",
            &[("Work", "use editors\n-apt:ripgrep\n")],
            &[("editors.txt", "cargo:ripgrep\napt:ripgrep\n")],
        );
        let d = resolve(&f).unwrap();
        assert_eq!(names(&d, "cargo"), ["ripgrep"]);
        assert_eq!(names(&d, "apt"), [] as [String; 0]);
    }

    #[test]
    fn exclude_subtracts_a_whole_modules_packages() {
        let f = fx(
            "Work\n",
            &[("Work", "use everything\nexclude heavy\n")],
            &[
                ("everything.txt", "apt:vim\napt:libreoffice\napt:jq\n"),
                ("heavy.txt", "apt:libreoffice\n"),
            ],
        );
        assert_eq!(names(&resolve(&f).unwrap(), "apt"), ["jq", "vim"]);
    }

    #[test]
    fn intersect_keeps_only_what_is_in_both() {
        let f = fx(
            "Work\n",
            &[("Work", "use everything\nintersect approved\n")],
            &[
                ("everything.txt", "apt:vim\napt:steam\napt:jq\n"),
                ("approved.txt", "apt:vim\napt:jq\napt:never-installed\n"),
            ],
        );
        // Intersect NARROWS. It must not add `never-installed`, which `approved` has and
        // `everything` does not.
        assert_eq!(names(&resolve(&f).unwrap(), "apt"), ["jq", "vim"]);
    }

    #[test]
    fn subtraction_wins_however_you_order_the_lines() {
        // II.4's fixed order: gather, narrow, then subtract. Otherwise `use gaming` after
        // `-steam` quietly puts steam back, and the winner depends on the order you typed.
        let a = fx(
            "Work\n",
            &[("Work", "-steam\nuse gaming\n")],
            &[("gaming.txt", "apt:steam\napt:lutris\n")],
        );
        let b = fx(
            "Work\n",
            &[("Work", "use gaming\n-steam\n")],
            &[("gaming.txt", "apt:steam\napt:lutris\n")],
        );
        assert_eq!(names(&resolve(&a).unwrap(), "apt"), ["lutris"]);
        assert_eq!(names(&resolve(&b).unwrap(), "apt"), ["lutris"]);
    }

    #[test]
    fn a_set_expression_unions_and_intersects() {
        // II.4's headline: `(Work | gaming) & security`.
        let f = fx(
            "Main\n",
            &[
                ("Main", "(Desk | gaming) & security\n"),
                ("Desk", "use editors\n"),
            ],
            &[
                ("editors.txt", "apt:vim\napt:emacs\n"),
                ("gaming.txt", "apt:steam\n"),
                ("security.txt", "apt:vim\napt:steam\napt:gpg\n"),
            ],
        );
        // vim (from Desk) and steam (from gaming) are both in security; emacs is not, and
        // gpg is in security only.
        assert_eq!(names(&resolve(&f).unwrap(), "apt"), ["steam", "vim"]);
    }

    #[test]
    fn a_package_surviving_set_math_still_knows_its_module() {
        // The reason set math maps back to statements rather than to strings: a line keeps
        // its file, so `upgrade --module editors` still finds vim.
        let f = fx(
            "Main\n",
            &[("Main", "use editors\nexclude heavy\n")],
            &[
                ("editors.txt", "apt:vim\n"),
                ("heavy.txt", "apt:libreoffice\n"),
            ],
        );
        let d = resolve(&f).unwrap();
        let vim = d.present().find(|p| p.name == "vim").unwrap();
        let scopes = vim.options.get("__scopes").unwrap();
        assert!(scopes.contains("module:editors"), "{}", scopes);
        assert!(scopes.contains("profile:Main"), "{}", scopes);
    }

    #[test]
    fn a_module_cannot_do_set_math() {
        // II.3: a module is a list. Choosing is the profile's job (V.2).
        for line in ["-vim", "exclude heavy", "intersect secure", "(a | b)"] {
            let f = fx(
                "Work\n",
                &[("Work", "use base\n")],
                &[("base.txt", &format!("apt:curl\n{}\n", line))],
            );
            let err = resolve(&f).unwrap_err();
            assert!(
                err.to_string().contains("module cannot"),
                "`{}` in a module must be refused, got: {}",
                line,
                err
            );
        }
    }

    #[test]
    fn a_profile_cannot_use_absent() {
        // II.4. `absent:` reaches outside what LiNix manages (V.7); `-` only says this
        // profile does not want it.
        let f = fx("Work\n", &[("Work", "absent:apt:steam\n")], &[]);
        let err = resolve(&f).unwrap_err();
        assert!(err.to_string().contains("cannot use `absent:"), "{}", err);
        assert!(err.hint.unwrap().contains("-<package>"));
    }

    #[test]
    fn include_says_to_write_use_instead() {
        // V.46: `use` already means union, and two words for one thing is the disease.
        let f = fx("Work\n", &[("Work", "include editors\n")], &[]);
        let err = resolve(&f).unwrap_err();
        assert!(err.to_string().contains("there is no `include`"), "{}", err);
        assert!(err.hint.unwrap().contains("use editors"));
    }

    #[test]
    fn a_regex_keeps_its_pattern_for_the_prober() {
        let f = fx(
            "Work\n",
            &[("Work", "use fonts\n")],
            &[("fonts.txt", "apt:re:^fonts-\n")],
        );
        let d = resolve(&f).unwrap();
        let spec = d.present().find(|p| p.name == "^fonts-").unwrap();
        assert_eq!(spec.options["__regex"], "^fonts-");
    }

    #[test]
    fn every_spec_records_where_it_came_from() {
        // II.8: every command prints the file it touched, and `why` needs this.
        let f = fx(
            "Work\n",
            &[("Work", "use base\n")],
            &[("base.txt", "apt:curl\n")],
        );
        let d = resolve(&f).unwrap();
        let spec = d.present().find(|p| p.name == "curl").unwrap();
        assert!(spec.options["__source"].contains("base.txt:1"));
    }

    #[test]
    fn an_unapproved_executing_vars_provider_is_refused_until_the_ledger_approves_it() {
        use crate::core::hook_lock::{hash_script, vars_id, HookLedger};

        let f = fx("Work\n", &[("Work", "use base\n")], &[("base.txt", "apt:curl\n")]);
        // An embedded provider is a script that runs at step 0 — it must be approved first.
        let provider = f.layout.config_root().join("vars.linix");
        std::fs::write(&provider, "#{ role: \"work\" }").unwrap();

        let run = || {
            Resolver::new(&f.layout, &known, &f.priority)
                .with_facts(facts())
                .load_vars_with_origins()
        };

        let err = run().unwrap_err().to_string();
        assert!(err.contains("never been approved"), "{}", err);

        // `linix lock` records the current hash; the same provider now runs.
        let locks = f.layout.config_root().join("locks");
        let path = HookLedger::path_in(&locks);
        let mut ledger = HookLedger::load(&path).unwrap();
        let body = std::fs::read_to_string(&provider).unwrap();
        ledger.approve(&vars_id("vars.linix"), &hash_script(&body));
        ledger.save(&path).unwrap();

        let (vars, _) = run().expect("an approved provider runs");
        assert_eq!(
            vars.get("role"),
            Some(&crate::model::vars::Value::Str("work".into()))
        );

        // A changed provider stops again — the case the ledger exists for.
        std::fs::write(&provider, "#{ role: \"travel\" }").unwrap();
        let err = run().unwrap_err().to_string();
        assert!(err.contains("changed") || err.contains("approve"), "{}", err);
    }
}
