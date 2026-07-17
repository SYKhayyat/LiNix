use super::conflict::{reconcile, Declared};
use super::dated::dating_of;
use super::layout::Layout;
use super::modules::{expand, ModuleLoader};
use super::priority::Priority;
use super::profiles::{parse_active, ProfileLoader};
use crate::config::grammar::{
    BackendNames, GrammarError, Options, Origin, PackageDecl, Result, Selector, Statement,
};
use crate::config::parser::HostFacts;
use crate::core::PackageSpec;
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashMap};

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

    /// The seam: `HashMap<backend, Vec<PackageSpec>>`, install intents only. What the
    /// planner has always consumed.
    pub fn into_install_map(self) -> HashMap<String, Vec<PackageSpec>> {
        self.packages
            .into_iter()
            .filter_map(|(b, specs)| {
                let keep: Vec<PackageSpec> = specs.into_iter().filter(|p| p.present).collect();
                (!keep.is_empty()).then_some((b, keep))
            })
            .collect()
    }
}

/// Resolves `active` -> the desired state (SPEC II.7).
pub struct Resolver<'a> {
    layout: &'a Layout,
    backends: &'a dyn BackendNames,
    priority: &'a Priority,
    facts: HostFacts,
    now: DateTime<Utc>,
}

impl<'a> Resolver<'a> {
    pub fn new(layout: &'a Layout, backends: &'a dyn BackendNames, priority: &'a Priority) -> Self {
        Self {
            layout,
            backends,
            priority,
            facts: HostFacts::current(),
            now: Utc::now(),
        }
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
        // 1. Read `active` -> the profile set.
        let active_file = self.layout.active_file();
        let body = std::fs::read_to_string(&active_file).unwrap_or_default();
        let active = parse_active(&active_file, &body)?;

        // 2. Resolve profiles -> the module set. Profiles may reference profiles; modules
        //    may not.
        let profiles = ProfileLoader::new(self.layout, self.backends);
        let mut wanted_modules: Vec<String> = Vec::new();
        let mut direct: Vec<(Statement, Origin)> = Vec::new();
        for name in &active {
            let r = profiles.resolve(name, &Origin::new(&active_file, 0), &self.facts, &mut Vec::new())?;
            for m in r.modules {
                if !wanted_modules.contains(&m) {
                    wanted_modules.push(m);
                }
            }
            direct.extend(r.direct);
        }

        // 3. Parse ONLY the modules reached. Apply `when`.
        let mut loader = ModuleLoader::new(self.layout, self.backends);
        let mut statements: Vec<(Statement, Origin)> = Vec::new();
        for m in &wanted_modules {
            statements.extend(expand(
                &mut loader,
                m,
                &Origin::new(&active_file, 0),
                &self.facts,
                &mut Vec::new(),
            )?);
        }
        statements.extend(direct);

        // 4-6. Resolve each line; conflicts are errors; dated lines get rule 6.
        self.collect(statements)
    }

    fn collect(&self, statements: Vec<(Statement, Origin)>) -> Result<DesiredState> {
        // Keyed `backend:name` so two declarations of one package meet each other. BTreeMap
        // so the plan and any error list in a stable order rather than a hash one.
        let mut merged: BTreeMap<String, (Declared, Option<String>, Selector)> = BTreeMap::new();
        let mut out = DesiredState::default();

        for (stmt, origin) in statements {
            let (decl, present) = match stmt {
                Statement::Package(d) => (d, true),
                Statement::Absent(d) => (d, false),
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
                origin,
                present,
            };

            match merged.remove(&key) {
                Some((existing, b, sel)) => {
                    let winner = reconcile(&key, existing, incoming, self.now)?;
                    merged.insert(key, (winner, b, sel));
                }
                None => {
                    merged.insert(key, (incoming, Some(backend), decl.selector.clone()));
                }
            }
        }

        for (key, (declared, backend, selector)) in merged {
            // A line whose date has passed has no opinion at all — it is not "absent", it
            // simply stops counting (II.7 rule 6).
            if dating_of(&declared.options, self.now) == super::dated::Dating::Lapsed {
                continue;
            }
            let backend = backend.unwrap_or_else(|| key.split(':').next().unwrap_or("").to_string());
            let spec = to_spec(
                &backend,
                &selector,
                &declared.options,
                &declared.origin,
                declared.present,
            );
            out.packages.entry(backend).or_default().push(spec);
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
                // Left for the prober: the bare name is the question, the lock is the
                // answer (V.16). Marked so nothing downstream mistakes it for a decision.
                Ok(BARE.to_string())
            }
        }
    }
}

/// The backend of a bare name, before probing. Never reaches a backend: `Resolver::resolve`
/// hands these to the prober, which replaces it with the lock's answer or `priority`'s.
pub const BARE: &str = "?";

fn to_spec(
    backend: &str,
    selector: &Selector,
    options: &Options,
    origin: &Origin,
    present: bool,
) -> PackageSpec {
    let mut properties: HashMap<String, String> = HashMap::new();
    for (k, vs) in options.iter() {
        // `requires` is a list; the rest are single values. Joined with `;` because that is
        // what the planner already splits on.
        properties.insert(k.to_string(), vs.join(";"));
    }
    properties.insert("__source".to_string(), origin.to_string());
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
            family: "unix".into(),
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
    fn extras_are_carried_in_declaration_order() {
        let f = fx(
            "Work\n",
            &[("Work", "use py\n")],
            &[("py.txt", "repo:ppa:deadsnakes/ppa\napt:python3.12\n")],
        );
        let d = resolve(&f).unwrap();
        assert!(matches!(d.extras[0].0, Statement::Repo(_)));
        assert_eq!(names(&d, "apt"), ["python3.12"]);
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
