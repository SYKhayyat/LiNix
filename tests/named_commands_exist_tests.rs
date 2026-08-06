//! Every command LiNix names, in text a person reads or a machine runs, is a command LiNix has.
//!
//! Eight review rounds named "a check that cannot fail" as this repository's signature defect,
//! and the mechanism behind it has never been written down. It is this: **the gate is drawn
//! around the artifact that was under review, and the property escapes through the next copy of
//! the fact.**
//!
//! `help_map_tests.rs` compares the map in `args.rs` to `--help` in both directions, and its own
//! header cites `undo` — a command deleted while two exemption lists went on naming it — as the
//! reason it exists. That gate works. It is drawn around `args.rs`. Meanwhile, with no top-level
//! `status`, `doctor`, `undo` or `audit` verb in the program:
//!
//! - `app/fleet.rs` asked every host for `linix status --json`, so `linix fleet` could not
//!   return "in sync" for a correctly-installed machine — the command it ran did not exist.
//! - `scripts/install.sh` and `install.ps1` ran `doctor` to vouch for the binary they had just
//!   built, and signed off by recommending `status`. The first thing a new user runs.
//! - `verbs/cleanup.rs` printed `Undo with 'linix undo <id>'` after `purge-undeclared`, the most
//!   destructive command in the program.
//! - `cli/args.rs` itself — inside the file `help_map_tests.rs` gates — documented `--security`
//!   as upgrading what `linix audit` reports.
//!
//! One fact, six copies, one gate around one copy. Fixing those six strings is not the answer;
//! a seventh will be written next week. So this gate is drawn around the property instead:
//!
//! > **A lowercase `linix` at command position names a live path through the clap surface.**
//!
//! ## The convention this depends on, and enforces
//!
//! Prose calls the product `LiNix`. A lowercase `linix` that begins a line or follows a quote,
//! a backtick or a shell operator is an *invocation*, and is checked. That distinction is what
//! lets the scan be exact instead of maintaining a list of English words to ignore — a list
//! which would be one more artifact-shaped exemption, and would rot the same way.
//!
//! ## What is checked, and what is not
//!
//! The truth is clap's own command tree, read through `CommandFactory` — names, aliases, and
//! nested subcommands. Never a list maintained beside it; that shape is what `known_subcommands`
//! in `main.rs` already avoids and what `help_map_tests.rs` was written to catch.
//!
//! Covered: everything a user reads or a machine runs — `src/`, `tests/`, `scripts/`, `docker/`,
//! `examples/`, `.github/`, and `readme.md`.
//!
//! Not covered: `docs/`. It is a record — a changelog, a bug tracker and a decision register —
//! and a record must be free to say `linix doctor` when it is describing the day `linix doctor`
//! was deleted. Forcing those lines live would make the history lie. `readme.md` is in, because
//! it is the one document a user reads as instructions rather than as a record.
//!
//! Also not covered: an argv built from `Command::new(env!("CARGO_BIN_EXE_linix"))`, which is
//! argument-vector shaped rather than text shaped. Those run in the suite, so clap answers them
//! directly with "unrecognized subcommand" — a wrong name there fails loudly on its own.

use clap::CommandFactory;
use linix::cli::args::Cli;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------
// The surface: what the program actually answers to.
// ---------------------------------------------------------------------------------------------

/// One node of clap's command tree: the subcommands reachable from here, by every name they
/// answer to.
#[derive(Default, Clone)]
struct Surface {
    children: BTreeMap<String, Surface>,
}

impl Surface {
    /// Read a command's subcommands, recursively, indexing each under its name *and* each of its
    /// aliases — `linix tui` is `linix history`, and a gate that did not know that would report
    /// the alias as an invention.
    fn read(cmd: &clap::Command) -> Self {
        let mut node = Surface::default();
        for sub in cmd.get_subcommands() {
            let child = Surface::read(sub);
            for name in std::iter::once(sub.get_name()).chain(sub.get_all_aliases()) {
                node.children.insert(name.to_string(), child.clone());
            }
        }
        node
    }

    fn live() -> Self {
        let mut root = Surface::read(&Cli::command());
        // clap injects `help` when it builds the command, which is after the derive this reads.
        // It takes any command name as its argument, so it gets the root's own children: a typo
        // in `linix help sync` is the same defect as a typo in `linix sync`.
        let below = root.clone();
        root.children.entry("help".to_string()).or_insert(below);
        root
    }
}

/// A word that could be a subcommand: the shape clap gives every verb in this program.
///
/// Anything else — `<command>`, `{}`, `BACKEND:NAME`, `$pkg`, `HEAD~3` — is a placeholder or an
/// argument, and neither can be resolved against the surface.
fn is_bare_word(w: &str) -> bool {
    let mut chars = w.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Walk an invocation's words down the surface. Returns the first word that had to be a
/// subcommand and was not.
///
/// The walk stops as soon as it reaches a command with no subcommands of its own: from there on
/// the words are positional arguments (`linix check drift`, `linix adopt service`,
/// `linix lock scripts`), and a gate that kept walking would report every argument as a typo.
fn first_unknown(root: &Surface, words: &[String]) -> Option<String> {
    let mut node = root;
    for w in words {
        // A global flag may sit before the subcommand (`linix --dry-run sync`), and a
        // subcommand's own flags sit before its sub-subcommand.
        if w.starts_with('-') {
            continue;
        }
        if node.children.is_empty() {
            return None;
        }
        if !is_bare_word(w) {
            return None;
        }
        match node.children.get(w.as_str()) {
            Some(child) => node = child,
            None => return Some(w.clone()),
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// The scan: finding invocations in text.
// ---------------------------------------------------------------------------------------------

/// A `linix …` invocation found in the tree, with enough of itself to be recognised in the
/// failure message.
#[derive(Debug)]
struct Invocation {
    file: String,
    line: usize,
    words: Vec<String>,
    quoted: String,
}

/// Shell and PowerShell spellings of "the path to the binary". `install.sh` runs
/// `"$LINIX" doctor`, which is an invocation and reads nothing like one.
const BINARY_VARS: &[&str] = &["${LINIX}", "$LINIX_BIN", "$LINIX", "$linix"];

/// Is a lowercase `linix` here being invoked, rather than being talked about?
///
/// Talked about, it follows a word — *the* linix binary, *this* linix speaks schema 2. Invoked,
/// it opens a line or follows a delimiter or a shell operator. A line that carries only comment
/// or list decoration before the token (`//   linix why …`, `#   linix unmanage …`,
/// `- linix sync`) is still a line that begins with the invocation.
fn at_command_position(before: &str) -> bool {
    let trimmed = before.trim_end();
    match trimmed.chars().last() {
        None => true,
        Some('`' | '"' | '\'' | '(' | '|' | ';' | '&') => true,
        Some(_) => trimmed
            .chars()
            .all(|c| matches!(c, '/' | '#' | '*' | '-' | '>' | '+' | '\\' | ' ' | '\t')),
    }
}

/// The words following a binary reference, up to whatever closes the invocation.
///
/// Each token is cut at the first character that ends a command — a closing quote or backtick, a
/// pipe, a redirect, a separator, a sentence's full stop — and collection stops there. Six words
/// is past the deepest path this program has and past every flag that precedes one.
fn words_after(rest: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in rest.split_whitespace() {
        let cut = raw.find(|c| {
            matches!(
                c,
                '`' | '"' | '\'' | '|' | ';' | '&' | ')' | ',' | '.' | '>' | '<' | '\\'
            )
        });
        let head = match cut {
            Some(i) => &raw[..i],
            None => raw,
        };
        if !head.is_empty() {
            out.push(head.to_string());
        }
        if cut.is_some() || out.len() >= 6 {
            break;
        }
    }
    out
}

/// Every invocation on one line of text.
fn invocations_in_line(line: &str) -> Vec<(Vec<String>, String)> {
    let mut found = Vec::new();
    let mut at = 0usize;

    while at < line.len() {
        // Whichever comes first: a bare `linix`, or one of the variables holding its path.
        let bare = line[at..].find("linix").map(|i| (at + i, "linix"));
        let var = BINARY_VARS
            .iter()
            .filter_map(|v| line[at..].find(v).map(|i| (at + i, *v)))
            .min_by_key(|(i, _)| *i);
        let Some((start, token)) = [bare, var]
            .into_iter()
            .flatten()
            .min_by_key(|(i, tok)| (*i, std::cmp::Reverse(tok.len())))
        else {
            break;
        };

        let after = start + token.len();
        at = after;

        // `linix` inside a longer word (`linix-plan.json`, `LINIX_BIN`) is not an invocation, and
        // neither is `linix:` — the config repo's own name in a path.
        let next = line[after..].chars().next();
        if next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        if token == "linix" && !at_command_position(&line[..start]) {
            continue;
        }

        // A *variable* reference may be quoted or braced (`"$LINIX" adopt`), and that closing
        // punctuation belongs to the reference rather than to the first word. A bare `linix`
        // gets no such courtesy: in `A "linix" shim would overwrite …` the closing quote is
        // what says the name is being quoted, not run.
        let rest = if token == "linix" {
            &line[after..]
        } else {
            line[after..].trim_start_matches(['"', '\'', '}'])
        };
        let words = words_after(rest);
        if !words.is_empty() {
            let quoted: String = line
                .chars()
                .skip(line[..start].chars().count())
                .take(60)
                .collect();
            found.push((words, quoted.trim_end().to_string()));
        }
    }
    found
}

/// Every file under the covered roots whose contents are text a person reads or a machine runs.
fn covered_files() -> Vec<PathBuf> {
    const ROOTS: &[&str] = &[
        "src",
        "tests",
        "scripts",
        "docker",
        "examples",
        ".github",
        "readme.md",
    ];
    const EXTENSIONS: &[&str] = &["rs", "sh", "ps1", "md", "toml", "txt", "yml", "yaml"];

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                walk(&p, out);
            } else {
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                // Dockerfiles carry no extension and carry `RUN linix …`.
                if EXTENSIONS.contains(&ext) || name.starts_with("Dockerfile") {
                    out.push(p);
                }
            }
        }
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for r in ROOTS {
        let p = root.join(r);
        if p.is_dir() {
            walk(&p, &mut out);
        } else if p.is_file() {
            out.push(p);
        }
    }
    out
}

fn scan() -> Vec<Invocation> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();

    for path in covered_files() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        // The one file that cannot obey this rule is the one that states it: a gate asserting a
        // string is absent has to spell the string out — in its own header, and in the oracle
        // below that feeds it every defect it was written to catch. Named by `file!()` rather
        // than by a path literal, so renaming this file cannot silently widen the hole.
        if rel == file!().replace('\\', "/") {
            continue;
        }
        // Production code only, matching `removal_guard_enumeration_tests.rs`: a unit test's
        // fixtures are not text anybody reads as instructions. Integration tests under `tests/`
        // have no such marker and are scanned whole.
        let production_only = rel.starts_with("src/");

        for (i, line) in text.lines().enumerate() {
            if production_only && line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            for (words, quoted) in invocations_in_line(line) {
                out.push(Invocation {
                    file: rel.clone(),
                    line: i + 1,
                    words,
                    quoted,
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The gate.
// ---------------------------------------------------------------------------------------------

#[test]
fn every_command_this_repo_names_is_a_command_this_repo_has() {
    let surface = Surface::live();
    let mut problems = Vec::new();

    for inv in scan() {
        if let Some(unknown) = first_unknown(&surface, &inv.words) {
            problems.push(format!(
                "{}:{}  `{}`\n    `{}` is not a command here.",
                inv.file, inv.line, inv.quoted, unknown
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "{} invocation(s) name a command the program does not have:\n\n{}\n\n\
         Each of these is a string a user is told to type or a machine is about to run. \
         Rename it to the live command, or — if the sentence is prose about the product rather \
         than an invocation — write the product's name as `LiNix`, which is what tells this gate \
         the two apart.",
        problems.len(),
        problems.join("\n\n")
    );
}

// ---------------------------------------------------------------------------------------------
// The same property where the prefix cannot reach it: `readme.md`'s verb tables.
// ---------------------------------------------------------------------------------------------

/// A markdown table's rows, and which of them name a verb.
struct VerbTable {
    line: usize,
    /// Rows whose first cell is nothing but backticked command paths: `` `sync` ``,
    /// `` `install` / `uninstall` ``, `` `check health` ``. Each entry is the line and the
    /// paths that cell names, each path already split into its words.
    rows: Vec<(usize, Vec<Vec<String>>)>,
}

/// The tables in `readme.md` that list verbs, found by what they contain rather than by where
/// they are.
///
/// A verb table is one whose candidate rows are *mostly* live commands. That is what separates
/// it from the config-key and option tables in the same file, without a heading match or a line
/// range — either of which would be a gate drawn around an artifact again.
fn verb_tables(text: &str, surface: &Surface) -> Vec<VerbTable> {
    let mut tables: Vec<VerbTable> = Vec::new();
    let mut current: Option<VerbTable> = None;

    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if !t.starts_with('|') {
            if let Some(table) = current.take() {
                tables.push(table);
            }
            continue;
        }
        let table = current.get_or_insert(VerbTable {
            line: i + 1,
            rows: Vec::new(),
        });

        let Some(cell) = t.trim_start_matches('|').split('|').next() else {
            continue;
        };
        let cell = cell.trim();
        if cell.is_empty() || cell.chars().all(|c| c == '-' || c == ':' || c == ' ') {
            continue; // the header separator, or an empty leading cell
        }
        // Only a cell that is entirely backticked command paths separated by `/` is a
        // candidate. A sentence, a config key with a dot, a `[section]` — none of those are
        // verbs, and none of them survive this.
        let paths: Vec<Vec<String>> = cell
            .split('/')
            .map(|p| {
                p.trim()
                    .trim_matches('`')
                    .split_whitespace()
                    .map(str::to_string)
                    .collect()
            })
            .collect();
        if cell.starts_with('`')
            && cell.ends_with('`')
            && !paths.is_empty()
            && paths
                .iter()
                .all(|p| !p.is_empty() && p.iter().all(|w| is_bare_word(w)))
        {
            table.rows.push((i + 1, paths));
        }
    }
    if let Some(table) = current.take() {
        tables.push(table);
    }

    tables
        .into_iter()
        .filter(|t| {
            let live = t
                .rows
                .iter()
                .filter(|(_, paths)| paths.iter().all(|p| first_unknown(surface, p).is_none()))
                .count();
            // Three rows is enough to tell a verb table from a coincidence, and "mostly live"
            // is what identifies one. The count below stops a table from rotting past the
            // threshold and quietly leaving the gate.
            t.rows.len() >= 3 && live * 2 >= t.rows.len()
        })
        .collect()
}

/// How many verb tables `readme.md` has.
///
/// Pinned, because the rule above identifies a verb table by the rows that are still right — so
/// a table that rotted badly enough would stop being recognised as one and would leave the gate
/// in silence. A count cannot do that: deleting a verb table, or letting one decay past
/// recognition, fails here either way.
const README_VERB_TABLES: usize = 5;

#[test]
fn the_readme_verb_tables_name_only_commands_that_exist() {
    let readme = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("readme.md"))
        .expect("readme.md should be readable");
    let surface = Surface::live();
    let tables = verb_tables(&readme, &surface);

    assert_eq!(
        tables.len(),
        README_VERB_TABLES,
        "readme.md has {} verb table(s), and {} were expected (they begin at lines {:?}).\n\
         If one was deliberately added or removed, move this number. If one simply stopped \
         looking like a verb table, it has rotted past the point where this gate can see it — \
         which is the case this number exists to catch.",
        tables.len(),
        README_VERB_TABLES,
        tables.iter().map(|t| t.line).collect::<Vec<_>>()
    );

    let mut problems = Vec::new();
    for table in &tables {
        for (line, paths) in &table.rows {
            for path in paths {
                if let Some(unknown) = first_unknown(&surface, path) {
                    problems.push(format!(
                        "readme.md:{}  the verb table beginning at line {} lists `{}`, and `{}` \
                         is not a command.",
                        line,
                        table.line,
                        path.join(" "),
                        unknown
                    ));
                }
            }
        }
    }

    assert!(
        problems.is_empty(),
        "{}\n\nreadme.md:648 argues that `--help` cannot go stale the way a README can. These \
         rows are the README going stale.",
        problems.join("\n")
    );
}

// ---------------------------------------------------------------------------------------------
// The oracle: the instrument, tested before it is trusted.
// ---------------------------------------------------------------------------------------------

/// GRADE, repeatedly: do not test your own oracle by assuming it works. A scan whose patterns
/// stopped matching would find nothing and pass the gate above for the worst possible reason.
#[test]
fn the_scan_can_see_what_it_is_looking_for() {
    let surface = Surface::live();

    let unknown = |line: &str| -> Vec<String> {
        invocations_in_line(line)
            .into_iter()
            .filter_map(|(words, _)| first_unknown(&surface, &words))
            .collect()
    };

    // Every shape the six real defects were written in. These are the literal lines, as they
    // stood before this gate existed.
    assert_eq!(
        unknown(r#"                match ssh_capture(&host, "linix status --json").await {"#),
        ["status"],
        "the string `fleet` sent to every host"
    );
    assert_eq!(
        unknown(r#""$LINIX" doctor || true"#),
        ["doctor"],
        "the health check that vouches for a freshly installed binary"
    );
    assert_eq!(
        unknown(r#"say "done. Try \`linix status\` or \`linix doctor\`.""#),
        ["status", "doctor"],
        "both halves of one sentence"
    );
    assert_eq!(
        unknown(r#"& $linix doctor"#),
        ["doctor"],
        "the PowerShell spelling of the same call"
    );
    assert_eq!(
        unknown(r#"        println!("Undo with `linix undo {}`.", id);"#),
        ["undo"],
        "what `purge-undeclared` told the user to run"
    );
    assert_eq!(
        unknown(r#"    /// Upgrade only packages that `linix audit` reports as vulnerable"#),
        ["audit"],
        "a dead command inside the very file `help_map_tests.rs` gates"
    );
    assert_eq!(
        unknown(r#"                 linix status        see every destination first\n  \"#),
        ["status"],
        "a line of a multi-line refusal message, which no closing delimiter precedes"
    );
    assert_eq!(
        unknown("linix undo               # interactive snapshot gallery"),
        ["undo"],
        "a line in one of readme.md's fenced blocks"
    );

    // Depth: a wrong word in a nested path is the same defect one level down.
    assert_eq!(unknown("`linix git stauts`"), ["stauts"]);
    assert_eq!(unknown("`linix snapshot delete`"), ["delete"]);

    // And the controls, without which every assertion above would hold for a scan that
    // reports everything.
    assert!(unknown("`linix check drift`").is_empty(), "a live section");
    assert!(unknown("`linix git status`").is_empty(), "a live sub-verb");
    assert!(unknown("`linix tui`").is_empty(), "an alias clap declares");
    assert!(unknown("`linix help sync`").is_empty(), "clap's own verb");
    assert!(
        unknown("`linix adopt service`").is_empty(),
        "a positional argument that happens to share a verb's name"
    );
    assert!(
        unknown("`linix lock scripts` approves every hook").is_empty(),
        "a value enum, followed by prose inside the same backticks"
    );
    assert!(
        unknown("`linix upgrade --profile dev`").is_empty(),
        "a flag's value"
    );
    assert!(
        unknown("`linix --dry-run sync`").is_empty(),
        "a global flag before the subcommand"
    );
    assert!(
        unknown("`linix <command> --allow-mass-removal`").is_empty(),
        "a placeholder, which cannot be resolved and must not be guessed at"
    );
    assert!(
        unknown("cd $(linix path)").is_empty(),
        "a command substitution"
    );

    // Prose, which is the whole reason the product is spelled `LiNix` in a sentence.
    assert!(unknown("Every hook actually invokes the linix binary.").is_empty());
    assert!(unknown("on startup linix reads the name it was called by").is_empty());
    assert!(unknown("this linix speaks schema 2").is_empty());
    assert!(unknown("Rebuilding linix from {repo} via cargo").is_empty());
    assert!(
        unknown("the linix-plan.json written by an earlier run").is_empty(),
        "`linix` inside a longer word"
    );
    assert!(
        unknown(r#"A "linix" shim would overwrite linix itself with itself"#).is_empty(),
        "a quoted *name*, closed by the quote that opened it"
    );

    // This file is skipped, and the skip is only sound while the file would otherwise be read.
    // An exemption nobody re-derives is how `undo` stayed in two lists after it was deleted.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        covered_files().iter().any(|p| p
            .strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
            == file!().replace('\\', "/")),
        "this file is excluded from the scan by name, and the scan no longer reaches it — so \
         the exclusion is now hiding nothing and proving nothing"
    );

    // The scan must still find things in the real tree. A floor, not a count — the ledger above
    // is what answers "is every one of them live".
    let found = scan();
    assert!(
        found.len() > 200,
        "the scan found only {} invocation(s) across the tree, which is fewer than this repo \
         writes down; the patterns have stopped matching",
        found.len()
    );
    assert!(
        found.iter().any(|i| i.file == "scripts/install.sh"),
        "install.sh is what a user pipes from the web, and it is the file this gate exists for"
    );
    assert!(
        found.iter().any(|i| i.file == "readme.md"),
        "readme.md is the only user-facing document in the repo"
    );
}

/// The surface must be the program's, and it must be whole — a `Surface` that read nothing
/// would make every invocation above a violation, and a `Surface` that lost its nesting would
/// make none of them one.
#[test]
fn the_surface_is_the_program() {
    let surface = Surface::live();
    assert!(
        surface.children.len() > 50,
        "clap declares more than fifty subcommands; this read {}",
        surface.children.len()
    );
    assert!(
        surface.children["git"].children.contains_key("status"),
        "nested subcommands must be read, or `linix git status` would look like `linix status`"
    );
    assert!(
        surface.children["check"].children.is_empty(),
        "`check` takes a section as an argument, not as a subcommand — the walk depends on it"
    );
    assert!(
        surface.children.contains_key("hook-record"),
        "a hidden command is still a command the program answers to"
    );
    assert!(
        !surface.children.contains_key("status"),
        "there is no top-level `status`; if one was added, this whole gate has less to say"
    );
}
