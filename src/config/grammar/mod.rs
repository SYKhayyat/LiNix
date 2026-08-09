//! The grammar of a LiNix file (SPEC II.2).
//!
//! One parser. Eight parsed `backend:name` before this, six of them without checking that
//! the prefix named a real backend — so every prefix added here (`absent:`, `repo:`,
//! `re:`) was a thing they read as a backend name instead (C13).

pub mod error;
pub mod gated;
pub mod options;
pub mod statement;

pub use error::{GrammarError, Origin, Result};
pub use gated::{GatedLine, Vocabulary};
pub use options::Options;
pub use statement::{
    BackendNames, Candidates, KeywordRole, PackageDecl, Phase, Reference, ResourceKind, Selector,
    Statement, PRIORITY_KEYWORD, RESERVED_BACKEND_NAMES,
};

use crate::config::parser::{eval_when, HostFacts};
use std::path::Path;

/// Whether `backend:name` could be written on a line and read back as the same package.
///
/// A package manager may report something that is not a declarable name: `winget list`
/// answers for Add/Remove-Programs entries with pseudo-IDs like
/// `ARP\Machine\X64\Android Studio`, and a package name is one word (II.2). Anything that
/// turns a manager's answer into a declaration — or decides whether LiNix could ever have
/// been asked to keep one — has to agree on that, so they all ask here.
///
/// Round-tripped rather than parsed: `winget:ARP\Machine\X64\Android Studio` *parses*, as a
/// set expression, and only reading it back as the package it came from catches that.
///
/// **`None` is a name with no backend written beside it** — `jq`, as a user types it into
/// `linix protected`, and as the grammar accepts it on a line that lets `priority` decide the
/// manager. It is not the same question as `""`, which builds the line `:jq` and is refused by
/// every grammar there has ever been: asking it that way told `linix protected` that `jq`,
/// `sudo` and every other bare name was undeclarable, so the guard's declarability test fired
/// before a single rule was read and the answer to *which rule protects this* was a sentence
/// about package lines.
pub fn is_declarable(backend: Option<&str>, name: &str) -> bool {
    declarable_line(backend, name).is_some()
}

/// What the grammar makes of `backend:name` — and, when it is a package, how it is spelled.
///
/// Three answers rather than two, because the two-answer version told a lie that a user then
/// read: `service:AppMgmt` parses, and every caller asked "is this a package line?" and reported
/// the `false` as *no line can hold this name*. On a stock Windows box that was 155 services
/// held back from adoption with a reason that was not true of a single one of them.
#[derive(Debug, PartialEq, Eq)]
pub enum Declared {
    /// A package, and this is the line that declares it.
    Package(String),
    /// A legal declaration that is not a package, and this is the line that declares it.
    /// `service:`, `link:` and `setting:` are their own statement kinds (II.2).
    Resource(String),
    /// No line of any kind can carry this name.
    Nothing,
}

/// Ask the grammar, once, and let the caller decide what to do with each answer.
pub fn declared(backend: Option<&str>, name: &str) -> Declared {
    declared_as(backend, name, &[])
}

/// As [`declared`], for a line that must also carry `options`.
///
/// `adopt` declares a service as the state it observed — `service:sshd@status=running` — and the
/// options get the same round trip the name does. A line that comes back carrying a different
/// state is not the line that was asked for, and writing it would declare something nobody saw.
pub fn declared_as(backend: Option<&str>, name: &str, options: &[(&str, &str)]) -> Declared {
    // A `$` in a written name is a variable reference (IX.3), and the resolver refuses an
    // undefined one — so `service:MSSQL$SQLEXPRESS`, written verbatim, is a file that parses
    // and then fails to resolve, wedging every later command. `$$` is the one escape. Applied
    // here rather than at the writer, because this is the function that decides how a name is
    // spelled, and the round trip below is against the spelling that will actually be read
    // back: quoting does not protect a `$`, and no amount of parser agreement would have
    // caught this, since expansion happens after the parse.
    let escaped = |s: &str| s.replace('$', "$$");
    let name_on_the_line = escaped(name);
    let suffix: String = options
        .iter()
        .map(|(k, v)| format!("@{}={}", k, escaped(v)))
        .collect();
    let asked_for = |parsed: &Options| {
        parsed.keys().count() == options.len()
            && options
                .iter()
                .all(|(k, v)| parsed.one(k) == Some(escaped(v).as_str()))
    };
    // Bare first, quoted only if the bare form will not carry it. A manifest full of
    // needlessly quoted `winget:Mozilla.Firefox` is noise, and the quotes would then be the
    // thing a reader has to decide is meaningful.
    let mut resource = None;
    for candidate in [
        name_on_the_line.clone(),
        format!("\"{}\"", name_on_the_line),
    ] {
        let line = match backend {
            Some(b) => format!("{}:{}{}", b, candidate, suffix),
            None => format!("{}{}", candidate, suffix),
        };
        let is_this_backend = |n: &str| Some(n) == backend;
        match statement::parse(&Origin::argument(), &line, &is_this_backend) {
            Ok(Statement::Package(d))
                if d.backend.as_deref() == backend
                    && d.selector.as_str() == name_on_the_line
                    && asked_for(&d.options) =>
            {
                return Declared::Package(line);
            }
            // A package whose name came back changed is not this package — that is what the
            // round trip is for, and it is not evidence that some other statement kind would
            // carry it either.
            Ok(Statement::Package(_)) | Err(_) => {}
            Ok(st) => {
                // A resource is listed under its own prefix, so the prefix has to be the
                // backend the caller asked about: `winget:something` that happens to parse as
                // some other statement kind is not a `winget` resource, and offering it as one
                // would put a line in the manifest that declares a different thing entirely.
                if let Some(((prefix, back), opts)) = st.listed_with_options() {
                    if Some(prefix) == backend && back == name_on_the_line && asked_for(opts) {
                        resource = Some(line);
                    }
                }
            }
        }
    }
    resource.map_or(Declared::Nothing, Declared::Resource)
}

/// The exact line that declares this package, or `None` if no line can.
///
/// **One function decides both whether a name can be written and how it is spelled.** They were
/// the same question asked in two places: `is_declarable` round-tripped `backend:name` while
/// `adopt` rendered `backend:name` by hand, so the day the grammar learned to quote a name with
/// a space in it, the check would have said yes and the writer would still have emitted the
/// unquoted form — a manifest that fails to parse, written by the command whose whole job is to
/// produce one that does. That is the bug `2c51968` already fixed once in the other direction.
///
/// Round-tripped rather than assumed: whatever comes back out has to be the name that went in.
pub fn declarable_line(backend: Option<&str>, name: &str) -> Option<String> {
    match declared(backend, name) {
        Declared::Package(line) => Some(line),
        Declared::Resource(_) | Declared::Nothing => None,
    }
}

impl Declared {
    /// The line that declares this, whichever kind it turned out to be.
    pub fn line(&self) -> Option<&str> {
        match self {
            Declared::Package(line) | Declared::Resource(line) => Some(line),
            Declared::Nothing => None,
        }
    }
}

/// A `{ }` block, already classified by its header. II.2: the header decides what the body
/// is — `module` and `when` are keywords whose bodies are lines; anything else is a
/// declaration whose body is options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// `module fancy { ... }` — body is lines.
    Module(String, Vec<Item>),
    /// `when os == linux { ... }` — body is lines, gated by the predicate.
    ///
    /// One rule everywhere: in a module those lines are packages, in a profile they are
    /// imports, in `priority` they are backends.
    When(String, Vec<Item>),
}

/// One thing a file says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Statement(Statement, Origin),
    Block(Block, Origin),
}

/// A `when` condition that admitted a statement, and the line it is written on.
///
/// Distinct from the statement's own [`Origin`], which says where the *line* is: two
/// questions, two answers. `why` needs both to say "htop is here because `$role == travel`
/// matched, and that is written at `active:4`".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub predicate: String,
    pub origin: Origin,
}

impl Gate {
    pub fn new(predicate: impl Into<String>, origin: Origin) -> Self {
        Self {
            predicate: predicate.into(),
            origin,
        }
    }
}

impl std::fmt::Display for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "when {} @ {}", self.predicate, self.origin)
    }
}

impl std::str::FromStr for Gate {
    type Err = ();

    /// The inverse of [`Display`], kept beside it: the round trip crosses the `PackageSpec`
    /// seam, where everything is a string, so the two halves drift the moment they are apart.
    fn from_str(s: &str) -> std::result::Result<Self, ()> {
        let (pred, at) = s.rsplit_once(" @ ").ok_or(())?;
        let predicate = pred.strip_prefix("when ").ok_or(())?.to_string();
        Ok(Gate::new(predicate, at.parse::<Origin>()?))
    }
}

/// The chain of `when` conditions that admitted a statement, outermost first — the `active`
/// block that turned the profile on, then the profile's own block, then the module's.
pub type Gates = Vec<Gate>;

/// A parsed file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub items: Vec<Item>,
}

impl Document {
    /// Flatten to the statements this host actually gets, evaluating `when` and inlining
    /// `module` blocks' bodies. Returns each statement with where it came from, so a
    /// conflict can name both files (II.7 rule 5).
    pub fn statements_for(&self, facts: &HostFacts) -> Result<Vec<(Statement, Origin)>> {
        Ok(self
            .statements_with_gating(facts)?
            .into_iter()
            .map(|(s, o, _)| (s, o))
            .collect())
    }

    /// As [`Document::statements_for`], but each statement also carries the `when` conditions
    /// that admitted it, outermost first.
    ///
    /// IX.3 turns on the emptiness of that chain: a top-level line defines a variable and a
    /// conditional one may only override, so the two cannot be flattened together. W11 turns
    /// on its contents: `why` names the condition, and the variables inside it, behind a
    /// package.
    pub fn statements_with_gating(
        &self,
        facts: &HostFacts,
    ) -> Result<Vec<(Statement, Origin, Gates)>> {
        let mut out = Vec::new();
        Self::walk(&self.items, facts, &Vec::new(), &mut out)?;
        Ok(out)
    }

    /// Every statement the file contains, `when` blocks included whether or not they match,
    /// each flagged with whether a `when` put it there.
    ///
    /// For checks that are properties of the FILE rather than of this machine — IX.3's "every
    /// variable is defined on every machine" cannot be answered by the box that happens to be
    /// running, or the same file is valid on the laptop and broken on the desktop.
    ///
    /// Never use this to decide what to install: it deliberately ignores the gating that says
    /// what belongs on this host.
    pub fn every_statement(&self) -> Vec<(&Statement, &Origin, bool)> {
        let mut out = Vec::new();
        Self::walk_ungated(&self.items, false, &mut out);
        out
    }

    fn walk_ungated<'d>(
        items: &'d [Item],
        conditional: bool,
        out: &mut Vec<(&'d Statement, &'d Origin, bool)>,
    ) {
        for item in items {
            match item {
                Item::Statement(s, o) => out.push((s, o, conditional)),
                Item::Block(Block::Module(_, body), _) => {
                    Self::walk_ungated(body, conditional, out)
                }
                Item::Block(Block::When(_, body), _) => Self::walk_ungated(body, true, out),
            }
        }
    }

    fn walk(
        items: &[Item],
        facts: &HostFacts,
        gates: &Gates,
        out: &mut Vec<(Statement, Origin, Gates)>,
    ) -> Result<()> {
        for item in items {
            match item {
                Item::Statement(s, o) => out.push((s.clone(), o.clone(), gates.clone())),
                Item::Block(Block::Module(_, body), _) => Self::walk(body, facts, gates, out)?,
                Item::Block(Block::When(pred, body), origin) => {
                    let hit = eval_when(pred, facts).map_err(|e| {
                        GrammarError::new(origin.clone(), e.to_string()).with_hint(
                            "`when` keys are os, arch, host, hostname, family, or `$name` for \
                             a variable; operators are ==, != and `in [a, b]`.",
                        )
                    })?;
                    if hit {
                        let mut inner = gates.clone();
                        inner.push(Gate::new(pred.clone(), origin.clone()));
                        Self::walk(body, facts, &inner, out)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Every `module NAME { }` block in the file, by name.
    pub fn module_blocks(&self) -> Vec<(&str, &[Item])> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::Block(Block::Module(n, body), _) => Some((n.as_str(), body.as_slice())),
                _ => None,
            })
            .collect()
    }
}

/// Strip a trailing comment from a statement line.
///
/// A `#` opens a comment only at the start of a line or after whitespace. Cutting at the
/// first `#` anywhere truncated any short-form value containing one — `@content=#!/bin/sh`
/// became `@content=` — silently, and made the block form the only way to write a `#` at all.
///
/// `#` inside a `{ }` block VALUE is data whatever it follows (V.9); that case never reaches
/// here, because block values are read by `options::parse_block_line`.
pub fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, c) in line.char_indices() {
        if c != '#' {
            continue;
        }
        if i == 0 || bytes[i - 1].is_ascii_whitespace() {
            return &line[..i];
        }
    }
    line
}

/// Parse a whole file body.
pub fn parse_document(file: &Path, body: &str, backends: &dyn BackendNames) -> Result<Document> {
    let body = crate::config::without_bom(body);
    let mut lines = body.lines().enumerate().peekable();
    let items = parse_items(file, &mut lines, backends, false)?;
    Ok(Document { items })
}

type Lines<'a> = std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'a>>>;

fn parse_items(
    file: &Path,
    lines: &mut Lines<'_>,
    backends: &dyn BackendNames,
    nested: bool,
) -> Result<Vec<Item>> {
    let mut items = Vec::new();

    while let Some((idx, raw)) = lines.next() {
        let origin = Origin::new(file, idx + 1);
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if line == "}" {
            if nested {
                return Ok(items);
            }
            return Err(GrammarError::new(
                origin,
                "`}` closes a block that was never opened",
            ));
        }

        // A `{` at the end makes this a block header. The header decides the body kind.
        if let Some(header) = line.strip_suffix('{') {
            items.push(parse_block(file, header.trim(), lines, backends, &origin)?);
            continue;
        }

        items.push(Item::Statement(
            statement::parse(&origin, line, backends).map_err(|e| unrecognised(e, line))?,
            origin,
        ));
    }

    if nested {
        return Err(
            GrammarError::new(Origin::new(file, 0), "a `{` block is never closed")
                .with_hint("add the matching `}`."),
        );
    }
    Ok(items)
}

/// II.2: an unrecognised line is an error, not a package name. The parser's own message is
/// usually the specific one; this only fires when nothing recognised the line at all.
fn unrecognised(e: GrammarError, line: &str) -> GrammarError {
    if e.hint.is_some() {
        return e;
    }
    e.with_hint(format!(
        "expected a package (`apt:curl`), one of {}, `use NAME`, or a `{{` block. \
         `{}` is none of those.",
        statement::known_prefixes()
            .iter()
            .map(|p| format!("`{}`", p))
            .collect::<Vec<_>>()
            .join(", "),
        line
    ))
}

/// II.2: the header decides what the body is. `module` and `when` are keywords and their
/// bodies are lines; anything else is a declaration and its body is options.
fn parse_block(
    file: &Path,
    header: &str,
    lines: &mut Lines<'_>,
    backends: &dyn BackendNames,
    origin: &Origin,
) -> Result<Item> {
    if let Some(name) = header.strip_prefix("module ") {
        let name = name.trim();
        if name.is_empty() {
            return Err(GrammarError::new(
                origin.clone(),
                "`module` block has no name",
            ));
        }
        // A module name is lowercase; a Capitalized one would mint a profile, which only a
        // file in `profiles/` may do (II.5).
        if name.chars().next().is_some_and(|c| c.is_uppercase()) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("module `{}` is Capitalized", name),
            )
            .with_hint("modules are lowercase; profiles are Capitalized."));
        }
        let body = parse_items(file, lines, backends, true)?;
        return Ok(Item::Block(
            Block::Module(name.to_string(), body),
            origin.clone(),
        ));
    }

    if let Some(pred) = header.strip_prefix("when ") {
        let pred = pred.trim();
        if pred.is_empty() {
            return Err(GrammarError::new(
                origin.clone(),
                "`when` block has no condition",
            ));
        }
        let body = parse_items(file, lines, backends, true)?;
        return Ok(Item::Block(
            Block::When(pred.to_string(), body),
            origin.clone(),
        ));
    }

    // Not a keyword, so it is a declaration and its body is options (II.2).
    parse_declaration_block(file, header, lines, backends, origin)
}

/// `apt:nginx { after_install = ./setup.sh }` — a declaration whose body is options.
///
/// The options fold onto the declaration, so downstream sees one `Statement` whichever
/// form was written: the block form is a way to hold values the short form cannot (V.9),
/// not a different kind of thing.
fn parse_declaration_block(
    file: &Path,
    header: &str,
    lines: &mut Lines<'_>,
    backends: &dyn BackendNames,
    origin: &Origin,
) -> Result<Item> {
    let mut stmt =
        statement::parse(origin, header, backends).map_err(|e| unrecognised(e, header))?;

    let mut body_opts = Options::default();
    loop {
        let Some((idx, raw)) = lines.next() else {
            return Err(GrammarError::new(
                origin.clone(),
                format!("the `{}` block is never closed", header),
            )
            .with_hint("add the matching `}`."));
        };
        let line_origin = Origin::new(file, idx + 1);
        let trimmed = raw.trim();
        if trimmed == "}" {
            break;
        }
        // A whole-line comment is still a comment inside a block; only VALUES are verbatim.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (k, v) = options::parse_block_line(&line_origin, trimmed)?;
        body_opts.insert(k, v);
    }

    merge_options(&mut stmt, body_opts, origin)?;
    Ok(Item::Statement(stmt, origin.clone()))
}

fn merge_options(stmt: &mut Statement, extra: Options, origin: &Origin) -> Result<()> {
    let target = match stmt {
        Statement::Package(d) | Statement::Absent(d) => &mut d.options,
        Statement::Shim(_, o)
        | Statement::Schedule(_, o)
        | Statement::Service(_, o)
        | Statement::Link(_, o)
        | Statement::Setting(_, o)
        | Statement::Exec(_, o)
        | Statement::Dotfiles(_, o)
        | Statement::Firewall(_, o) => o,
        Statement::Repo { .. }
        | Statement::Use(..)
        | Statement::Param { .. }
        | Statement::Generate(..)
        | Statement::Exclude(_)
        | Statement::Intersect(_)
        | Statement::Subtract(_)
        | Statement::Var { .. }
        | Statement::Expr(_) => {
            return Err(GrammarError::new(
                origin.clone(),
                "this statement takes no options",
            ))
        }
    };
    for (k, vs) in extra.iter() {
        for v in vs {
            target.insert(k, v.clone());
        }
    }
    // The header was validated when it parsed, but the body's keys arrive after that, so a
    // block form re-checks the whole statement or it checks nothing.
    statement::validate(origin, stmt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn known(name: &str) -> bool {
        matches!(name, "apt" | "cargo" | "snap")
    }

    fn doc(body: &str) -> Result<Document> {
        parse_document(&PathBuf::from("modules/dev.txt"), body, &known)
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

    fn stmts(body: &str) -> Vec<Statement> {
        doc(body)
            .unwrap()
            .statements_for(&facts())
            .unwrap()
            .into_iter()
            .map(|(s, _)| s)
            .collect()
    }

    // ---------------------------------------------------------------- lines

    #[test]
    fn blank_lines_and_whole_line_comments_are_skipped() {
        let out = stmts("# a comment\n\n   \napt:curl\n");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_trailing_comment_is_stripped_from_a_statement() {
        let out = stmts("apt:curl    # we need this\n");
        let Statement::Package(d) = &out[0] else {
            panic!()
        };
        assert_eq!(d.selector.as_str(), "curl");
    }

    #[test]
    fn an_unrecognised_line_is_an_error_not_a_package_name() {
        // II.2, and VI.1's "any typo becomes a package name". The error must name the
        // file, the line, and what was expected.
        let err = doc("apt:curl\nthis is not a thing\n").unwrap_err();
        assert_eq!(err.origin.line, 2);
        assert!(err.to_string().contains("modules/dev.txt:2"), "{}", err);
    }

    // --------------------------------------------------------------- blocks

    #[test]
    fn a_module_block_body_is_lines() {
        let out = stmts("module fancy {\n  apt:neovim\n  cargo:ripgrep\n}\n");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn a_module_block_must_be_lowercase() {
        // A Capitalized name would mint a profile, and only profiles/ may do that (II.5).
        let err = doc("module Fancy {\n  apt:neovim\n}\n").unwrap_err();
        assert!(err.hint.unwrap().contains("modules are lowercase"));
    }

    #[test]
    fn a_declaration_block_body_is_options() {
        // The header decides the body kind: `apt:nginx` is not a keyword, so options.
        let out = stmts("apt:nginx {\n  after_install = ./setup.sh\n}\n");
        let Statement::Package(d) = &out[0] else {
            panic!()
        };
        assert_eq!(d.options.one("after_install"), Some("./setup.sh"));
    }

    #[test]
    fn a_declaration_block_keeps_commas_and_equals_in_a_value() {
        let out = stmts("apt:nginx {\n  after_install = ./setup.sh --flag=a,b\n}\n");
        let Statement::Package(d) = &out[0] else {
            panic!()
        };
        assert_eq!(
            d.options.one("after_install"),
            Some("./setup.sh --flag=a,b")
        );
    }

    #[test]
    fn a_key_given_twice_in_a_block_makes_a_list() {
        let out = stmts("apt:nginx {\n  requires = apt:libfoo\n  requires = apt:libbar\n}\n");
        let Statement::Package(d) = &out[0] else {
            panic!()
        };
        assert_eq!(d.options.all("requires"), ["apt:libfoo", "apt:libbar"]);
    }

    #[test]
    fn an_unclosed_block_is_an_error() {
        assert!(doc("module fancy {\n  apt:neovim\n").is_err());
        assert!(doc("apt:nginx {\n  after_install = ./x.sh\n").is_err());
    }

    #[test]
    fn a_stray_closing_brace_is_an_error() {
        assert!(doc("apt:curl\n}\n").is_err());
    }

    // ----------------------------------------------------------------- when

    #[test]
    fn when_gates_the_lines_inside_it() {
        let out = stmts("when os == linux {\n  apt:htop\n}\n");
        assert_eq!(out.len(), 1);
        let out = stmts("when os == windows {\n  apt:htop\n}\n");
        assert!(out.is_empty());
    }

    #[test]
    fn when_supports_inequality_and_membership() {
        assert_eq!(stmts("when os != windows {\n  apt:htop\n}\n").len(), 1);
        assert_eq!(
            stmts("when arch in [x86_64, aarch64] {\n  apt:htop\n}\n").len(),
            1
        );
        assert!(stmts("when arch in [aarch64] {\n  apt:htop\n}\n").is_empty());
    }

    #[test]
    fn when_nests_inside_a_module() {
        let out = stmts("module fancy {\n  when os == linux {\n    apt:htop\n  }\n}\n");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn an_unknown_when_key_is_an_error_that_lists_the_real_ones() {
        let err = doc("when platform == linux {\n  apt:htop\n}\n")
            .unwrap()
            .statements_for(&facts())
            .unwrap_err();
        assert!(err
            .hint
            .unwrap()
            .contains("os, arch, host, hostname, family"));
    }

    // ------------------------------------------------------------ statements

    #[test]
    fn every_statement_form_parses() {
        let out = stmts(
            "ripgrep\n\
             apt:curl\n\
             apt:re:^fonts-\n\
             absent:apt:libreoffice\n\
             repo:apt:ppa:deadsnakes/ppa\n\
             shim:jq@source=cargo:jq\n\
             service:nginx@enabled=true\n\
             link:/home/me/.vimrc\n\
             use editors\n",
        );
        assert_eq!(out.len(), 9);
        assert!(matches!(out[3], Statement::Absent(_)));
        assert!(matches!(out[4], Statement::Repo { .. }));
        assert!(matches!(out[8], Statement::Use(Reference::Module(_), _)));
    }

    /// The block form used to be a way around every rule in II.2's table: the header was
    /// validated, the body was merged in afterwards unchecked. Each of these passed clean.
    #[test]
    fn a_block_body_is_held_to_the_same_rules_as_the_short_form() {
        for (short, block) in [
            (
                "apt:nginx@requires=libfoo",
                "apt:nginx {\n  requires = libfoo\n}",
            ),
            (
                "apt:jq@hold@version=1.6",
                "apt:jq@hold {\n  version = 1.6\n}",
            ),
            ("apt:nginx@colour=blue", "apt:nginx {\n  colour = blue\n}"),
            ("apt:nginx@lease=2h", "apt:nginx {\n  lease = 2h\n}"),
            ("apt:curl@formats=deb", "apt:curl {\n  formats = deb\n}"),
            ("apt:curl@expires=2h", "apt:curl {\n  expires = 2h\n}"),
        ] {
            assert!(
                doc(&format!("{}\n", short)).is_err(),
                "short form allowed `{}`",
                short
            );
            assert!(
                doc(&format!("{}\n", block)).is_err(),
                "block form allowed `{}`",
                block
            );
        }
    }

    /// Cutting at the first `#` anywhere made the block form the only way to write a value
    /// containing one, and did it without saying a word.
    #[test]
    fn a_hash_inside_a_value_is_data_and_a_hash_after_a_space_is_a_comment() {
        assert_eq!(strip_comment("apt:curl   # why"), "apt:curl   ");
        assert_eq!(strip_comment("# whole line"), "");
        assert_eq!(
            strip_comment("link:/etc/x@content=#!/bin/sh"),
            "link:/etc/x@content=#!/bin/sh"
        );
    }

    #[test]
    fn a_retired_option_cannot_be_reached_through_a_block() {
        // `@lease` is the one that mattered: II.16 retired it, and something downstream still
        // read it and turned it into a real expiry (S19).
        let err = doc("apt:jq {\n  lease = 2h\n}\n").unwrap_err().to_string();
        assert!(err.contains("lease"), "{}", err);
    }

    /// Three answers, because the two-answer version reported a true thing about packages as a
    /// false thing about names: `service:AppMgmt` is a perfectly good declaration, and `adopt`
    /// told 155 of them their manager reports "a name no package line can hold".
    ///
    /// The family, not the finding: every statement kind that is not a package answers the same
    /// way, a name nothing can carry still answers `Nothing`, and `Package` still carries the
    /// exact spelling — which is what `adopt` writes, so the three answers cannot drift from the
    /// one line.
    #[test]
    fn a_name_that_declares_something_other_than_a_package_says_so() {
        use super::{declarable_line, declared, is_declarable, Declared};

        // Its own statement kind (II.2) — writable, just not as a package. The whole family,
        // because `adopt` writes all three and a spelling that drifts writes a manifest that
        // declares something other than what was found.
        for (backend, name) in [
            ("service", "AppMgmt"),
            ("service", "sshd"),
            ("link", "/home/u/.vimrc"),
        ] {
            assert_eq!(
                declared(Some(backend), name),
                Declared::Resource(format!("{}:{}", backend, name)),
                "`{}:{}` parses, and was being reported as unwritable",
                backend,
                name
            );
        }

        // Options round-trip alongside the name: a line that came back declaring a different
        // state is not the line that was asked for.
        assert_eq!(
            super::declared_as(Some("service"), "sshd", &[("status", "running")]),
            Declared::Resource("service:sshd@status=running".to_string())
        );
        assert_eq!(
            super::declared_as(Some("service"), "sshd", &[("nonsense", "1")]),
            Declared::Nothing,
            "an option the grammar refuses must not be written into the manifest"
        );
        // A `setting:` is a resource whose line is illegal until it carries its value, so the
        // name alone is genuinely unwritable — which is why the guard asks about the backend
        // and not about the name.
        let key = "org.gnome.desktop.interface/clock-format";
        assert_eq!(declared(Some("setting"), key), Declared::Nothing);
        assert_eq!(
            super::declared_as(Some("setting"), key, &[("value", "24h")]),
            Declared::Resource(format!("setting:{}@value=24h", key))
        );

        // A `$` in a name is a variable reference, and an undefined one is refused at resolve
        // time — after the parse, so no amount of parser agreement catches it. `MSSQL$SQLEXPRESS`
        // is a real service on a stock SQL Server box, and written verbatim it produced a file
        // that parsed and then wedged every later command. The family: a package name can carry
        // one too, and did so through `declarable_line` before this.
        assert_eq!(
            declared(Some("service"), "MSSQL$SQLEXPRESS"),
            Declared::Resource("service:MSSQL$$SQLEXPRESS".to_string())
        );
        assert_eq!(
            declared(Some("winget"), "Foo$Bar"),
            Declared::Package("winget:Foo$$Bar".to_string())
        );

        // A resource name gets the same round trip a package name does. `sshd@status=running`
        // as a *name* parses as the service `sshd` carrying an option, and answering with
        // `service:sshd` would hand the manifest a line about a different declaration than the
        // one asked about — the exact failure the round trip exists to catch.
        assert_ne!(
            declared(Some("service"), "sshd@status=running"),
            Declared::Resource("service:sshd".to_string()),
            "a name that came back changed is not that name"
        );

        // A package is still a package, and still spelled exactly one way.
        assert_eq!(
            declared(Some("cargo"), "ripgrep"),
            Declared::Package("cargo:ripgrep".to_string())
        );
        assert_eq!(
            declared(Some("winget"), r"ARP\Machine\X64\Mozilla Firefox"),
            Declared::Package(r#"winget:"ARP\Machine\X64\Mozilla Firefox""#.to_string()),
            "a name needing quotes must come back quoted, or adopt writes a line that will \
             not parse"
        );

        // And a name no line of any kind can carry is still exactly that.
        assert_eq!(declared(Some("winget"), "two\nlines"), Declared::Nothing);
        assert_eq!(
            declared(Some("winget"), "Some \"Quoted\" Program"),
            Declared::Nothing
        );

        // The two older questions are the same question, so they cannot disagree with it.
        assert!(!is_declarable(Some("service"), "AppMgmt"));
        assert_eq!(declarable_line(Some("service"), "AppMgmt"), None);
        assert!(is_declarable(Some("cargo"), "ripgrep"));
    }

    #[test]
    fn statements_carry_where_they_came_from() {
        // II.7 rule 5 needs this: a conflict must name BOTH files.
        let d = doc("# header\napt:curl\n").unwrap();
        let out = d.statements_for(&facts()).unwrap();
        assert_eq!(out[0].1.line, 2);
        assert_eq!(out[0].1.file, PathBuf::from("modules/dev.txt"));
    }
}
