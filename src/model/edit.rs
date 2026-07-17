use super::layout::Layout;
use super::modules::ModuleLoader;
use super::profiles::{parse_active, ProfileLoader};
use crate::config::grammar::{statement, BackendNames, GrammarError, Origin, Result, Statement};
use crate::config::parser::HostFacts;
use std::path::PathBuf;

/// Where a line goes (SPEC II.8's `--into`).
///
/// Case decides, everywhere: `Editors` is a profile, `editors` is a module (II.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Module(String),
    Profile(String),
}

impl Target {
    pub fn parse(name: &str, origin: &Origin) -> Result<Self> {
        match name.chars().next() {
            Some(c) if c.is_uppercase() => Ok(Target::Profile(name.to_string())),
            Some(c) if c.is_lowercase() => Ok(Target::Module(name.to_string())),
            _ => Err(GrammarError::new(
                origin.clone(),
                format!("`{}` is neither a module nor a profile", name),
            )
            .with_hint("profiles are Capitalized, modules are lowercase.")),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Target::Module(n) | Target::Profile(n) => n,
        }
    }

    fn file(&self, layout: &Layout) -> PathBuf {
        match self {
            Target::Module(n) => layout.module_file(n),
            Target::Profile(n) => layout.profile_file(n),
        }
    }
}

/// The three landing modules, named for how the package arrived (II.8).
///
/// Provenance ends up in the filename, so `modules/hooks.txt` is exactly what got in behind
/// LiNix's back. One `local.txt` mixed them and forgot which was which (V.40).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    Imperative,
    Hooks,
    Adopted,
}

impl Landing {
    pub fn module(self) -> &'static str {
        match self {
            Landing::Imperative => "imperative",
            Landing::Hooks => "hooks",
            Landing::Adopted => "adopted",
        }
    }

    pub fn target(self) -> Target {
        Target::Module(self.module().to_string())
    }

    /// Why this file exists, written into it the first time LiNix creates it.
    fn header(self) -> &'static str {
        match self {
            Landing::Imperative => {
                "# Packages that arrived via `linix install`.\n\
                 #\n\
                 # This is an ordinary module: read it, edit it, delete a line to uninstall.\n\
                 # LiNix writes here so an imperative command still ends up as a file you own.\n\n"
            }
            Landing::Hooks => {
                "# Packages that arrived behind LiNix's back — `apt install`, caught by the hook.\n\
                 #\n\
                 # This is an ordinary module: read it, edit it, delete a line to uninstall.\n\n"
            }
            Landing::Adopted => {
                "# Packages that arrived via `linix adopt` — what was already on this machine.\n\
                 #\n\
                 # This is an ordinary module: read it, edit it, delete a line to uninstall.\n\n"
            }
        }
    }
}

/// One file LiNix touched, for the sentence II.8 requires it to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub file: PathBuf,
    /// The line written or removed.
    pub line: String,
    /// The profile that gained `use <module>`, if this write made the module reachable.
    pub wired_into: Option<String>,
}

impl Edit {
    /// `Added jq to modules/imperative.txt (used by profile Work)` — II.8: every command
    /// prints the file it touched.
    pub fn describe(&self, verb: &str) -> String {
        let mut s = format!("{} {} in {}", verb, self.line, self.file.display());
        if let Some(p) = &self.wired_into {
            s.push_str(&format!("\n  Added `use {}` to profile {} — that module is now part \
                                 of this machine. It is a normal line you can read and delete.",
                self.module_name().unwrap_or_default(), p));
        }
        s
    }

    fn module_name(&self) -> Option<String> {
        self.file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    }
}

/// Edits your files: the other half of P1, where every imperative command is a shortcut for
/// editing a file and syncing.
pub struct Editor<'a> {
    layout: &'a Layout,
    backends: &'a dyn BackendNames,
    facts: HostFacts,
}

impl<'a> Editor<'a> {
    pub fn new(layout: &'a Layout, backends: &'a dyn BackendNames) -> Self {
        Self {
            layout,
            backends,
            facts: HostFacts::current(),
        }
    }

    pub fn with_facts(mut self, facts: HostFacts) -> Self {
        self.facts = facts;
        self
    }

    /// Write `line` into `target`, and make sure something reaches it.
    ///
    /// A line in a module nothing activates is a line that does nothing, so this refuses
    /// rather than write one: `install` that quietly installs nothing is the disease.
    pub fn add(&self, target: &Target, line: &str) -> Result<Edit> {
        // Decided before writing. Writing the line and then failing to wire it leaves the
        // file changed and the machine not converging, which is the worst of both.
        let wired_into = match target {
            Target::Module(m) => self.reachable_via(m)?,
            // A profile is reached by `active`, not by a `use` line somewhere else.
            Target::Profile(_) => None,
        };

        let path = target.file(self.layout);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let created = existing.is_empty();
        let body = self.replace_or_append(&existing, line, created, target);

        write(&path, &body)?;
        if let Some(p) = &wired_into {
            let pf = self.layout.profile_file(p);
            let mut b = std::fs::read_to_string(&pf).unwrap_or_default();
            if !b.is_empty() && !b.ends_with('\n') {
                b.push('\n');
            }
            b.push_str(&format!("use {}\n", target.name()));
            write(&pf, &b)?;
        }

        Ok(Edit {
            file: path,
            line: line.to_string(),
            wired_into,
        })
    }

    /// Replace a module's whole contents with generated text, and make sure something
    /// reaches it.
    ///
    /// For a module LiNix writes rather than edits — `adopted`, which II.9 says is **one**
    /// file. A timestamped file per run would make the second `adopt` declare everything
    /// twice, and two declarations of one package is a conflict the resolver then refuses
    /// (II.7 rule 5). Overwriting is also what makes it re-runnable: `adopt` again and the
    /// answer is the machine as it is now, not the machine plus history.
    pub fn write_module(&self, target: &Target, body: &str) -> Result<Edit> {
        let wired_into = match target {
            Target::Module(m) => self.reachable_via(m)?,
            Target::Profile(_) => None,
        };

        let path = target.file(self.layout);
        write(&path, body)?;
        if let Some(p) = &wired_into {
            let pf = self.layout.profile_file(p);
            let mut b = std::fs::read_to_string(&pf).unwrap_or_default();
            if !b.is_empty() && !b.ends_with('\n') {
                b.push('\n');
            }
            b.push_str(&format!("use {}\n", target.name()));
            write(&pf, &b)?;
        }

        Ok(Edit {
            file: path,
            line: format!("{} line(s)", body.lines().count()),
            wired_into,
        })
    }

    /// A second declaration of the same package would be a conflict the resolver then
    /// refuses (II.7 rule 5), so `install jq@version=2` must replace the `jq` line rather
    /// than sit next to it.
    fn replace_or_append(
        &self,
        existing: &str,
        line: &str,
        created: bool,
        target: &Target,
    ) -> String {
        let key = self.key_of(line);
        let mut out: Vec<String> = Vec::new();
        let mut replaced = false;

        for raw in existing.lines() {
            if key.is_some() && self.key_of(raw) == key {
                if !replaced {
                    out.push(line.to_string());
                    replaced = true;
                }
                continue;
            }
            out.push(raw.to_string());
        }

        if !replaced {
            if created {
                if let Target::Module(m) = target {
                    if let Some(l) = landing_of(m) {
                        out.push(l.header().trim_end().to_string());
                        out.push(String::new());
                    }
                }
            }
            out.push(line.to_string());
        }

        let mut body = out.join("\n");
        body.push('\n');
        body
    }

    /// `backend:name` for a package line, `None` for anything else.
    ///
    /// Parsed, never split on `:`. A parser that trusts the prefix reads the backend half of
    /// `apt:jq` as a name, which is how removing a package called `apt` came to delete every
    /// `apt:*` line (S9).
    fn key_of(&self, line: &str) -> Option<String> {
        let line = match line.find('#') {
            Some(i) => &line[..i],
            None => line,
        }
        .trim();
        if line.is_empty() {
            return None;
        }
        let stmt = statement::parse(&Origin::argument(), line, self.backends).ok()?;
        match &stmt {
            Statement::Package(d) => Some(format!(
                "{}:{}",
                d.backend.clone().unwrap_or_default(),
                d.selector.as_str()
            )),
            // `use` is not an identity to replace: a module may use many.
            Statement::Use(_) => None,
            other => other_key(other),
        }
    }

    /// Which profile must gain `use <module>` for this write to mean anything.
    ///
    /// `None` = already reached, nothing to do. `Some(p)` = add it to `p` and say so
    /// (II.8: a normal line you can read and delete — never implicit).
    fn reachable_via(&self, module: &str) -> Result<Option<String>> {
        let active_file = self.layout.active_file();
        let body = std::fs::read_to_string(&active_file).unwrap_or_default();
        let active = parse_active(&active_file, &body)?;

        if self.reaches(&active, module) {
            return Ok(None);
        }

        match active.as_slice() {
            // One active profile: no question which of your identities owns this.
            [only] => Ok(Some(only.clone())),

            // Nothing is active, so there is nowhere for this to be reached from. Writing
            // the line anyway would report success and change nothing.
            [] => Err(GrammarError::new(
                Origin::new(&active_file, 0),
                "nothing is active, so there is nowhere to put this.",
            )
            .with_hint(
                "activate a profile first (`linix activate Main`), or name one with \
                 `--into <Profile>`. A module no profile reaches is a module LiNix never reads.",
            )),

            // Several: `--into` is how II.8 already asks this question, so ask it rather
            // than pick one of your identities at random and never mention it.
            many => Err(GrammarError::new(
                Origin::new(&active_file, 0),
                format!(
                    "{} profiles are active ({}), so which one should own `{}`?",
                    many.len(),
                    many.join(", "),
                    module
                ),
            )
            .with_hint(format!(
                "say where it goes: `--into {}` puts it in that profile, `--into <module>` \
                 in a module. Only needed once — after that the `use {}` line is there.",
                many[0], module
            ))),
        }
    }

    /// Whether any active profile already reaches this module.
    fn reaches(&self, active: &[String], module: &str) -> bool {
        let profiles = ProfileLoader::new(self.layout, self.backends);
        let mut loader = ModuleLoader::new(self.layout, self.backends);
        let asked = Origin::new(self.layout.active_file(), 0);

        for name in active {
            let Ok(r) = profiles.resolve(name, &asked, &self.facts, &mut Vec::new()) else {
                continue;
            };
            for m in &r.modules {
                if m.eq_ignore_ascii_case(module) {
                    return true;
                }
                // A module reached through another module is reached.
                if let Ok(stmts) =
                    super::modules::expand(&mut loader, m, &asked, &self.facts, &mut Vec::new())
                {
                    let want = self.layout.module_file(module);
                    if stmts.iter().any(|(_, o)| o.file == want) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Whether any of `files` declares `target_pkg`.
    pub fn declares_in(&self, files: &[PathBuf], target_pkg: &str) -> bool {
        let wanted = self.match_key(target_pkg);
        files.iter().any(|f| {
            std::fs::read_to_string(f)
                .map(|b| b.lines().any(|l| self.matches(l, &wanted)))
                .unwrap_or(false)
        })
    }

    /// Remove every declaration of `target_pkg` from `files`, returning what was removed.
    ///
    /// The match is on the package, never on the raw prefix: `uninstall apt` must remove a
    /// package named `apt`, not every line whose backend is apt (S9).
    pub fn remove_from(&self, files: &[PathBuf], target_pkg: &str) -> Result<Vec<Edit>> {
        let wanted = self.match_key(target_pkg);
        let mut edits = Vec::new();

        for file in files {
            let Ok(body) = std::fs::read_to_string(file) else {
                continue;
            };
            let mut out: Vec<String> = Vec::new();
            let mut hit: Vec<String> = Vec::new();

            for raw in body.lines() {
                if self.matches(raw, &wanted) {
                    hit.push(raw.trim().to_string());
                    continue;
                }
                out.push(raw.to_string());
            }

            if hit.is_empty() {
                continue;
            }
            let mut new_body = out.join("\n");
            new_body.push('\n');
            write(file, &new_body)?;
            for line in hit {
                edits.push(Edit {
                    file: file.clone(),
                    line,
                    wired_into: None,
                });
            }
        }
        Ok(edits)
    }

    /// What the user typed, as something to match against.
    pub(crate) fn match_key(&self, target: &str) -> Match {
        match statement::parse(&Origin::argument(), target, self.backends) {
            Ok(Statement::Package(d)) | Ok(Statement::Absent(d)) => Match::Package {
                backend: d.backend,
                name: d.selector.as_str().to_string(),
            },
            Ok(other) => match other_key(&other) {
                Some(k) => Match::Other(k),
                None => Match::Other(target.to_string()),
            },
            Err(_) => Match::Other(target.to_string()),
        }
    }

    pub(crate) fn matches(&self, raw: &str, wanted: &Match) -> bool {
        let line = match raw.find('#') {
            Some(i) => &raw[..i],
            None => raw,
        }
        .trim();
        if line.is_empty() {
            return false;
        }
        let Ok(stmt) = statement::parse(&Origin::argument(), line, self.backends) else {
            return false;
        };

        match (wanted, &stmt) {
            (
                Match::Package { backend, name },
                Statement::Package(d) | Statement::Absent(d),
            ) => {
                if d.selector.as_str() != name {
                    return false;
                }
                match (backend, &d.backend) {
                    (Some(want), Some(got)) => want == got,
                    // A bare target names the package whatever backend holds it.
                    (None, _) => true,
                    (Some(_), None) => false,
                }
            }
            (Match::Other(k), other) => other_key(other).as_ref() == Some(k),
            _ => false,
        }
    }
}

/// What `uninstall`/`disable` was asked to find.
///
/// A package is matched on backend and name, because a bare name means "under whatever
/// backend has it". Everything else is matched whole: `service:nginx` names one thing.
pub(crate) enum Match {
    Package {
        backend: Option<String>,
        name: String,
    },
    Other(String),
}

/// `service:nginx`, `shim:jq` — the identity of a non-package statement.
fn other_key(stmt: &Statement) -> Option<String> {
    match stmt {
        Statement::Repo(s) => Some(format!("repo:{}", s)),
        Statement::Shim(n, _) => Some(format!("shim:{}", n)),
        Statement::Schedule(n, _) => Some(format!("schedule:{}", n)),
        Statement::Service(n, _) => Some(format!("service:{}", n)),
        Statement::Link(n, _) => Some(format!("link:{}", n)),
        Statement::Use(r) => Some(format!("use {}", r.name())),
        // Set math is an operation, not a thing with a name to look up.
        Statement::Exclude(_)
        | Statement::Intersect(_)
        | Statement::Subtract(_)
        | Statement::Expr(_)
        | Statement::Package(_)
        | Statement::Absent(_) => None,
    }
}

/// A failed write is an error that names the file. "Permission denied" with no path is a
/// message nobody can act on.
fn write(path: &std::path::Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_error(path, &e))?;
    }
    std::fs::write(path, body).map_err(|e| io_error(path, &e))
}

fn io_error(path: &std::path::Path, e: &std::io::Error) -> GrammarError {
    GrammarError::new(Origin::new(path, 0), format!("could not write this file: {}", e))
}

fn landing_of(module: &str) -> Option<Landing> {
    match module {
        "imperative" => Some(Landing::Imperative),
        "hooks" => Some(Landing::Hooks),
        "adopted" => Some(Landing::Adopted),
        _ => None,
    }
}

/// Every module file the active profiles reach — what `uninstall` edits (II.8).
pub fn active_module_files(
    layout: &Layout,
    backends: &dyn BackendNames,
    facts: &HostFacts,
) -> Vec<PathBuf> {
    let active_file = layout.active_file();
    let body = std::fs::read_to_string(&active_file).unwrap_or_default();
    let Ok(active) = parse_active(&active_file, &body) else {
        return Vec::new();
    };

    let profiles = ProfileLoader::new(layout, backends);
    let mut loader = ModuleLoader::new(layout, backends);
    let asked = Origin::new(&active_file, 0);
    let mut out: Vec<PathBuf> = Vec::new();

    for name in &active {
        let Ok(r) = profiles.resolve(name, &asked, facts, &mut Vec::new()) else {
            continue;
        };
        // A profile may hold package lines directly (II.4), so it is a file `uninstall`
        // must edit too.
        let pf = layout.profile_file(name);
        if !out.contains(&pf) {
            out.push(pf);
        }
        for m in &r.modules {
            let Ok(stmts) = super::modules::expand(&mut loader, m, &asked, facts, &mut Vec::new())
            else {
                continue;
            };
            for (_, o) in stmts {
                if !out.contains(&o.file) {
                    out.push(o.file);
                }
            }
        }
    }
    out
}

/// Where a package should land when nobody said (II.8's three landing modules).
pub fn landing_target(landing: Landing) -> Target {
    landing.target()
}

/// Modules that declare `target_pkg` but that no active profile reaches (II.8).
///
/// What `uninstall` warns about: *"jq is still declared in module `gaming`, which isn't
/// active. It will come back if you activate Gaming."* Deleting the line you can see, while
/// an identical line waits in a module you forgot about, is a package that returns from the
/// dead the next time you switch profiles — and nothing said so.
pub fn inactive_declarations(
    layout: &Layout,
    backends: &dyn BackendNames,
    facts: &HostFacts,
    target_pkg: &str,
) -> Vec<String> {
    let reached = active_module_files(layout, backends, facts);
    let editor = Editor::new(layout, backends).with_facts(facts.clone());
    let wanted = editor.match_key(target_pkg);

    let mut out: Vec<String> = Vec::new();
    let loader = ModuleLoader::new(layout, backends);
    for name in loader.available() {
        let path = layout.module_file(&name);
        if reached.contains(&path) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        if body.lines().any(|l| editor.matches(l, &wanted)) {
            out.push(name);
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn known(name: &str) -> bool {
        matches!(name, "apt" | "cargo" | "npm")
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
    }

    fn fx(files: &[(&str, &str)]) -> Fx {
        let tmp = TempDir::new().unwrap();
        let layout = Layout::new(tmp.path().join("cfg"), tmp.path().join("data"));
        std::fs::create_dir_all(layout.modules_dir()).unwrap();
        std::fs::create_dir_all(layout.profiles_dir()).unwrap();
        for (p, b) in files {
            let full = layout.config_root().join(p);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, b).unwrap();
        }
        Fx { _tmp: tmp, layout }
    }

    fn editor(f: &Fx) -> Editor<'_> {
        Editor::new(&f.layout, &known).with_facts(facts())
    }

    fn read(f: &Fx, p: &str) -> String {
        std::fs::read_to_string(f.layout.config_root().join(p)).unwrap_or_default()
    }

    #[test]
    fn installing_writes_a_line_to_the_landing_module() {
        let f = fx(&[("active", "Work\n"), ("profiles/Work", "use dev\n")]);
        let edit = editor(&f)
            .add(&Landing::Imperative.target(), "apt:jq")
            .unwrap();
        assert!(read(&f, "modules/imperative.txt").contains("apt:jq"));
        assert_eq!(edit.line, "apt:jq");
    }

    #[test]
    fn the_first_write_wires_the_module_into_the_active_profile_and_says_so() {
        // II.8: a normal line you can read and delete. Never implicit.
        let f = fx(&[("active", "Work\n"), ("profiles/Work", "use dev\n")]);
        let edit = editor(&f)
            .add(&Landing::Imperative.target(), "apt:jq")
            .unwrap();
        assert_eq!(edit.wired_into.as_deref(), Some("Work"));
        assert!(read(&f, "profiles/Work").contains("use imperative"));
        assert!(edit.describe("Added").contains("Work"));
    }

    #[test]
    fn a_module_already_reached_is_not_wired_again() {
        let f = fx(&[
            ("active", "Work\n"),
            ("profiles/Work", "use imperative\n"),
            ("modules/imperative.txt", "apt:curl\n"),
        ]);
        let edit = editor(&f)
            .add(&Landing::Imperative.target(), "apt:jq")
            .unwrap();
        assert_eq!(edit.wired_into, None);
        assert_eq!(read(&f, "profiles/Work").matches("use imperative").count(), 1);
    }

    #[test]
    fn installing_with_several_profiles_active_asks_which_one_rather_than_guessing() {
        // The alternative is picking one of your identities at random, or — worse — writing
        // the line, wiring nothing, and reporting success while installing nothing.
        let f = fx(&[
            ("active", "Work\nHome\n"),
            ("profiles/Work", "use dev\n"),
            ("profiles/Home", "use media\n"),
        ]);
        let err = editor(&f)
            .add(&Landing::Imperative.target(), "apt:jq")
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Work") && msg.contains("Home"), "{}", msg);
        assert!(msg.contains("--into"), "{}", msg);
        // And it wrote nothing: a refusal that half-applies is worse than either answer.
        assert_eq!(read(&f, "modules/imperative.txt"), "");
    }

    #[test]
    fn installing_with_nothing_active_refuses_rather_than_writing_a_line_nobody_reads() {
        let f = fx(&[("active", "\n")]);
        let err = editor(&f)
            .add(&Landing::Imperative.target(), "apt:jq")
            .unwrap_err();
        assert!(err.to_string().contains("nothing is active"), "{}", err);
        assert_eq!(read(&f, "modules/imperative.txt"), "");
    }

    #[test]
    fn into_a_named_profile_settles_the_question() {
        // Which is why the refusal above points at `--into`: it has to actually work.
        let f = fx(&[
            ("active", "Work\nHome\n"),
            ("profiles/Work", "use dev\n"),
            ("profiles/Home", "use media\n"),
        ]);
        let edit = editor(&f)
            .add(&Target::Profile("Work".into()), "apt:jq")
            .unwrap();
        assert!(read(&f, "profiles/Work").contains("apt:jq"));
        assert_eq!(edit.wired_into, None, "a profile is reached by `active`");
    }

    #[test]
    fn a_new_landing_module_explains_itself() {
        // V.40: provenance ends up in the filename, and the file says what it is for.
        let f = fx(&[("active", "Work\n"), ("profiles/Work", "\n")]);
        editor(&f)
            .add(&Landing::Hooks.target(), "apt:htop")
            .unwrap();
        let body = read(&f, "modules/hooks.txt");
        assert!(body.contains("behind LiNix's back"), "{}", body);
        assert!(body.contains("apt:htop"));
    }

    #[test]
    fn installing_a_pinned_version_replaces_the_unpinned_line() {
        // Two declarations of one package is a conflict the resolver then refuses, so the
        // line must be replaced rather than joined.
        let f = fx(&[
            ("active", "Work\n"),
            ("profiles/Work", "use imperative\n"),
            ("modules/imperative.txt", "apt:jq\napt:curl\n"),
        ]);
        editor(&f)
            .add(&Landing::Imperative.target(), "apt:jq@version=1.6")
            .unwrap();
        let body = read(&f, "modules/imperative.txt");
        assert!(body.contains("apt:jq@version=1.6"), "{}", body);
        assert_eq!(body.matches("apt:jq").count(), 1, "one jq line, not two");
        assert!(body.contains("apt:curl"), "other lines survive");
    }

    #[test]
    fn removing_a_package_named_after_a_backend_does_not_delete_that_backend() {
        // S9. `remove_package_from_local` compared the target against the BACKEND half, so
        // `uninstall npm` deleted every `npm:*` line. The package manager's whole package
        // set, gone, because one package shares its name.
        let f = fx(&[(
            "modules/dev.txt",
            "npm:typescript\nnpm:eslint\napt:npm\n",
        )]);
        let file = f.layout.module_file("dev");
        let edits = editor(&f).remove_from(&[file], "npm").unwrap();

        let body = read(&f, "modules/dev.txt");
        assert!(body.contains("npm:typescript"), "{}", body);
        assert!(body.contains("npm:eslint"), "{}", body);
        assert!(!body.contains("apt:npm"), "the package named npm goes");
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn a_bare_target_removes_the_package_under_any_backend() {
        let f = fx(&[("modules/dev.txt", "cargo:ripgrep\napt:curl\n")]);
        let file = f.layout.module_file("dev");
        editor(&f).remove_from(&[file], "ripgrep").unwrap();
        let body = read(&f, "modules/dev.txt");
        assert!(!body.contains("ripgrep"), "{}", body);
        assert!(body.contains("apt:curl"));
    }

    #[test]
    fn an_explicit_target_removes_only_that_backends_line() {
        let f = fx(&[("modules/dev.txt", "cargo:ripgrep\napt:ripgrep\n")]);
        let file = f.layout.module_file("dev");
        editor(&f).remove_from(&[file], "apt:ripgrep").unwrap();
        let body = read(&f, "modules/dev.txt");
        assert!(body.contains("cargo:ripgrep"), "{}", body);
        assert!(!body.contains("apt:ripgrep"), "{}", body);
    }

    #[test]
    fn removing_keeps_comments_and_blank_lines() {
        // LiNix must not rewrite your files beyond the line it was asked to remove.
        let f = fx(&[(
            "modules/dev.txt",
            "# my tools\n\napt:curl   # needed for work\napt:jq\n",
        )]);
        let file = f.layout.module_file("dev");
        editor(&f).remove_from(&[file], "jq").unwrap();
        let body = read(&f, "modules/dev.txt");
        assert!(body.contains("# my tools"), "{}", body);
        assert!(body.contains("apt:curl   # needed for work"), "{}", body);
        assert!(!body.contains("apt:jq"));
    }

    #[test]
    fn uninstall_reaches_every_module_the_active_profiles_hold() {
        let f = fx(&[
            ("active", "Work\n"),
            ("profiles/Work", "use dev\n"),
            ("modules/dev.txt", "use base\napt:curl\n"),
            ("modules/base.txt", "apt:jq\n"),
            ("modules/unused.txt", "apt:steam\n"),
        ]);
        let files = active_module_files(&f.layout, &known, &facts());
        assert!(files.contains(&f.layout.module_file("dev")));
        assert!(
            files.contains(&f.layout.module_file("base")),
            "a module reached through another is still active"
        );
        assert!(
            !files.contains(&f.layout.module_file("unused")),
            "nothing is active unless a profile reaches it"
        );
        assert!(
            files.contains(&f.layout.profile_file("Work")),
            "a profile may hold packages directly, so it is a file uninstall edits"
        );
    }

    #[test]
    fn a_service_line_is_declared_and_undeclared_by_name() {
        // `linix service enable nginx` writes a line; `disable` takes it away again. It is
        // not a package, so matching it on backend and name would never find it.
        let f = fx(&[("active", "Work\n"), ("profiles/Work", "use imperative\n")]);
        let e = editor(&f);
        e.add(&Landing::Imperative.target(), "service:nginx@enabled=true")
            .unwrap();
        assert!(read(&f, "modules/imperative.txt").contains("service:nginx"));

        let file = f.layout.module_file("imperative");
        let edits = e.remove_from(&[file], "service:nginx").unwrap();
        assert_eq!(edits.len(), 1);
        assert!(!read(&f, "modules/imperative.txt").contains("service:nginx"));
    }

    #[test]
    fn re_enabling_a_service_replaces_its_line_rather_than_doubling_it() {
        let f = fx(&[
            ("active", "Work\n"),
            ("profiles/Work", "use imperative\n"),
            ("modules/imperative.txt", "service:nginx@enabled=false\n"),
        ]);
        editor(&f)
            .add(&Landing::Imperative.target(), "service:nginx@enabled=true")
            .unwrap();
        let body = read(&f, "modules/imperative.txt");
        assert_eq!(body.matches("service:nginx").count(), 1, "{}", body);
        assert!(body.contains("enabled=true"), "{}", body);
    }

    #[test]
    fn target_case_decides_module_or_profile() {
        let o = Origin::argument();
        assert_eq!(
            Target::parse("editors", &o).unwrap(),
            Target::Module("editors".into())
        );
        assert_eq!(
            Target::parse("Work", &o).unwrap(),
            Target::Profile("Work".into())
        );
        assert!(Target::parse("9lives", &o).is_err());
    }
}
