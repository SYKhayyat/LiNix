use super::layout::Layout;
use crate::config::grammar::{
    parse_document, BackendNames, GrammarError, Origin, Reference, Result, Statement,
};
use crate::config::parser::HostFacts;

/// What a profile resolved to: the modules it reaches, the lines it holds directly, and the
/// set math it applies to the result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    /// Module names, in first-seen order.
    pub modules: Vec<String>,
    /// **A profile MAY hold package lines directly** (II.4). A cost accepted knowingly: a
    /// module can never reach them (the layering rule), so they are unshareable,
    /// permanently — and you find out the day you want to share them (V.3).
    pub direct: Vec<(Statement, Origin)>,
    /// II.4's set math, in the order written. Applied by the caller, which is the only
    /// thing that can turn a module name into the packages to intersect or subtract.
    pub ops: Vec<(SetOp, Origin)>,
}

/// One set operation from a profile (II.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetOp {
    /// `exclude heavy` — take that module's or profile's packages out.
    Exclude(Reference),
    /// `intersect security` — keep only what is also in it.
    Intersect(Reference),
    /// `-vim` — take one package out.
    Subtract(String),
    /// `(Work | gaming) & security`.
    Expr(String),
}

impl Resolved {
    /// Whether this profile does set math at all.
    ///
    /// It decides the shape of the answer: without it a profile names modules and each
    /// package keeps its module's name, with it the profile resolves to packages and there
    /// is no module to name (V.44).
    pub fn does_set_math(&self) -> bool {
        !self.ops.is_empty()
    }
}

/// Loads and composes profiles (SPEC II.4).
///
/// **Only profiles can be activated.** Set math over modules and profiles: `|` union, `&`
/// intersect, `\` difference, parentheses — resolved at read time, with no
/// `_active_profiles.txt` and no materialization.
pub struct ProfileLoader<'a> {
    layout: &'a Layout,
    backends: &'a dyn BackendNames,
}

impl<'a> ProfileLoader<'a> {
    pub fn new(layout: &'a Layout, backends: &'a dyn BackendNames) -> Self {
        Self { layout, backends }
    }

    /// Every profile the folder holds. Capitalized names (II.5).
    pub fn available(&self) -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(self.layout.profiles_dir()) else {
            return Vec::new();
        };
        let mut out: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.chars().next().is_some_and(char::is_uppercase))
            .collect();
        out.sort();
        out
    }

    pub fn exists(&self, name: &str) -> bool {
        self.layout.profile_file(name).is_file()
    }

    /// Resolve a profile to the modules it reaches and the lines it holds.
    pub fn resolve(
        &self,
        name: &str,
        asked_by: &Origin,
        facts: &HostFacts,
        seen: &mut Vec<String>,
    ) -> Result<Resolved> {
        if seen.iter().any(|s| s == name) {
            return Err(GrammarError::new(
                asked_by.clone(),
                format!(
                    "profile `{}` ends up using itself: {} -> {}",
                    name,
                    seen.join(" -> "),
                    name
                ),
            ));
        }
        seen.push(name.to_string());

        let path = self.layout.profile_file(name);
        let body = std::fs::read_to_string(&path).map_err(|_| self.missing(name, asked_by))?;
        let doc = parse_document(&path, &body, self.backends)?;

        let mut out = Resolved::default();
        for (stmt, origin) in doc.statements_for(facts)? {
            match stmt {
                Statement::Use(Reference::Module(m)) => {
                    if !out.modules.contains(&m) {
                        out.modules.push(m);
                    }
                }
                // Profiles may reference profiles; modules may not (II.7 step 2).
                Statement::Use(Reference::Profile(p)) => {
                    let inner = self.resolve(&p, &origin, facts, seen)?;
                    for m in inner.modules {
                        if !out.modules.contains(&m) {
                            out.modules.push(m);
                        }
                    }
                    out.direct.extend(inner.direct);
                    // A profile's set math travels with it: `use Work` where Work excludes
                    // heavy means you asked for Work, and Work is Work-without-heavy.
                    out.ops.extend(inner.ops);
                }

                Statement::Exclude(r) => out.ops.push((SetOp::Exclude(r), origin)),
                Statement::Intersect(r) => out.ops.push((SetOp::Intersect(r), origin)),
                Statement::Subtract(p) => out.ops.push((SetOp::Subtract(p), origin)),
                Statement::Expr(e) => out.ops.push((SetOp::Expr(e), origin)),

                // II.4: `absent:` does not exist in profiles. `-` does. `absent:` reaches
                // outside what LiNix manages and deletes something you never declared
                // (V.7); `-vim` only says this profile does not want vim.
                Statement::Absent(d) => {
                    return Err(GrammarError::new(
                        origin,
                        format!("a profile cannot use `absent:{}`", d.selector.as_str()),
                    )
                    .with_hint(
                        "write `-<package>` to leave it out of this profile, or put the \
                         `absent:` line in a module if you mean it must not exist at all.",
                    ))
                }

                other => out.direct.push((other, origin)),
            }
        }

        seen.pop();
        Ok(out)
    }

    /// II.5's error must teach the rule, not just say no.
    fn missing(&self, name: &str, asked_by: &Origin) -> GrammarError {
        let modules_dir = self.layout.modules_dir();
        let lower = name.to_lowercase();
        if modules_dir.join(format!("{}.txt", lower)).is_file() {
            return GrammarError::new(
                asked_by.clone(),
                format!("no profile named `{}`", name),
            )
            .with_hint(format!(
                "did you mean the module `{}`? Profiles are Capitalized, modules are lowercase.",
                lower
            ));
        }
        let available = self.available();
        let hint = if available.is_empty() {
            "`profiles/` holds no profiles yet.".to_string()
        } else {
            format!("Profiles on this machine: {}.", available.join(", "))
        };
        GrammarError::new(asked_by.clone(), format!("no profile named `{}`", name)).with_hint(hint)
    }
}

/// The `active` file: a plain list of profile names, unioned (SPEC II.6).
///
/// Answers exactly one question — *what is this machine set to right now?* Nothing else
/// goes in it.
pub fn parse_active(file: &std::path::Path, body: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (idx, raw) in body.lines().enumerate() {
        let origin = Origin::new(file, idx + 1);
        let line = match raw.find('#') {
            Some(i) => &raw[..i],
            None => raw,
        }
        .trim();
        if line.is_empty() {
            continue;
        }
        if line.split_whitespace().count() > 1 {
            return Err(
                GrammarError::new(origin, format!("`{}` is not a profile name", line)).with_hint(
                    "`active` is a plain list of profile names, one per line. It answers one \
                     question: what is this machine set to right now?",
                ),
            );
        }
        if !line.chars().next().is_some_and(char::is_uppercase) {
            return Err(
                GrammarError::new(origin, format!("`{}` is not a profile name", line))
                    .with_hint("profiles are Capitalized, modules are lowercase. Only profiles can be activated."),
            );
        }
        if !out.iter().any(|x| x == line) {
            out.push(line.to_string());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn known(name: &str) -> bool {
        matches!(name, "apt" | "cargo")
    }

    fn facts() -> HostFacts {
        HostFacts {
            os: "linux".into(),
            arch: "x86_64".into(),
            host: "laptop".into(),
            family: "unix".into(),
        }
    }

    struct Fixture {
        _tmp: TempDir,
        layout: Layout,
    }

    fn fixture(profiles: &[(&str, &str)], modules: &[(&str, &str)]) -> Fixture {
        let tmp = TempDir::new().unwrap();
        let layout = Layout::new(tmp.path().join("cfg"), tmp.path().join("data"));
        std::fs::create_dir_all(layout.profiles_dir()).unwrap();
        std::fs::create_dir_all(layout.modules_dir()).unwrap();
        for (n, b) in profiles {
            std::fs::write(layout.profiles_dir().join(n), b).unwrap();
        }
        for (n, b) in modules {
            std::fs::write(layout.modules_dir().join(n), b).unwrap();
        }
        Fixture { _tmp: tmp, layout }
    }

    fn resolve(f: &Fixture, name: &str) -> Result<Resolved> {
        ProfileLoader::new(&f.layout, &known).resolve(
            name,
            &Origin::argument(),
            &facts(),
            &mut Vec::new(),
        )
    }

    #[test]
    fn a_profile_chooses_modules() {
        let f = fixture(&[("Work", "use editors\nuse dev\n")], &[]);
        assert_eq!(resolve(&f, "Work").unwrap().modules, ["editors", "dev"]);
    }

    #[test]
    fn a_profile_may_hold_package_lines_directly() {
        // II.4/V.3, accepted knowingly: `--into Work` is a real want, and the cost is that
        // a module can never reach these.
        let f = fixture(&[("Work", "use editors\napt:slack\n")], &[]);
        let r = resolve(&f, "Work").unwrap();
        assert_eq!(r.modules, ["editors"]);
        assert_eq!(r.direct.len(), 1);
    }

    #[test]
    fn a_profile_may_reference_a_profile() {
        // II.7 step 2. The opposite direction is the one that is forbidden.
        let f = fixture(
            &[("Work", "use Base\nuse dev\n"), ("Base", "use editors\n")],
            &[],
        );
        assert_eq!(resolve(&f, "Work").unwrap().modules, ["editors", "dev"]);
    }

    #[test]
    fn a_profile_that_uses_itself_is_an_error_not_a_hang() {
        let f = fixture(&[("A", "use B\n"), ("B", "use A\n")], &[]);
        assert!(resolve(&f, "A")
            .unwrap_err()
            .what
            .contains("ends up using itself"));
    }

    #[test]
    fn a_missing_profile_that_matches_a_module_teaches_the_rule() {
        // II.5's exact message.
        let f = fixture(&[], &[("editors.txt", "apt:neovim\n")]);
        let err = resolve(&f, "Editors").unwrap_err();
        assert!(err.what.contains("no profile named `Editors`"), "{}", err);
        let hint = err.hint.unwrap();
        assert!(hint.contains("did you mean the module `editors`"), "{}", hint);
        assert!(hint.contains("Profiles are Capitalized, modules are lowercase"));
    }

    #[test]
    fn active_is_a_plain_list_of_profile_names() {
        let out = parse_active(&PathBuf::from("active"), "# on now\nWork\nGaming\n").unwrap();
        assert_eq!(out, ["Work", "Gaming"]);
    }

    #[test]
    fn active_refuses_a_module_name() {
        // Only profiles can be activated (II.4).
        let err = parse_active(&PathBuf::from("active"), "editors\n").unwrap_err();
        assert!(err.hint.unwrap().contains("Only profiles can be activated"));
    }

    #[test]
    fn active_refuses_anything_that_is_not_a_name() {
        assert!(parse_active(&PathBuf::from("active"), "Work | Gaming\n").is_err());
    }

    #[test]
    fn active_ignores_a_repeat() {
        let out = parse_active(&PathBuf::from("active"), "Work\nWork\n").unwrap();
        assert_eq!(out, ["Work"]);
    }

}
