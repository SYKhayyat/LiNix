use super::layout::Layout;
use crate::config::grammar::{
    gated, parse_document, BackendNames, Gates, GrammarError, Origin, Reference, Result,
    Statement, Vocabulary,
};
use crate::config::parser::HostFacts;

/// What a profile resolved to: the modules it reaches, the lines it holds directly, and the
/// set math it applies to the result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    /// Module names, in first-seen order, each with the `when` conditions that led to it.
    pub modules: Vec<UsedModule>,
    /// **A profile MAY hold package lines directly** (II.4). A cost accepted knowingly: a
    /// module can never reach them (the layering rule), so they are unshareable,
    /// permanently — and you find out the day you want to share them (V.3).
    pub direct: Vec<(Statement, Origin, Gates)>,
    /// II.4's set math, in the order written. Applied by the caller, which is the only
    /// thing that can turn a module name into the packages to intersect or subtract.
    pub ops: Vec<(SetOp, Origin)>,
}

/// A module a profile reaches, and what had to be true to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsedModule {
    pub name: String,
    pub gates: Gates,
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

    /// Record a module this profile reaches.
    ///
    /// A module named twice keeps the **shortest** gate chain, not the first: reached once
    /// inside `when $role == travel` and once outside it, the truth is that it is here
    /// unconditionally, and an explanation that names the condition anyway is a wrong answer.
    fn reach(&mut self, name: String, gates: Gates) {
        match self.modules.iter_mut().find(|m| m.name == name) {
            Some(existing) if gates.len() < existing.gates.len() => existing.gates = gates,
            Some(_) => {}
            None => self.modules.push(UsedModule { name, gates }),
        }
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
        inherited: &Gates,
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
        for (stmt, origin, own) in doc.statements_with_gating(facts)? {
            let mut gates = inherited.to_vec();
            gates.extend(own);
            match stmt {
                Statement::Use(Reference::Module(m)) => out.reach(m, gates),
                // Profiles may reference profiles; modules may not (II.7 step 2).
                Statement::Use(Reference::Profile(p)) => {
                    let inner = self.resolve(&p, &origin, facts, seen, &gates)?;
                    for m in inner.modules {
                        out.reach(m.name, m.gates);
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

                other => out.direct.push((other, origin, gates)),
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
///
/// **`facts` must carry this run's variables.** There is deliberately no form that detects
/// its own: a `when $role == travel` block read against the empty set is not a block that
/// fails to match, it is an unknown key, and every caller that took the convenient form
/// refused a correct file (W8).
pub fn parse_active(
    file: &std::path::Path,
    body: &str,
    facts: &HostFacts,
) -> Result<Vec<String>> {
    Ok(read_active(file, body, facts)?
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
pub fn read_active(
    file: &std::path::Path,
    body: &str,
    facts: &HostFacts,
) -> Result<Vec<ActiveEntry>> {
    // The block structure is the shared one (`grammar::gated`) — `priority` reads the same
    // shape, and two copies of it had already drifted. What is left here is the one rule
    // that belongs to this file: a line names a profile, and profiles are Capitalized.
    let vocab = Vocabulary {
        noun: "profile name",
        holds: "`active` is a list of profile names, one per line, and `when` blocks. It \
                answers one question: what is this machine set to right now?",
        nesting: "`active` nests one level: name the condition once.",
        // A profile name answers one question and has nothing to configure.
        body: None,
    };

    let mut out: Vec<ActiveEntry> = Vec::new();
    for entry in gated::read(file, body, facts, &vocab)? {
        if !entry.text.chars().next().is_some_and(char::is_uppercase) {
            return Err(GrammarError::new(
                Origin::new(file, entry.line),
                format!("`{}` is not a profile name", entry.text),
            )
            .with_hint(
                "profiles are Capitalized, modules are lowercase. Only profiles can be \
                 activated.",
            ));
        }
        if out.iter().any(|e| e.name == entry.text) {
            continue;
        }
        out.push(ActiveEntry {
            name: entry.text,
            line: entry.line,
            gate: entry.gate,
            on: entry.on,
        });
    }
    Ok(out)
}

/// A `when` block as a message names it: the condition, and what its variables are on this
/// machine right now.
///
/// A message naming a block without its variables' values points the reader at a file that
/// does not contain the answer — `active` says `when $role == travel`, and what `$role` is
/// was decided in `vars`, or by a program (W8).
pub fn describe_gate(predicate: &str, facts: &HostFacts) -> String {
    let values: Vec<String> = crate::model::vars::referenced_names(predicate)
        .into_iter()
        .map(|n| match facts.vars.get(&n) {
            Some(v) => format!("${} is {}", n, v),
            None => format!("${} is undefined", n),
        })
        .collect();
    if values.is_empty() {
        return format!("`when {}`", predicate);
    }
    format!("`when {}` ({})", predicate, values.join(", "))
}

/// A name `deactivate` took out, and the `when` block it came from if it was in one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub name: String,
    pub line: usize,
    pub gate: Option<String>,
}

/// A `when` block that went with the last name in it, or that `activate` replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDrop {
    pub predicate: String,
    pub line: usize,
}

/// A name found only inside a `when` block that does not apply here. `active` is committed
/// and shared, so this host does not edit the arm another host runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotHere {
    pub name: String,
    pub predicate: String,
    pub line: usize,
}

/// The result of taking names out of an `active` body: the new text, and everything the
/// command has to be able to say about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActiveEdit {
    pub body: String,
    pub removed: Vec<Removal>,
    pub emptied: Vec<BlockDrop>,
    pub elsewhere: Vec<NotHere>,
    pub absent: Vec<String>,
}

impl ActiveEdit {
    pub fn changed(&self) -> bool {
        !self.removed.is_empty()
    }
}

/// `deactivate NAME…` (II.6): take each name out of the top level **and out of every `when`
/// block that applies to this host**, dropping a block the removal empties.
///
/// Removing the top-level line and leaving the name switched on by a block two lines down
/// would report a state the machine did not reach — the verb has to mean what it says. A
/// block that does not apply here is not activating anything, so there is nothing in it to
/// deactivate and it is left alone.
pub fn remove_from_active(
    file: &std::path::Path,
    body: &str,
    names: &[String],
    facts: &HostFacts,
) -> Result<ActiveEdit> {
    // Parse first: a malformed `active` must fail before anything is rewritten.
    read_active(file, body, facts)?;

    let bare = |raw: &str| -> String {
        crate::config::grammar::strip_comment(raw).trim().to_string()
    };

    struct Open {
        header: String,
        line: usize,
        predicate: String,
        on: bool,
        kept: Vec<String>,
        had_names: bool,
        keeps_a_name: bool,
    }

    let mut edit = ActiveEdit::default();
    let mut out: Vec<String> = Vec::new();
    let mut open: Option<Open> = None;

    for (idx, raw) in body.lines().enumerate() {
        let line_no = idx + 1;
        let text = bare(raw);

        if let Some(header) = text.strip_suffix('{') {
            let predicate = header.trim().strip_prefix("when ").unwrap_or("").trim();
            open = Some(Open {
                header: raw.to_string(),
                line: line_no,
                predicate: predicate.to_string(),
                on: crate::config::parser::eval_when(predicate, facts).unwrap_or(false),
                kept: Vec::new(),
                had_names: false,
                keeps_a_name: false,
            });
            continue;
        }

        if text == "}" {
            let Some(b) = open.take() else { continue };
            // A block whose last name just left is not left behind empty — an empty `when`
            // is a rule about nothing.
            if b.had_names && !b.keeps_a_name {
                edit.emptied.push(BlockDrop {
                    predicate: b.predicate,
                    line: b.line,
                });
                continue;
            }
            out.push(b.header);
            out.extend(b.kept);
            out.push(raw.to_string());
            continue;
        }

        let is_name = !text.is_empty();
        let targeted = names.iter().any(|n| n == &text);

        match &mut open {
            Some(b) => {
                if is_name {
                    b.had_names = true;
                }
                if targeted && b.on {
                    edit.removed.push(Removal {
                        name: text,
                        line: line_no,
                        gate: Some(b.predicate.clone()),
                    });
                    continue;
                }
                if targeted {
                    edit.elsewhere.push(NotHere {
                        name: text,
                        predicate: b.predicate.clone(),
                        line: line_no,
                    });
                }
                if is_name {
                    b.keeps_a_name = true;
                }
                b.kept.push(raw.to_string());
            }
            None => {
                if targeted {
                    edit.removed.push(Removal {
                        name: text,
                        line: line_no,
                        gate: None,
                    });
                    continue;
                }
                out.push(raw.to_string());
            }
        }
    }

    for name in names {
        let seen = edit.removed.iter().any(|r| &r.name == name)
            || edit.elsewhere.iter().any(|e| &e.name == name);
        if !seen {
            edit.absent.push(name.clone());
        }
    }

    edit.body = out.join("\n");
    if !edit.body.is_empty() {
        edit.body.push('\n');
    }
    Ok(edit)
}

/// Every `when` block in an `active` body, for `activate` to name as it overwrites them.
///
/// The set form sets, blocks included (II.6) — but automatic and silent are different things,
/// so it has to be able to say which blocks went.
pub fn blocks_in_active(body: &str) -> Vec<BlockDrop> {
    let mut out = Vec::new();
    for (idx, raw) in body.lines().enumerate() {
        let text = crate::config::grammar::strip_comment(raw).trim();
        if let Some(header) = text.strip_suffix('{') {
            if let Some(pred) = header.trim().strip_prefix("when ") {
                out.push(BlockDrop {
                    predicate: pred.trim().to_string(),
                    line: idx + 1,
                });
            }
        }
    }
    out
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
            vars: Default::default(),
        }
    }

    fn facts_with(pairs: &[(&str, &str)]) -> HostFacts {
        let mut f = facts();
        for (k, v) in pairs {
            f.vars
                .insert(k.to_string(), crate::model::vars::Value::parse_literal(v));
        }
        f
    }

    #[test]
    fn a_variable_block_in_active_is_read_against_this_runs_variables() {
        // W8. The editing verbs used to read `active` against detected facts only, so this
        // file was not "a block that does not apply" — it was an unknown key, and every one
        // of them refused a correct file.
        let body = "when $role == travel {\n  Trip\n}\nWork\n";
        let on = parse_active(
            &PathBuf::from("active"),
            body,
            &facts_with(&[("role", "travel")]),
        )
        .unwrap();
        assert_eq!(on, ["Trip", "Work"]);

        let off = parse_active(
            &PathBuf::from("active"),
            body,
            &facts_with(&[("role", "desktop")]),
        )
        .unwrap();
        assert_eq!(off, ["Work"]);
    }

    #[test]
    fn a_message_about_a_variable_block_says_what_the_variable_is() {
        // The point of W8's messaging: `active` says the condition, and the file the reader
        // would open next does not contain the value.
        let facts = facts_with(&[("role", "desktop")]);
        assert_eq!(
            describe_gate("$role == travel", &facts),
            "`when $role == travel` ($role is desktop)"
        );
        // No variable, nothing to add: the condition is already the whole answer.
        assert_eq!(
            describe_gate("host == laptop", &facts),
            "`when host == laptop`"
        );
    }

    #[test]
    fn a_message_names_an_undefined_variable_rather_than_going_quiet() {
        assert_eq!(
            describe_gate("$role == travel", &facts()),
            "`when $role == travel` ($role is undefined)"
        );
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
            &Vec::new(),
        )
    }

    fn module_names(r: &Resolved) -> Vec<&str> {
        r.modules.iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn a_profile_chooses_modules() {
        let f = fixture(&[("Work", "use editors\nuse dev\n")], &[]);
        assert_eq!(module_names(&resolve(&f, "Work").unwrap()), ["editors", "dev"]);
    }

    #[test]
    fn a_profile_may_hold_package_lines_directly() {
        // II.4/V.3, accepted knowingly: `--into Work` is a real want, and the cost is that
        // a module can never reach these.
        let f = fixture(&[("Work", "use editors\napt:slack\n")], &[]);
        let r = resolve(&f, "Work").unwrap();
        assert_eq!(module_names(&r), ["editors"]);
        assert_eq!(r.direct.len(), 1);
    }

    #[test]
    fn a_profile_may_reference_a_profile() {
        // II.7 step 2. The opposite direction is the one that is forbidden.
        let f = fixture(
            &[("Work", "use Base\nuse dev\n"), ("Base", "use editors\n")],
            &[],
        );
        assert_eq!(module_names(&resolve(&f, "Work").unwrap()), ["editors", "dev"]);
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
        let out = parse_active(&PathBuf::from("active"), "# on now\nWork\nGaming\n", &facts()).unwrap();
        assert_eq!(out, ["Work", "Gaming"]);
    }

    #[test]
    fn active_refuses_a_module_name() {
        // Only profiles can be activated (II.4).
        let err = parse_active(&PathBuf::from("active"), "editors\n", &facts()).unwrap_err();
        assert!(err.hint.unwrap().contains("Only profiles can be activated"));
    }

    #[test]
    fn active_refuses_anything_that_is_not_a_name() {
        assert!(parse_active(&PathBuf::from("active"), "Work | Gaming\n", &facts()).is_err());
    }

    #[test]
    fn active_ignores_a_repeat() {
        let out = parse_active(&PathBuf::from("active"), "Work\nWork\n", &facts()).unwrap();
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
            vars: Default::default(),
        }
    }

    fn read(body: &str, host: &str) -> Result<Vec<ActiveEntry>> {
        read_active(&PathBuf::from("active"), body, &facts(host))
    }

    fn on(body: &str, host: &str) -> Vec<String> {
        read(body, host)
            .unwrap()
            .into_iter()
            .filter(|e| e.on)
            .map(|e| e.name)
            .collect()
    }

    #[test]
    fn a_when_block_in_active_can_test_a_variable() {
        // W8: `active` is the most useful place for a variable, and it used to fail with
        // "unknown when key '$role'" because it detected its own varless facts.
        let mut f = facts("anyhost");
        f.vars.insert("role".into(), crate::model::vars::Value::Str("travel".into()));
        let body = "when $role == travel {\n  Travel\n}\nWork\n";
        let names: Vec<String> = read_active(&PathBuf::from("active"), body, &f)
            .unwrap()
            .into_iter()
            .filter(|e| e.on)
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["Travel".to_string(), "Work".to_string()]);

        f.vars.insert("role".into(), crate::model::vars::Value::Str("desktop".into()));
        let names: Vec<String> = read_active(&PathBuf::from("active"), body, &f)
            .unwrap()
            .into_iter()
            .filter(|e| e.on)
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["Work".to_string()], "Travel is off when the variable does not match");
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

    fn deactivate(body: &str, host: &str, names: &[&str]) -> ActiveEdit {
        let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        remove_from_active(&PathBuf::from("active"), body, &names, &facts(host)).unwrap()
    }

    #[test]
    fn deactivate_takes_a_top_level_name_out() {
        let e = deactivate("Work\nGaming\n", "laptop", &["Work"]);
        assert_eq!(e.body, "Gaming\n");
        assert_eq!(e.removed.len(), 1);
        assert_eq!(e.removed[0].gate, None);
    }

    /// The old rule — top-level lines only — left the name switched on by a block two lines
    /// down and reported a state the machine had not reached. Reversed by owner 2026-07-17.
    #[test]
    fn deactivate_reaches_into_a_block_that_applies_here() {
        let e = deactivate(
            "Work\nwhen host == laptop {\n  Travel\n}\n",
            "laptop",
            &["Travel"],
        );
        assert_eq!(e.removed.len(), 1);
        assert_eq!(e.removed[0].gate.as_deref(), Some("host == laptop"));
        // The block held nothing else, so it goes too, and it says so.
        assert_eq!(e.emptied.len(), 1);
        assert_eq!(e.emptied[0].line, 2);
        assert_eq!(e.body, "Work\n");
        assert!(e.elsewhere.is_empty());
    }

    #[test]
    fn a_block_with_another_name_left_in_it_survives() {
        let e = deactivate(
            "when host == laptop {\n  Travel\n  Work\n}\n",
            "laptop",
            &["Travel"],
        );
        assert!(e.emptied.is_empty());
        assert_eq!(e.body, "when host == laptop {\n  Work\n}\n");
    }

    /// `active` is committed and shared. On the desktop, `when host == laptop { Travel }` is
    /// activating nothing — so there is nothing there to deactivate, and editing it would
    /// change a machine nobody is sitting at.
    #[test]
    fn a_block_for_another_host_is_never_touched() {
        let e = deactivate(
            "when host == laptop {\n  Travel\n}\n",
            "desktop",
            &["Travel"],
        );
        assert!(e.removed.is_empty());
        assert!(!e.changed());
        assert_eq!(e.elsewhere.len(), 1);
        assert_eq!(e.elsewhere[0].predicate, "host == laptop");
        assert_eq!(e.elsewhere[0].line, 2);
        assert_eq!(e.body, "when host == laptop {\n  Travel\n}\n");
    }

    #[test]
    fn deactivating_a_name_that_is_not_there_changes_nothing_and_says_so() {
        let e = deactivate("Work\n", "laptop", &["Gaming"]);
        assert_eq!(e.absent, ["Gaming"]);
        assert!(!e.changed());
    }

    #[test]
    fn activate_can_name_every_block_it_overwrites() {
        let blocks = blocks_in_active("Work\nwhen host == laptop {\n  Travel\n}\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].predicate, "host == laptop");
        assert_eq!(blocks[0].line, 2);
    }

    #[test]
    fn active_holds_names_never_expressions() {
        // II.6: the set math lives inside profiles. `active` stays a list you can read at
        // a glance, because it is the one file you open to know what is on.
        assert!(read("(Work | Gaming)\n", "any").is_err());
    }
}
