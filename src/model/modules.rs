use super::cycle::{self, Hop, Visit};
use super::layout::{Layout, ModuleName};
use crate::config::grammar::{
    parse_document, BackendNames, Document, Gates, GrammarError, Origin, Reference, Result,
    Statement,
};
use crate::config::parser::HostFacts;
use std::collections::HashMap;

/// Names the set operation if `stmt` is one, and `None` if a module may hold it.
///
/// II.3: a module is a list. `-` subtraction does not exist in one; `absent:` does. Choosing
/// is the profile's job (II.4, V.2), and set math is choosing.
pub fn set_math_in_a_module(stmt: &Statement) -> Option<&'static str> {
    match stmt {
        Statement::Exclude(_) => Some("exclude"),
        Statement::Intersect(_) => Some("intersect"),
        Statement::Subtract(_) => Some("`-` subtraction"),
        Statement::Expr(_) => Some("a set expression"),
        _ => None,
    }
}

/// Loads modules on demand (SPEC II.3).
///
/// **LiNix only parses what the active profiles reach.** Not an optimisation: the old
/// resolver seeded every `.txt` in the folder unconditionally, which is why `group:editors`
/// was already a no-op before anyone deleted it — the file was loaded before you named it,
/// so it looked like opt-in and was not (V.4). Nothing is active unless a profile names it.
pub struct ModuleLoader<'a> {
    layout: &'a Layout,
    backends: &'a dyn BackendNames,
    cache: HashMap<String, Document>,
}

impl<'a> ModuleLoader<'a> {
    pub fn new(layout: &'a Layout, backends: &'a dyn BackendNames) -> Self {
        Self {
            layout,
            backends,
            cache: HashMap::new(),
        }
    }

    /// Load a module by name, parsing it the first time it is reached.
    pub fn load(&mut self, name: &str, asked_by: &Origin) -> Result<&Document> {
        let key = name.to_lowercase();
        if !self.cache.contains_key(&key) {
            let doc = self.read(&key, asked_by)?;
            self.cache.insert(key.clone(), doc);
        }
        Ok(&self.cache[&key])
    }

    fn read(&self, name: &str, asked_by: &Origin) -> Result<Document> {
        let module = ModuleName::new(name).map_err(|e| GrammarError::new(asked_by.clone(), e))?;
        let path = self.layout.module_file(&module);
        let body = std::fs::read_to_string(&path).map_err(|_| {
            GrammarError::new(asked_by.clone(), format!("no module named `{}`", name))
                .with_hint(self.suggest(name))
        })?;
        let doc = parse_document(&path, &body, self.backends)?;
        self.reject_profile_references(&doc)?;
        Ok(doc)
    }

    /// **A module can `use` other modules. A module can NEVER reference a profile** (II.3).
    ///
    /// The layering rule. Without it, "what does `editors` contain?" has a different answer
    /// depending on what you activated — the library cannot depend on the app (V.2).
    ///
    /// The per-statement half is [`set_math_in_a_module`], shared with the editor: what may
    /// not be read out of a module file may not be written into one either, and two copies of
    /// that rule is how a writer comes to produce files the reader refuses.
    fn reject_profile_references(&self, doc: &Document) -> Result<()> {
        for (stmt, origin) in flatten(doc) {
            if let Statement::Use(Reference::Profile(p), _) = stmt {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("a module cannot `use` the profile `{}`", p),
                )
                .with_hint(
                    "profiles choose; modules hold. If a module could name a profile, what a \
                     module contains would depend on what you activated.",
                ));
            }
            if let Some(what) = set_math_in_a_module(stmt) {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("a module cannot use {}", what),
                )
                .with_hint(
                    "a module is a list of what it holds; set math is how a profile chooses \
                     between them. To say something must NOT exist, write `absent:apt:foo`.",
                ));
            }
        }
        Ok(())
    }

    /// II.5's error: it must teach the rule, not just say no.
    fn suggest(&self, missing: &str) -> String {
        let mut names = self.available();
        names.sort();
        if let Some(hit) = names.iter().find(|n| n.eq_ignore_ascii_case(missing)) {
            return format!(
                "did you mean the module `{}`? Profiles are Capitalized, modules are lowercase.",
                hit
            );
        }
        if names.is_empty() {
            return format!(
                "`modules/{}.txt` does not exist, and `modules/` holds no modules yet.",
                missing
            );
        }
        format!(
            "`modules/{}.txt` does not exist. Modules on this machine: {}.",
            missing,
            names.join(", ")
        )
    }

    /// Every module the folder holds. **`modules/*.txt`. The folder decides** — anything
    /// else is silently ignored, so a `README.md` costs nothing (II.3).
    pub fn available(&self) -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(self.layout.modules_dir()) else {
            return Vec::new();
        };
        let mut out: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".txt").map(str::to_lowercase)
            })
            .collect();
        out.sort();
        out
    }
}

/// Every statement in a document, ignoring `when` (which needs host facts). Used for the
/// structural checks that must hold regardless of which host reads the file — a module
/// referencing a profile is wrong on every machine, not just this one.
pub fn flatten(doc: &Document) -> Vec<(&Statement, &Origin)> {
    fn walk<'a>(
        items: &'a [crate::config::grammar::Item],
        out: &mut Vec<(&'a Statement, &'a Origin)>,
    ) {
        use crate::config::grammar::{Block, Item};
        for item in items {
            match item {
                Item::Statement(s, o) => out.push((s, o)),
                Item::Block(Block::Module(_, body), _) | Item::Block(Block::When(_, body), _) => {
                    walk(body, out)
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.items, &mut out);
    out
}

/// The statements a module contributes on this host, following `use` into other modules.
///
/// Cycles are stopped by `seen`: `a` using `b` using `a` is a mistake, not a reason to hang.
///
/// `inherited` is what already gated the reader's way in — the `active` block that turned the
/// profile on, the profile's `when` around its `use`. A module's own blocks append to it, so a
/// statement carries the whole chain that admitted it and not just its last link (W11).
pub fn expand<'a>(
    loader: &mut ModuleLoader<'a>,
    name: &str,
    asked_by: &Origin,
    facts: &HostFacts,
    seen: &mut Vec<Visit>,
    inherited: &Gates,
) -> Result<Vec<(Statement, Origin, Gates)>> {
    expand_args(loader, name, asked_by, facts, seen, inherited, &[])
}

/// [`expand`], with call-site arguments binding this module's `param`s (U32).
///
/// The ordinary `use editors` reaches this with `args` empty and behaves exactly as before — no
/// params, no substitution, nothing to bind. `use workstation(user=shaul)` reaches it with the
/// bindings, which become a scope every statement's `$param` references are substituted against,
/// **leaving unknown `$refs` intact** so the later global `vars` pass still resolves them (and
/// still errors on a real typo). A required `param` the call omits is a loud error naming the
/// module and the parameter (V.78) — never a silent empty string.
#[allow(clippy::too_many_arguments)]
pub fn expand_args<'a>(
    loader: &mut ModuleLoader<'a>,
    name: &str,
    asked_by: &Origin,
    facts: &HostFacts,
    seen: &mut Vec<Visit>,
    inherited: &Gates,
    args: &[(String, String)],
) -> Result<Vec<(Statement, Origin, Gates)>> {
    let key = name.to_lowercase();
    let entered = Hop::new(asked_by.clone(), format!("use {}", name));
    if let Some(start) = seen.iter().position(|v| v.key == key) {
        // The loop only — what led *into* it is not part of it.
        let mut hops: Vec<Hop> = seen[start + 1..]
            .iter()
            .map(|v| v.entered.clone())
            .collect();
        hops.push(entered);
        return Err(GrammarError::new(
            asked_by.clone(),
            cycle::describe("modules use each other in a loop", &hops, &key),
        )
        .with_hint("a module cannot use itself, directly or through another."));
    }
    seen.push(Visit {
        key: key.clone(),
        entered,
    });

    // Bind params from the *raw* document (ungated), then flatten with the params merged into
    // the facts — so a `when $gpu == nvidia` inside the module sees the bound parameter, not just
    // a `link:` value does (XIII.29). Both are done while the doc is borrowed; `stmts` and
    // `scope` are owned so the borrow ends before the recursive `expand_args` below.
    let (scope, stmts) = {
        let doc = loader.load(&key, asked_by)?;
        let scope = bind_params(doc, args, &key, asked_by)?;
        let mut augmented = facts.clone();
        for (k, v) in &scope {
            augmented
                .vars
                .insert(k.clone(), crate::model::vars::Value::parse_literal(v));
        }
        let stmts = doc.statements_with_gating(&augmented)?;
        (scope, stmts)
    };

    let mut out = Vec::new();
    for (stmt, origin, own) in stmts {
        let mut gates = inherited.clone();
        gates.extend(own);
        match stmt {
            // A `param` is consumed here: it declared what this module takes, and binding it is
            // this function's job. It never reaches the resolved statement stream.
            Statement::Param { .. } => continue,
            Statement::Use(Reference::Module(m), child_args) => {
                // A nested `use inner(x=$user)` may reference this module's own parameters, so
                // its argument values are substituted before the inner module binds them.
                let child_args = substitute_args(&child_args, &scope);
                out.extend(expand_args(
                    loader,
                    &m,
                    &origin,
                    facts,
                    seen,
                    &gates,
                    &child_args,
                )?);
            }
            // Rejected at load time; unreachable, but not worth an unwrap.
            Statement::Use(Reference::Profile(_), _) => continue,
            mut other => {
                if !scope.is_empty() {
                    substitute_in_statement(&mut other, &scope);
                }
                out.push((other, origin, gates));
            }
        }
    }
    seen.pop();
    Ok(out)
}

/// Bind a module's declared `param`s against the call-site `args` (U32). A `param` with a
/// default falls back to it; a required `param` the call omits is an error, and an argument that
/// names no `param` is an error too — a closed vocabulary names its typos rather than binding
/// them to nothing (VIII.2).
fn bind_params(
    doc: &Document,
    args: &[(String, String)],
    module: &str,
    asked_by: &Origin,
) -> Result<std::collections::HashMap<String, String>> {
    // Ungated, so a `param` is found regardless of any `when` around it — and a param must be
    // bound before `when` is evaluated, since a param may be what a `when` tests.
    let params: Vec<(&str, &Option<String>)> = doc
        .every_statement()
        .into_iter()
        .filter_map(|(s, _, _)| match s {
            Statement::Param { name, default } => Some((name.as_str(), default)),
            _ => None,
        })
        .collect();

    // An argument that names no parameter is a typo, caught rather than dropped.
    for (k, _) in args {
        if !params.iter().any(|(name, _)| name == k) {
            return Err(GrammarError::new(
                asked_by.clone(),
                format!("module `{}` has no parameter `{}`", module, k),
            )
            .with_hint(if params.is_empty() {
                format!("`{}` declares no parameters.", module)
            } else {
                format!(
                    "`{}` takes: {}.",
                    module,
                    params
                        .iter()
                        .map(|(n, _)| *n)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }));
        }
    }

    let mut scope = std::collections::HashMap::new();
    for (name, default) in params {
        let value = args
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .or_else(|| default.clone());
        match value {
            Some(v) => {
                scope.insert(name.to_string(), v);
            }
            None => {
                return Err(GrammarError::new(
                    asked_by.clone(),
                    format!("module `{}` requires parameter `{}`", module, name),
                )
                .with_hint(format!(
                    "pass it: `use {}({}=…)`, or give the parameter a default: `param {} = …`.",
                    module, name, name
                )));
            }
        }
    }
    Ok(scope)
}

/// Replace `$param` in a value for every `param` in `scope`, leaving any other `$ref` verbatim.
///
/// This is deliberately *not* `vars::expand`: params are an inner scope resolved before the
/// global one, so an unknown `$ref` here is not an error — it is a global variable the later pass
/// will resolve (or a real typo the later pass will name). `$$` is a literal `$`.
fn substitute_params(value: &str, scope: &std::collections::HashMap<String, String>) -> String {
    if scope.is_empty() || !value.contains('$') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut chars = value.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        // `$$` → a literal `$`.
        if matches!(chars.peek(), Some((_, '$'))) {
            out.push('$');
            chars.next();
            continue;
        }
        let rest = &value[i + 1..];
        let ident: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        match scope.get(&ident) {
            Some(v) if !ident.is_empty() => {
                out.push_str(v);
                for _ in 0..ident.chars().count() {
                    chars.next();
                }
            }
            // Not a param (unknown, or `$` not followed by an identifier): leave it for the
            // global vars pass, verbatim.
            _ => out.push('$'),
        }
    }
    out
}

/// Substitute params into the values of a nested `use`'s arguments (`use inner(x=$user)`).
fn substitute_args(
    args: &[(String, String)],
    scope: &std::collections::HashMap<String, String>,
) -> Vec<(String, String)> {
    args.iter()
        .map(|(k, v)| (k.clone(), substitute_params(v, scope)))
        .collect()
}

/// Substitute params into every interpolatable field of a statement — the same fields the global
/// `vars` pass touches, so the two scopes reach exactly the same places (V.62).
fn substitute_in_statement(
    stmt: &mut Statement,
    scope: &std::collections::HashMap<String, String>,
) {
    let sub = |s: &mut String| *s = substitute_params(s, scope);
    match stmt {
        Statement::Package(d) | Statement::Absent(d) => {
            for value in d.options.values_mut() {
                sub(value);
            }
        }
        Statement::Shim(name, opts)
        | Statement::Service(name, opts)
        | Statement::Link(name, opts)
        | Statement::Setting(name, opts)
        | Statement::Exec(name, opts)
        | Statement::Generate(name, opts)
        | Statement::Dotfiles(name, opts)
        | Statement::Firewall(name, opts) => {
            sub(name);
            for value in opts.values_mut() {
                sub(value);
            }
        }
        Statement::Repo { spec, .. } => sub(spec),
        // A schedule's `run` is a shell command where `$` is the shell's; set math and `use`
        // name files, not values; a param never reaches here.
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

#[cfg(test)]
mod tests {
    use super::*;
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

    struct Fixture {
        _tmp: TempDir,
        layout: Layout,
    }

    fn fixture(files: &[(&str, &str)]) -> Fixture {
        let tmp = TempDir::new().unwrap();
        let layout = Layout::new(tmp.path().join("cfg"), tmp.path().join("data"));
        std::fs::create_dir_all(layout.modules_dir()).unwrap();
        for (name, body) in files {
            std::fs::write(layout.modules_dir().join(name), body).unwrap();
        }
        Fixture { _tmp: tmp, layout }
    }

    fn expand_module(f: &Fixture, name: &str) -> Result<Vec<Statement>> {
        let mut loader = ModuleLoader::new(&f.layout, &known);
        let out = expand(
            &mut loader,
            name,
            &Origin::argument(),
            &facts(),
            &mut Vec::new(),
            &Vec::new(),
        )?;
        Ok(out.into_iter().map(|(s, _, _)| s).collect())
    }

    #[test]
    fn a_module_is_a_list_of_lines() {
        let f = fixture(&[("editors.txt", "apt:neovim\napt:vim\n")]);
        assert_eq!(expand_module(&f, "editors").unwrap().len(), 2);
    }

    #[test]
    fn the_filename_is_the_module_name_lowercased() {
        // II.3: `Editors.txt` -> module `editors`. A filename can never mint a profile.
        let f = fixture(&[("editors.txt", "apt:neovim\n")]);
        assert!(expand_module(&f, "Editors").is_ok());
    }

    #[test]
    fn a_module_can_use_another_module() {
        let f = fixture(&[
            ("dev.txt", "use editors\ncargo:ripgrep\n"),
            ("editors.txt", "apt:neovim\n"),
        ]);
        assert_eq!(expand_module(&f, "dev").unwrap().len(), 2);
    }

    #[test]
    fn a_module_can_never_reference_a_profile() {
        // The layering rule (II.3, V.2). Otherwise "what does `editors` contain?" depends
        // on what you activated.
        let f = fixture(&[("editors.txt", "use Work\n")]);
        let err = expand_module(&f, "editors").unwrap_err();
        assert!(err.what.contains("cannot `use` the profile"), "{}", err);
        assert!(err.hint.unwrap().contains("profiles choose; modules hold"));
    }

    #[test]
    fn only_txt_files_are_modules_so_a_readme_costs_nothing() {
        // The folder decides what is a module, not the file's contents.
        let f = fixture(&[
            ("editors.txt", "apt:neovim\n"),
            ("README.md", "# these are my modules\n"),
        ]);
        let loader = ModuleLoader::new(&f.layout, &known);
        assert_eq!(loader.available(), ["editors"]);
    }

    #[test]
    fn a_missing_module_names_what_is_available() {
        let f = fixture(&[("editors.txt", "apt:neovim\n")]);
        let err = expand_module(&f, "editrs").unwrap_err();
        assert!(err.what.contains("no module named `editrs`"), "{}", err);
        assert!(err.hint.unwrap().contains("editors"));
    }

    #[test]
    fn a_wrong_case_reference_teaches_the_rule() {
        // II.5: "no profile named `Editors` — did you mean the module `editors`?"
        let f = fixture(&[("editors.txt", "apt:neovim\n")]);
        let mut loader = ModuleLoader::new(&f.layout, &known);
        let err = loader
            .load("EDITORS_TYPO", &Origin::argument())
            .unwrap_err();
        assert!(err.hint.is_some());
    }

    #[test]
    fn a_module_that_uses_itself_is_an_error_not_a_hang() {
        // II.7: the error names every file and line in the loop, in order, and stops.
        let f = fixture(&[("a.txt", "use b\n"), ("b.txt", "use a\n")]);
        let err = expand_module(&f, "a").unwrap_err();
        assert!(
            err.what.contains("modules use each other in a loop"),
            "{}",
            err
        );
        assert!(err.what.contains("a.txt:1  use b"), "{}", err);
        assert!(err.what.contains("b.txt:1  use a"), "{}", err);
        assert!(err.what.trim_end().ends_with("^ back to a"), "{}", err);
    }

    #[test]
    fn a_diamond_is_not_a_loop() {
        // II.7: reaching a module twice by two routes is what modules are for. Only a path
        // that returns to where it started is a loop — so the walk is a path, not a set.
        let f = fixture(&[
            ("top.txt", "use left\nuse right\n"),
            ("left.txt", "use base\n"),
            ("right.txt", "use base\n"),
            ("base.txt", "apt:curl\n"),
        ]);
        assert_eq!(expand_module(&f, "top").unwrap().len(), 2);
    }

    #[test]
    fn only_what_is_reached_is_parsed() {
        // II.3. `broken.txt` would fail to parse, but nothing reaches it, so nothing looks.
        let f = fixture(&[
            ("editors.txt", "apt:neovim\n"),
            ("broken.txt", "this is not a statement at all\n"),
        ]);
        assert!(expand_module(&f, "editors").is_ok());
    }

    #[test]
    fn when_gates_lines_inside_a_module() {
        let f = fixture(&[(
            "dev.txt",
            "apt:neovim\nwhen os == windows {\n  apt:notepad\n}\n",
        )]);
        assert_eq!(expand_module(&f, "dev").unwrap().len(), 1);
    }

    // --- U32: module parameters ---

    fn expand_with_args(f: &Fixture, name: &str, args: &[(&str, &str)]) -> Result<Vec<Statement>> {
        let args: Vec<(String, String)> = args
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let mut loader = ModuleLoader::new(&f.layout, &known);
        let out = expand_args(
            &mut loader,
            name,
            &Origin::argument(),
            &facts(),
            &mut Vec::new(),
            &Vec::new(),
            &args,
        )?;
        Ok(out.into_iter().map(|(s, _, _)| s).collect())
    }

    fn link_target(stmt: &Statement) -> Option<&str> {
        match stmt {
            Statement::Link(_, opts) => opts.one("target"),
            _ => None,
        }
    }

    #[test]
    fn a_param_is_substituted_from_a_use_argument() {
        // U32: the flagship example. `$user` in the module body becomes the passed value, and
        // the `param` line itself never reaches the output.
        let f = fixture(&[(
            "workstation.txt",
            "param user\nlink:./gitconfig@target=/home/$user/.gitconfig\n",
        )]);
        let stmts = expand_with_args(&f, "workstation", &[("user", "shaul")]).unwrap();
        assert_eq!(stmts.len(), 1, "the param line is consumed: {:?}", stmts);
        assert_eq!(link_target(&stmts[0]), Some("/home/shaul/.gitconfig"));
    }

    #[test]
    fn a_param_default_is_used_when_no_argument_is_passed() {
        let f = fixture(&[(
            "workstation.txt",
            "param dir = /opt\nlink:./x@target=$dir/x\n",
        )]);
        let stmts = expand_with_args(&f, "workstation", &[]).unwrap();
        assert_eq!(link_target(&stmts[0]), Some("/opt/x"));
    }

    #[test]
    fn a_required_param_with_no_argument_is_a_loud_error() {
        // Never a silent empty string.
        let f = fixture(&[("workstation.txt", "param user\napt:vim\n")]);
        let err = expand_with_args(&f, "workstation", &[]).unwrap_err();
        assert!(err.what.contains("requires parameter `user`"), "{}", err);
        assert!(err.what.contains("workstation"), "{}", err);
    }

    #[test]
    fn an_argument_that_names_no_parameter_is_an_error() {
        let f = fixture(&[("workstation.txt", "param user\napt:vim\n")]);
        let err = expand_with_args(&f, "workstation", &[("gpu", "nvidia")]).unwrap_err();
        assert!(err.what.contains("has no parameter `gpu`"), "{}", err);
    }

    #[test]
    fn a_param_gates_a_when_block() {
        // `$gpu` reaches a `when` — the same variable machinery, one scope wider.
        let f = fixture(&[(
            "workstation.txt",
            "param gpu = none\nwhen $gpu == nvidia {\n  apt:nvidia-driver\n}\n",
        )]);
        assert!(expand_with_args(&f, "workstation", &[]).unwrap().is_empty());
        let on = expand_with_args(&f, "workstation", &[("gpu", "nvidia")]).unwrap();
        assert_eq!(on.len(), 1);
    }

    #[test]
    fn an_unknown_dollar_reference_is_left_for_the_global_vars_pass() {
        // A param scope substitutes only its own names; `$notaparam` survives verbatim so the
        // later global `vars` pass resolves it (or names it as a typo). It must not error here.
        let f = fixture(&[(
            "m.txt",
            "param user\nlink:./x@target=/home/$user/$notaparam\n",
        )]);
        let stmts = expand_with_args(&f, "m", &[("user", "a")]).unwrap();
        assert_eq!(link_target(&stmts[0]), Some("/home/a/$notaparam"));
    }

    #[test]
    fn a_nested_use_receives_substituted_arguments() {
        // `use inner(who=$user)` inside a parameterized module: the outer param is substituted
        // into the inner call's argument before the inner module binds it.
        let f = fixture(&[
            ("outer.txt", "param user\nuse inner(who=$user)\n"),
            ("inner.txt", "param who\nlink:./x@target=/home/$who/x\n"),
        ]);
        let stmts = expand_with_args(&f, "outer", &[("user", "shaul")]).unwrap();
        assert_eq!(link_target(&stmts[0]), Some("/home/shaul/x"));
    }

    #[test]
    fn a_plain_use_of_a_parameterless_module_is_unchanged() {
        // The common case: no params, no args, nothing substituted.
        let f = fixture(&[("editors.txt", "apt:neovim\napt:vim\n")]);
        assert_eq!(expand_module(&f, "editors").unwrap().len(), 2);
    }
}
