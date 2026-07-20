use super::conflict::{reconcile, Declared};
use super::dated::dating_of;
use super::layout::Layout;
use super::modules::{expand, ModuleLoader};
use super::priority::Priority;
use super::profiles::{parse_active, ProfileLoader, SetOp};
use crate::config::grammar::{
    statement, BackendNames, GrammarError, Options, Origin, PackageDecl, Result, Selector,
    Statement,
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
                Statement::Shim(..) | Statement::Service(..) | Statement::Link(..)
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
}

/// What the active profiles reach: the statements, and which profile and module each file's
/// lines belong to.
///
/// The scopes are collected here because this is the only place that knows them. Once the
/// statements are flattened, "profile `Work` reaches module `dev`" is gone, and `linix
/// upgrade --profile Work` has no way to ask.
pub struct Reached {
    pub statements: Vec<(Statement, Origin)>,
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
        }
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

    /// II.7 steps 1-3: `active` -> profiles -> the modules they reach, parsed and gated.
    ///
    /// Split from `collect` because resolving a bare name needs the network and this does
    /// not. The caller probes the bare names these statements carry, then hands the answers
    /// back to `collect` via `with_bare` — so the merge in `collect` sees real backends.
    pub fn statements(&self) -> Result<Reached> {
        // 1. Read `active` -> the profile set.
        let active_file = self.layout.active_file();
        let body = std::fs::read_to_string(&active_file).unwrap_or_default();
        let active = parse_active(&active_file, &body)?;

        let mut out = Reached {
            statements: Vec::new(),
            scopes: HashMap::new(),
        };

        // 2. Resolve profiles -> the module set. Profiles may reference profiles; modules
        //    may not.
        let profiles = ProfileLoader::new(self.layout, self.backends);
        let mut wanted_modules: Vec<String> = Vec::new();
        // Which profiles want each module. A module two profiles both reach belongs to
        // both, and `upgrade --profile` for either must find it.
        let mut wanted_by: HashMap<String, Vec<String>> = HashMap::new();
        let mut direct: Vec<(Statement, Origin)> = Vec::new();
        let mut loader = ModuleLoader::new(self.layout, self.backends);
        let asked = Origin::new(&active_file, 0);

        for name in &active {
            let r = profiles.resolve(name, &asked, &self.facts, &mut Vec::new())?;

            // A profile doing set math resolves to packages, not to modules: an
            // intersection of two modules' packages is not a module (V.46). So it is
            // materialised here and its result joins `direct`.
            if r.does_set_math() {
                let stmts = self.apply_set_math(&profiles, &mut loader, &r, &asked)?;
                for (_, origin) in &stmts {
                    out.record(&origin.file, format!("profile:{}", name));
                    self.record_module_scope(&mut out, origin);
                }
                direct.extend(stmts);
                continue;
            }

            for m in r.modules {
                if !wanted_modules.contains(&m) {
                    wanted_modules.push(m.clone());
                }
                wanted_by.entry(m).or_default().push(name.clone());
            }
            // A profile's own package lines belong to the profile and to no module: a
            // module can never reach them, which is the cost II.4 accepts knowingly (V.3).
            for (_, origin) in &r.direct {
                out.record(&origin.file, format!("profile:{}", name));
            }
            direct.extend(r.direct);
        }

        // 3. Parse ONLY the modules reached. Apply `when`.
        for m in &wanted_modules {
            let stmts = expand(
                &mut loader,
                m,
                &Origin::new(&active_file, 0),
                &self.facts,
                &mut Vec::new(),
            )?;
            // Attributed by the file each line actually came from, so a module reached
            // through another module is scoped to itself and to the profile that led here.
            for (_, origin) in &stmts {
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
            out.statements.extend(doc.statements_for(&self.facts)?);
        }

        Ok(out)
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
    ) -> Result<Vec<(Statement, Origin)>> {
        let mut base: Vec<(Statement, Origin)> = Vec::new();
        for m in &r.modules {
            base.extend(expand(loader, m, asked, &self.facts, &mut Vec::new())?);
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
                    intersects.push(other.iter().map(|(s, _)| set_key(s)).collect());
                }
                SetOp::Exclude(reference) => {
                    let other = self.atom(profiles, loader, reference.name(), origin)?;
                    subtract.extend(other.iter().map(|(s, _)| set_key(s)));
                }
                SetOp::Subtract(pkg) => subtract.push(pkg.trim().to_string()),
            }
        }

        for keep in &intersects {
            base.retain(|(s, _)| keep.iter().any(|k| same_package(k, &set_key(s))));
        }
        base.retain(|(s, _)| !subtract.iter().any(|k| same_package(k, &set_key(s))));

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
    ) -> Result<Vec<(Statement, Origin)>> {
        let mut table: HashMap<String, (Statement, Origin)> = HashMap::new();
        let mut failure: Option<GrammarError> = None;

        let keys = crate::app::profile_expr::evaluate(expr, &mut |atom| {
            match self.atom(profiles, loader, atom, origin) {
                Ok(stmts) => stmts
                    .into_iter()
                    .map(|(s, o)| {
                        let k = set_key(&s);
                        table.entry(k.clone()).or_insert((s, o));
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
    ) -> Result<Vec<(Statement, Origin)>> {
        let capitalized = atom.chars().next().is_some_and(char::is_uppercase);

        if capitalized {
            let r = profiles.resolve(atom, origin, &self.facts, &mut Vec::new())?;
            return self.apply_set_math(profiles, loader, &r, origin);
        }

        // A name that cannot be a module is not a broken module — it falls through to the
        // package parse below, which is where `(Work | apt:jq)` is meant to land.
        let is_module = super::ModuleName::new(atom)
            .map(|m| self.layout.module_file(&m).is_file())
            .unwrap_or(false);
        if is_module {
            return expand(loader, atom, origin, &self.facts, &mut Vec::new());
        }

        // Not a file, so it is a package named literally: `(Work | jq)`. Parsed, so a typo
        // is an error here rather than a package nobody has.
        let stmt = statement::parse(origin, atom, self.backends)?;
        Ok(vec![(stmt, origin.clone())])
    }

    /// II.7 steps 4-7: resolve each line, conflicts are errors, dated lines get rule 6.
    pub fn collect(&self, reached: Reached) -> Result<DesiredState> {
        // Keyed `backend:name` so two declarations of one package meet each other. BTreeMap
        // so the plan and any error list in a stable order rather than a hash one.
        let mut merged: BTreeMap<String, Entry> = BTreeMap::new();
        let mut out = DesiredState::default();

        for (stmt, origin) in reached.statements.iter().cloned() {
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
            let spec = to_spec(
                &e.backend,
                &e.selector,
                &e.declared.options,
                &e.declared.origin,
                e.declared.present,
                &scopes,
            );
            out.packages.entry(e.backend).or_default().push(spec);
        }

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
/// Not the resolved backend, because nothing has probed yet — set math happens while reading
/// the files, and probing needs the network.
fn set_key(stmt: &Statement) -> String {
    match stmt {
        Statement::Package(d) | Statement::Absent(d) => match &d.backend {
            Some(b) => format!("{}:{}", b, d.selector.as_str()),
            None => d.selector.as_str().to_string(),
        },
        Statement::Repo { backend, spec } => format!("repo:{}:{}", backend, spec),
        Statement::Shim(n, _) => format!("shim:{}", n),
        Statement::Schedule(n, _) => format!("schedule:{}", n),
        Statement::Service(n, _) => format!("service:{}", n),
        Statement::Link(n, _) => format!("link:{}", n),
        Statement::Use(r) => format!("use {}", r.name()),
        Statement::Exclude(r) => format!("exclude {}", r.name()),
        Statement::Intersect(r) => format!("intersect {}", r.name()),
        Statement::Subtract(p) => format!("-{}", p),
        Statement::Expr(e) => e.clone(),
    }
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
}

/// Build the seam's `PackageSpec` from one declaration. The only place this conversion
/// happens: an imperative `linix install jq` and a line in a module must produce the same
/// spec, or the two paths drift (P4).
pub fn to_spec(
    backend: &str,
    selector: &Selector,
    options: &Options,
    origin: &Origin,
    present: bool,
    scopes: &[String],
) -> PackageSpec {
    let mut properties: HashMap<String, String> = HashMap::new();
    for (k, vs) in options.iter() {
        // `requires` is a list; the rest are single values. Joined with `;` because that is
        // what the planner already splits on.
        properties.insert(k.to_string(), vs.join(";"));
    }
    // Two different questions, two tags. `__source` is where the line is, for the human
    // reading an error or "Added jq to modules/imperative.txt" (II.8). `__scopes` is what
    // it belongs to, for `--module` / `--profile` to match on. One tag answering both is
    // how `upgrade --module dev` came to be matched against a filename.
    properties.insert("__source".to_string(), origin.to_string());
    if !scopes.is_empty() {
        properties.insert("__scopes".to_string(), scopes.join(";"));
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
}
