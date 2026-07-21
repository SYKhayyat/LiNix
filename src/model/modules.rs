use super::layout::{Layout, ModuleName};
use crate::config::grammar::{
    parse_document, BackendNames, Document, Gates, GrammarError, Origin, Reference, Result,
    Statement,
};
use crate::config::parser::HostFacts;
use std::collections::HashMap;

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
    fn reject_profile_references(&self, doc: &Document) -> Result<()> {
        for (stmt, origin) in flatten(doc) {
            if let Statement::Use(Reference::Profile(p)) = stmt {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("a module cannot `use` the profile `{}`", p),
                )
                .with_hint(
                    "profiles choose; modules hold. If a module could name a profile, what a \
                     module contains would depend on what you activated.",
                ));
            }
            // II.3: a module is a list. `-` subtraction does not exist in one; `absent:`
            // does. Choosing is the profile's job (II.4, V.2), and set math is choosing.
            let what = match stmt {
                Statement::Exclude(_) => Some("exclude"),
                Statement::Intersect(_) => Some("intersect"),
                Statement::Subtract(_) => Some("`-` subtraction"),
                Statement::Expr(_) => Some("a set expression"),
                _ => None,
            };
            if let Some(what) = what {
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
    seen: &mut Vec<String>,
    inherited: &Gates,
) -> Result<Vec<(Statement, Origin, Gates)>> {
    let key = name.to_lowercase();
    if seen.contains(&key) {
        return Err(GrammarError::new(
            asked_by.clone(),
            format!(
                "module `{}` ends up using itself: {} -> {}",
                name,
                seen.join(" -> "),
                key
            ),
        )
        .with_hint("a module cannot use itself, directly or through another."));
    }
    seen.push(key.clone());

    let stmts = loader.load(&key, asked_by)?.statements_with_gating(facts)?;

    let mut out = Vec::new();
    for (stmt, origin, own) in stmts {
        let mut gates = inherited.clone();
        gates.extend(own);
        match stmt {
            Statement::Use(Reference::Module(m)) => {
                out.extend(expand(loader, &m, &origin, facts, seen, &gates)?);
            }
            // Rejected at load time; unreachable, but not worth an unwrap.
            Statement::Use(Reference::Profile(_)) => continue,
            other => out.push((other, origin, gates)),
        }
    }
    seen.pop();
    Ok(out)
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
        // II.3: the folder decides.
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
        let f = fixture(&[("a.txt", "use b\n"), ("b.txt", "use a\n")]);
        let err = expand_module(&f, "a").unwrap_err();
        assert!(err.what.contains("ends up using itself"), "{}", err);
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
}
