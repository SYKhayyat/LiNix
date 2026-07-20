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
    /// is no module to name (V.46).
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
    Ok(read_active(file, body)?
        .into_iter()
        .filter(|e| e.on)
        .map(|e| e.name)
        .collect())
}

/// One name in `active`, and whether this machine gets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveEntry {
    pub name: String,
    /// 1-based, as an editor counts.
    pub line: usize,
    /// Inside a `when` block, and if so which — so `deactivate` can say *"it is still
    /// activated by the `when` block on line 4"* rather than silently doing nothing.
    pub gate: Option<String>,
    /// Whether it applies to this host. A name inside a `when` that does not match is in
    /// the file and not in force.
    pub on: bool,
}

/// Read `active` with its `when` blocks intact.
///
/// `when` gates it like any other file — one rule, everywhere (II.2). `active` used to be
/// the exception: it rejected any line with more than one word, so the `when host == laptop
/// {` in II.6's own example was a hard error.
pub fn read_active(file: &std::path::Path, body: &str) -> Result<Vec<ActiveEntry>> {
    let facts = HostFacts::current();
    read_active_with(file, body, &facts)
}

pub fn read_active_with(
    file: &std::path::Path,
    body: &str,
    facts: &HostFacts,
) -> Result<Vec<ActiveEntry>> {
    let mut out: Vec<ActiveEntry> = Vec::new();
    let mut gate: Option<(String, bool)> = None;

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

        if line == "}" {
            if gate.is_none() {
                return Err(GrammarError::new(
                    origin,
                    "`}` closes a `when` that was never opened",
                ));
            }
            gate = None;
            continue;
        }

        if let Some(header) = line.strip_suffix('{') {
            let header = header.trim();
            let Some(pred) = header.strip_prefix("when ") else {
                return Err(GrammarError::new(
                    origin,
                    format!("`{}` is not a `when` block", header),
                )
                .with_hint("`active` holds profile names and `when` blocks, nothing else."));
            };
            if gate.is_some() {
                return Err(GrammarError::new(origin, "a `when` block inside a `when` block")
                    .with_hint("`active` nests one level: name the condition once."));
            }
            let hit = crate::config::parser::eval_when(pred.trim(), facts)
                .map_err(|e| GrammarError::new(Origin::new(file, idx + 1), e.to_string()))?;
            gate = Some((pred.trim().to_string(), hit));
            continue;
        }

        if line.split_whitespace().count() > 1 {
            return Err(
                GrammarError::new(origin, format!("`{}` is not a profile name", line)).with_hint(
                    "`active` is a list of profile names, one per line, and `when` blocks.                      It answers one question: what is this machine set to right now?",
                ),
            );
        }
        if !line.chars().next().is_some_and(char::is_uppercase) {
            return Err(
                GrammarError::new(origin, format!("`{}` is not a profile name", line))
                    .with_hint("profiles are Capitalized, modules are lowercase. Only profiles can be activated."),
            );
        }
        if out.iter().any(|e| e.name == line) {
            continue;
        }
        out.push(ActiveEntry {
            name: line.to_string(),
            line: idx + 1,
            gate: gate.as_ref().map(|(p, _)| p.clone()),
            on: gate.as_ref().map(|(_, hit)| *hit).unwrap_or(true),
        });
    }

    if gate.is_some() {
        return Err(
            GrammarError::new(Origin::new(file, 0), "a `when` block is never closed")
                .with_hint("add the matching `}`."),
        );
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
            family: "debian".into(),
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

#[cfg(test)]
mod active_tests {
    use super::*;
    use std::path::PathBuf;

    fn facts(host: &str) -> HostFacts {
        HostFacts {
            os: "linux".into(),
            arch: "x86_64".into(),
            host: host.into(),
            family: "debian".into(),
        }
    }

    fn read(body: &str, host: &str) -> Result<Vec<ActiveEntry>> {
        read_active_with(&PathBuf::from("active"), body, &facts(host))
    }

    fn on(body: &str, host: &str) -> Vec<String> {
        read(body, host)
            .unwrap()
            .into_iter()
            .filter(|e| e.on)
            .map(|e| e.name)
            .collect()
    }

    /// II.6's own example file. It did not parse: `active` rejected any line with more than
    /// one word, so `when host == laptop {` was a hard error — the one file that broke
    /// II.2's "one rule, everywhere".
    const II6_EXAMPLE: &str = "Work\nGaming\n\nwhen host == laptop {\n  Travel\n}\n";

    #[test]
    fn the_example_in_the_spec_parses() {
        assert_eq!(on(II6_EXAMPLE, "laptop"), ["Work", "Gaming", "Travel"]);
    }

    #[test]
    fn when_gates_active_like_every_other_file() {
        assert_eq!(on(II6_EXAMPLE, "server"), ["Work", "Gaming"]);
    }

    #[test]
    fn a_gated_name_is_in_the_file_and_says_which_block_holds_it() {
        // What `deactivate` needs to say "it is still activated by the `when` block on
        // line 4" rather than silently doing nothing.
        let entries = read(II6_EXAMPLE, "laptop").unwrap();
        let travel = entries.iter().find(|e| e.name == "Travel").unwrap();
        assert_eq!(travel.gate.as_deref(), Some("host == laptop"));
        assert_eq!(travel.line, 5);
        assert!(travel.on);

        // On another host it is still in the file, just not in force.
        let entries = read(II6_EXAMPLE, "server").unwrap();
        let travel = entries.iter().find(|e| e.name == "Travel").unwrap();
        assert!(!travel.on);
    }

    #[test]
    fn comments_and_blanks_are_skipped() {
        assert_eq!(on("# what I am\n\nWork   # for work\n", "any"), ["Work"]);
    }

    #[test]
    fn a_repeat_is_ignored() {
        assert_eq!(on("Work\nWork\n", "any"), ["Work"]);
    }

    #[test]
    fn a_lowercase_name_is_not_a_profile() {
        let err = read("editors\n", "any").unwrap_err();
        assert!(err.hint.unwrap().contains("profiles are Capitalized"));
    }

    #[test]
    fn an_unclosed_or_stray_block_is_an_error() {
        assert!(read("when host == laptop {\n  Travel\n", "laptop").is_err());
        assert!(read("Work\n}\n", "any").is_err());
        assert!(read("when a == b {\n when c == d {\n Work\n}\n}\n", "any").is_err());
    }

    #[test]
    fn active_holds_names_never_expressions() {
        // II.6: the set math lives inside profiles. `active` stays a list you can read at
        // a glance, because it is the one file you open to know what is on.
        assert!(read("(Work | Gaming)\n", "any").is_err());
    }
}
