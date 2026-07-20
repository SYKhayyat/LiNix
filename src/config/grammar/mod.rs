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
    BackendNames, PackageDecl, Reference, Selector, Statement, RESERVED_BACKEND_NAMES,
};

use crate::config::parser::{eval_when, HostFacts};
use std::path::Path;

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

    /// As [`Document::statements_for`], but each statement also says whether a `when` block
    /// put it here.
    ///
    /// IX.3 turns on exactly that distinction: a top-level line defines a variable and a
    /// conditional one may only override, so the two cannot be flattened together.
    pub fn statements_with_gating(
        &self,
        facts: &HostFacts,
    ) -> Result<Vec<(Statement, Origin, bool)>> {
        let mut out = Vec::new();
        Self::walk(&self.items, facts, false, &mut out)?;
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
        conditional: bool,
        out: &mut Vec<(Statement, Origin, bool)>,
    ) -> Result<()> {
        for item in items {
            match item {
                Item::Statement(s, o) => out.push((s.clone(), o.clone(), conditional)),
                Item::Block(Block::Module(_, body), _) => {
                    Self::walk(body, facts, conditional, out)?
                }
                Item::Block(Block::When(pred, body), origin) => {
                    let hit = eval_when(pred, facts).map_err(|e| {
                        GrammarError::new(origin.clone(), e.to_string()).with_hint(
                            "`when` keys are os, arch, host, hostname, family, or `$name` for \
                             a variable; operators are ==, != and `in [a, b]`.",
                        )
                    })?;
                    if hit {
                        Self::walk(body, facts, true, out)?;
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
pub fn parse_document(
    file: &Path,
    body: &str,
    backends: &dyn BackendNames,
) -> Result<Document> {
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
            return Err(GrammarError::new(origin, "`}` closes a block that was never opened"));
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
        return Err(GrammarError::new(
            Origin::new(file, 0),
            "a `{` block is never closed",
        )
        .with_hint("add the matching `}`."));
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
            return Err(GrammarError::new(origin.clone(), "`module` block has no name"));
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
        return Ok(Item::Block(Block::Module(name.to_string(), body), origin.clone()));
    }

    if let Some(pred) = header.strip_prefix("when ") {
        let pred = pred.trim();
        if pred.is_empty() {
            return Err(GrammarError::new(origin.clone(), "`when` block has no condition"));
        }
        let body = parse_items(file, lines, backends, true)?;
        return Ok(Item::Block(Block::When(pred.to_string(), body), origin.clone()));
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
    let mut stmt = statement::parse(origin, header, backends).map_err(|e| unrecognised(e, header))?;

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
        | Statement::Setting(_, o) => o,
        Statement::Repo { .. }
        | Statement::Use(_)
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
        assert_eq!(d.options.one("after_install"), Some("./setup.sh --flag=a,b"));
    }

    #[test]
    fn a_key_given_twice_in_a_block_makes_a_list() {
        let out = stmts(
            "apt:nginx {\n  requires = apt:libfoo\n  requires = apt:libbar\n}\n",
        );
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
        assert!(err.hint.unwrap().contains("os, arch, host, hostname, family"));
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
        assert!(matches!(out[8], Statement::Use(Reference::Module(_))));
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
            ("apt:jq@hold@version=1.6", "apt:jq@hold {\n  version = 1.6\n}"),
            ("apt:nginx@colour=blue", "apt:nginx {\n  colour = blue\n}"),
            ("apt:nginx@lease=2h", "apt:nginx {\n  lease = 2h\n}"),
            ("apt:curl@formats=deb", "apt:curl {\n  formats = deb\n}"),
            (
                "apt:curl@expires=2h",
                "apt:curl {\n  expires = 2h\n}",
            ),
        ] {
            assert!(doc(&format!("{}\n", short)).is_err(), "short form allowed `{}`", short);
            assert!(doc(&format!("{}\n", block)).is_err(), "block form allowed `{}`", block);
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

    #[test]
    fn statements_carry_where_they_came_from() {
        // II.7 rule 5 needs this: a conflict must name BOTH files.
        let d = doc("# header\napt:curl\n").unwrap();
        let out = d.statements_for(&facts()).unwrap();
        assert_eq!(out[0].1.line, 2);
        assert_eq!(out[0].1.file, PathBuf::from("modules/dev.txt"));
    }
}
