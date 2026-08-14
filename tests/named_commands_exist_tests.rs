//! Every command Shall names, in text a person reads or a machine runs, is a command Shall has.
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
//! - `app/fleet.rs` asked every host for `shall status --json`, so `shall fleet` could not
//!   return "in sync" for a correctly-installed machine — the command it ran did not exist.
//! - `scripts/install.sh` and `install.ps1` ran `doctor` to vouch for the binary they had just
//!   built, and signed off by recommending `status`. The first thing a new user runs.
//! - `verbs/cleanup.rs` printed `Undo with 'shall undo <id>'` after `purge-undeclared`, the most
//!   destructive command in the program.
//! - `cli/args.rs` itself — inside the file `help_map_tests.rs` gates — documented `--security`
//!   as upgrading what `shall audit` reports.
//!
//! One fact, six copies, one gate around one copy. Fixing those six strings is not the answer;
//! a seventh will be written next week. So this gate is drawn around the property instead:
//!
//! > **A lowercase `shall` at command position names a live path through the clap surface.**
//!
//! ## The convention this depends on, and enforces
//!
//! Prose calls the product `Shall`. A lowercase `shall` that begins a line or follows a quote,
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
//! `examples/`, `.github/`, and `README.md`.
//!
//! ## `docs/`, and the weaker property that fits it
//!
//! `docs/` was out of scope, on the argument that it is a record — a changelog, a bug tracker and
//! a decision register — and a record has to be free to name a command on the day that command
//! was deleted. Forcing those lines live would make the history lie.
//!
//! That argument is right about *this* rule and was mistaken for a reason to check nothing. It
//! left 2.5 MB of specification unscanned, and inside it a **CLOSED owner ruling whose entire
//! stated justification was a command the program does not have** — `bugs.md`'s F4, which
//! declines to wire `--help` to the registry because "`doctor` already carries the live count".
//! `doctor` was folded into `check <section>` in S38. The code was swept; the ruling that rests
//! on it was not, because nothing read it.
//!
//! So `docs/` is checked against the property a record can actually satisfy:
//!
//! > **A dead command named in `docs/` is a command the spec says is dead.**
//!
//! The register is `target-state.md` II.17 *Deleted*, read as data rather than restated here —
//! the same reason `grammar_table_matches_the_spec_tests.rs` reads `KEYWORDS` through the
//! parser's accessors. A record keeps its freedom: the register may write `shall doctor` as often
//! as it likes, because II.17 says `doctor` is gone. What it may no longer do is name a command
//! that is neither live nor recorded as dead — which is what a stale instruction looks like.
//!
//! `docs/attic/` is out. It holds one file, and its first line tells the reader not to read it;
//! a gate reading it anyway would be the only thing in the tree that does.
//!
//! Also not covered: an argv built from `Command::new(env!("CARGO_BIN_EXE_shall"))`, which is
//! argument-vector shaped rather than text shaped. Those run in the suite, so clap answers them
//! directly with "unrecognized subcommand" — a wrong name there fails loudly on its own.

use clap::CommandFactory;
use shall::cli::args::Cli;
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
    /// aliases — `shall tui` is `shall history`, and a gate that did not know that would report
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
        // in `shall help sync` is the same defect as a typo in `shall sync`.
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
/// the words are positional arguments (`shall check drift`, `shall adopt service`,
/// `shall lock scripts`), and a gate that kept walking would report every argument as a typo.
fn first_unknown(root: &Surface, words: &[String]) -> Option<String> {
    let mut node = root;
    for w in words {
        // A global flag may sit before the subcommand (`shall --dry-run sync`), and a
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

/// A `shall …` invocation found in the tree, with enough of itself to be recognised in the
/// failure message.
#[derive(Debug)]
struct Invocation {
    file: String,
    line: usize,
    words: Vec<String>,
    quoted: String,
}

/// Shell and PowerShell spellings of "the path to the binary". `install.sh` runs
/// `"$SHALL" doctor`, which is an invocation and reads nothing like one.
const BINARY_VARS: &[&str] = &["${SHALL}", "$SHALL_BIN", "$SHALL", "$shall"];

/// Is a lowercase `shall` here being invoked, rather than being talked about?
///
/// Talked about, it follows a word — *the* shall binary, *this* shall speaks schema 2. Invoked,
/// it opens a line or follows a delimiter or a shell operator. A line that carries only comment
/// or list decoration before the token (`//   shall why …`, `#   shall unmanage …`,
/// `- shall sync`) is still a line that begins with the invocation.
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
        // Whichever comes first: a bare `shall`, or one of the variables holding its path.
        let bare = line[at..].find("shall").map(|i| (at + i, "shall"));
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

        // `shall` inside a longer word (`shall-plan.json`, `SHALL_BIN`) is not an invocation, and
        // neither is `shall:` — the config repo's own name in a path.
        let next = line[after..].chars().next();
        if next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        if token == "shall" && !at_command_position(&line[..start]) {
            continue;
        }

        // A *variable* reference may be quoted or braced (`"$SHALL" adopt`), and that closing
        // punctuation belongs to the reference rather than to the first word. A bare `shall`
        // gets no such courtesy: in `A "shall" shim would overwrite …` the closing quote is
        // what says the name is being quoted, not run.
        let rest = if token == "shall" {
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
        "README.md",
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
                // Dockerfiles carry no extension and carry `RUN shall …`.
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

/// Every file under `docs/` that carries prose, except the attic.
///
/// `docs/attic/lessons.md` opens by telling every reader not to read it, and a scan that read it
/// would be holding the one file in the tree to a rule the file exists to opt out of.
fn documentation_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.file_name().and_then(|n| n.to_str()) == Some("attic") {
                continue;
            }
            if p.is_dir() {
                walk(&p, out);
            } else if matches!(p.extension().and_then(|e| e.to_str()), Some("md" | "org")) {
                out.push(p);
            }
        }
    }

    let mut out = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("docs"),
        &mut out,
    );
    out
}

/// The command paths `target-state.md` II.17 records as deleted.
///
/// Read from the spec rather than restated here. A second copy of this list would be free to be
/// wrong in the direction that hides a defect — it would let a command be deleted from the
/// program, forgotten by II.17, and still named all over `docs/` without anything noticing, which
/// is the exact failure this gate exists to close.
///
/// II.17 writes each entry as a backticked path, sometimes with the replacement beside it
/// (`` `migrate` (→ `adopt`) ``). The parenthetical names a *live* command by construction, so
/// taking every backticked path in the section is safe: a live name in this set changes nothing,
/// because the surface answers for it first.
fn deleted_register() -> BTreeMap<String, Vec<String>> {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/spec/target-state.md"),
    )
    .expect("docs/spec/target-state.md should be readable");

    let mut out = BTreeMap::new();
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("## II.17") {
            inside = true;
            continue;
        }
        // The section runs to the next heading. `**Commands:**` and `**Flags:**` are inside it.
        if inside && t.starts_with("## ") {
            break;
        }
        if !inside {
            continue;
        }
        for chunk in t.split('`').skip(1).step_by(2) {
            let words: Vec<String> = chunk.split_whitespace().map(str::to_string).collect();
            // A flag, a config key, or prose that happened to be backticked. Only a path of bare
            // words can name a command.
            if !words.is_empty() && words.iter().all(|w| is_bare_word(w)) {
                out.insert(words.join(" "), words);
            }
        }
    }
    out
}

/// Words that appear at command position in `docs/` and are not commands at all — neither live
/// nor deleted. Each is a deliberate non-invocation, and each carries the reason it is one.
///
/// This is a ledger, not a filter: it is asserted to be exactly this size, so a new entry cannot
/// arrive by accident, and an entry that stops occurring cannot linger.
const NOT_AN_INVOCATION: &[(&str, &str)] = &[
    (
        "nosuchcommand",
        "the repo's canonical name for a command that does not exist — it is the *subject* of \
         every exit-code test, so a gate that demanded it be real would be demanding the opposite \
         of what the tests prove",
    ),
    (
        "refresh",
        "a verb proposed and declined — \"a named composition of existing verbs: \
         `shall refresh` = `sync`, then `upgrade`\". Never built, so never deleted; II.17 records \
         what was removed, not what was turned down. The proposal document it came from was cut \
         with the rest of the record corpus (`Y21`); the register's U-series entries are where \
         the decision now lives",
    ),
];

/// A record naming a command that never existed here, where the sentence itself is the report
/// that it never existed.
///
/// These are not deletions and II.17 is right not to carry them: `backends` was a pre-v7 verb in
/// a script this repo deleted, `config path` and `config edit` are sub-verbs, and `setup` is this
/// gate's own catch quoted back in the session that fixed it. Every one is pinned to its line, so
/// an exemption cannot drift onto a sentence that stopped being a record.
const RECORDED_AS_ABSENT: &[(&str, usize, &str)] = &[];

/// Part II naming a command the program does not have.
///
/// **These are findings, not exemptions**, and they are the owner's to resolve: `CLAUDE.md` says
/// a spec that looks wrong is reported rather than edited. Each entry names the site and what the
/// live answer appears to be, so the ruling is a one-line edit rather than an investigation.
///
/// Every one of these was found by this gate on the run that introduced it — which is the whole
/// argument for pointing it at `docs/`. The list is asserted exact and shrink-only: closing one
/// means deleting a line here, and nothing can be added without a build failure to argue about
/// first.
const PART_II_LOOKS_WRONG: &[(&str, &str, &str)] = &[(
    "shim",
    "docs/spec/target-state.md:2217",
    "II.16's own table records `shall shim jq --source cargo:jq` becoming the line \
         `shim:jq@source=cargo:jq` — so the command was deleted, and II.17's register does not \
         say so. The gap has a live cost: `bugs.md:76` still carries \"`shall shim --source` is \
         required, documented, and thrown away. **(verified)**\" as an open bug against a command \
         that does not exist, which is F4's failure a second time",
)];

fn scan() -> Vec<Invocation> {
    scan_files(covered_files())
}

/// Every `shall …` invocation in `docs/`, outside the archive.
fn scan_documentation() -> Vec<Invocation> {
    scan_files(documentation_files())
}

fn scan_files(files: Vec<PathBuf>) -> Vec<Invocation> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();

    for path in files {
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
         than an invocation — write the product's name as `Shall`, which is what tells this gate \
         the two apart.",
        problems.len(),
        problems.join("\n\n")
    );
}

// ---------------------------------------------------------------------------------------------
// The same property one notch weaker, where the text is a record: `docs/`.
// ---------------------------------------------------------------------------------------------

/// Is this invocation accounted for — live, or recorded as deleted?
///
/// The register holds command *paths*, so `config path` and `config` are both answerable. The
/// walk is the same one the surface gets: leading flags skipped, first bare word onwards.
fn accounted_for(register: &BTreeMap<String, Vec<String>>, words: &[String]) -> bool {
    let live: Vec<&String> = words.iter().filter(|w| !w.starts_with('-')).collect();
    // Longest path first: `config path` must win over `config`, or a deleted sub-verb would be
    // excused by its live parent.
    (1..=live.len()).rev().any(|n| {
        let path = live[..n]
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        register.contains_key(&path)
    })
}

#[test]
fn a_dead_command_named_in_the_docs_is_a_command_the_spec_says_is_dead() {
    let surface = Surface::live();
    let register = deleted_register();
    let mut problems = Vec::new();

    for inv in scan_documentation() {
        let Some(unknown) = first_unknown(&surface, &inv.words) else {
            continue;
        };
        if accounted_for(&register, &inv.words) {
            continue;
        }
        if NOT_AN_INVOCATION.iter().any(|(w, _)| *w == unknown) {
            continue;
        }
        if PART_II_LOOKS_WRONG.iter().any(|(w, _, _)| *w == unknown) {
            continue;
        }
        if RECORDED_AS_ABSENT
            .iter()
            .any(|(f, l, _)| *f == inv.file && *l == inv.line)
        {
            continue;
        }
        problems.push(format!(
            "{}:{}  `{}`\n    `{}` is neither a command this program has nor one II.17 records \
             as deleted.",
            inv.file, inv.line, inv.quoted, unknown
        ));
    }

    assert!(
        problems.is_empty(),
        "{} invocation(s) in `docs/` name a command that is neither live nor recorded as \
         dead:\n\n{}\n\nThree things this can be, and they want opposite fixes. If the command \
         was deleted, `target-state.md` II.17 is the register and it is missing an entry — the \
         record stays as written and II.17 gains a line. If the command is live under another \
         name, the sentence is a stale instruction and should name the live verb. If the line is \
         prose about the product rather than an invocation, write `Shall`, which is what tells \
         this gate the two apart.",
        problems.len(),
        problems.join("\n\n")
    );
}

/// The register must be the spec's, and it must be whole.
///
/// A `deleted_register` that parsed nothing would excuse no invocation and redden the gate
/// honestly — but one that parsed *everything backticked in the file* would excuse all of them
/// in silence, which is the failure worth a test.
#[test]
fn the_deleted_register_is_the_specs_and_is_bounded() {
    let register = deleted_register();

    for expected in ["status", "doctor", "undo", "audit", "migrate", "prune"] {
        assert!(
            register.contains_key(expected),
            "II.17 records `{expected}` as deleted; the parse did not find it. Entries read: {:?}",
            register.keys().collect::<Vec<_>>()
        );
    }
    assert!(
        !register.contains_key("sync"),
        "the parse reached beyond II.17 and swallowed `sync`, which would excuse every \
         invocation in `docs/`. Entries read: {:?}",
        register.keys().collect::<Vec<_>>()
    );
    assert!(
        register.len() < 40,
        "II.17 lists a bounded set of deleted commands and the parse read {} entries, which \
         means it ran past the section",
        register.len()
    );

    // The excuse must excuse exactly what it claims to. Without these, every assertion above
    // holds for an `accounted_for` that answers `true` to everything — which would turn the
    // `docs/` gate into a scan that reads 2.5 MB and reports nothing, the failure this whole
    // file is about.
    let words = |s: &str| -> Vec<String> { s.split(' ').map(str::to_string).collect() };
    assert!(
        accounted_for(&register, &words("doctor --json")),
        "II.17 says `doctor` is deleted, and a record naming it must be excused"
    );
    assert!(
        !accounted_for(&register, &words("nosuchcommand")),
        "a word in neither the surface nor the register must not be excused, or the gate reports \
         nothing"
    );
    assert!(
        !accounted_for(&register, &words("config")),
        "`config` is live and is not in the register; excusing it here would mean the register \
         had swallowed the live surface"
    );
    // Depth: a deleted sub-verb must not be excused by a live parent, nor a live parent by the
    // presence of its deleted child.
    let mut nested = register.clone();
    nested.insert("config path".to_string(), words("config path"));
    assert!(
        accounted_for(&nested, &words("config path")),
        "the register holds paths, not just first words"
    );
    assert!(
        !accounted_for(&nested, &words("config elsewhere")),
        "a deleted sibling must not excuse a name nobody recorded"
    );

    // The two ledgers are exact, so neither can grow by accident nor rot after it empties.
    assert_eq!(
        NOT_AN_INVOCATION.len(),
        2,
        "a deliberate non-invocation was added or removed; each one is a claim that a word at \
         command position is not a command at all, and it should be argued rather than appended"
    );
    assert_eq!(
        PART_II_LOOKS_WRONG.len(),
        1,
        "this list is findings against the canonical spec, and it shrinks. If an owner ruled on \
         one, delete its line. If a new one appeared, the spec grew a stale instruction and that \
         is the thing to report — not the thing to record."
    );
    assert_eq!(
        RECORDED_AS_ABSENT.len(),
        0,
        "each entry claims a specific line is a record of a command's absence rather than an \
         instruction to run it; that claim is read, not appended to. All three lived in \
         `history.md`, which `Y21` deleted — the sentences went with the file, so the exemptions \
         went too rather than sitting here excusing nothing."
    );

    // Every finding must still be findable. An entry whose line was fixed without the entry being
    // deleted would sit here excusing nothing and claiming a defect that is gone — which is the
    // exact shape of `help_map_tests.rs` still exempting `undo` years after `undo` was deleted.
    let surface = Surface::live();
    let found = scan_documentation();
    for (word, whence, _) in PART_II_LOOKS_WRONG {
        assert!(
            found.iter().any(|inv| {
                first_unknown(&surface, &inv.words).as_deref() == Some(word)
                    && whence.starts_with(inv.file.as_str())
            }),
            "`{word}` is recorded as Part II naming a dead command, at {whence}, and the scan no \
             longer finds it there — so either it was fixed and this line should go, or the scan \
             stopped seeing it"
        );
    }
    for (file, line, claim) in RECORDED_AS_ABSENT {
        assert!(
            found
                .iter()
                .any(|inv| inv.file == *file && inv.line == *line),
            "{file}:{line} is exempted as a record ({claim}) and carries no invocation at all now"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The same property where the prefix cannot reach it: `README.md`'s verb tables.
// ---------------------------------------------------------------------------------------------

/// A markdown table's rows, and which of them name a verb.
struct VerbTable {
    line: usize,
    /// Rows whose first cell is nothing but backticked command paths: `` `sync` ``,
    /// `` `install` / `uninstall` ``, `` `check health` ``. Each entry is the line and the
    /// paths that cell names, each path already split into its words.
    rows: Vec<(usize, Vec<Vec<String>>)>,
}

/// The tables in `README.md` that list verbs, found by what they contain rather than by where
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

/// How many verb tables `README.md` has.
///
/// Pinned, because the rule above identifies a verb table by the rows that are still right — so
/// a table that rotted badly enough would stop being recognised as one and would leave the gate
/// in silence. A count cannot do that: deleting a verb table, or letting one decay past
/// recognition, fails here either way.
const README_VERB_TABLES: usize = 5;

#[test]
fn the_readme_verb_tables_name_only_commands_that_exist() {
    let readme = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("README.md should be readable");
    let surface = Surface::live();
    let tables = verb_tables(&readme, &surface);

    assert_eq!(
        tables.len(),
        README_VERB_TABLES,
        "README.md has {} verb table(s), and {} were expected (they begin at lines {:?}).\n\
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
                        "README.md:{}  the verb table beginning at line {} lists `{}`, and `{}` \
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
        "{}\n\nREADME.md:648 argues that `--help` cannot go stale the way a README can. These \
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
        unknown(r#"                match ssh_capture(&host, "shall status --json").await {"#),
        ["status"],
        "the string `fleet` sent to every host"
    );
    assert_eq!(
        unknown(r#""$SHALL" doctor || true"#),
        ["doctor"],
        "the health check that vouches for a freshly installed binary"
    );
    assert_eq!(
        unknown(r#"say "done. Try \`shall status\` or \`shall doctor\`.""#),
        ["status", "doctor"],
        "both halves of one sentence"
    );
    assert_eq!(
        unknown(r#"& $shall doctor"#),
        ["doctor"],
        "the PowerShell spelling of the same call"
    );
    assert_eq!(
        unknown(r#"        println!("Undo with `shall undo {}`.", id);"#),
        ["undo"],
        "what `purge-undeclared` told the user to run"
    );
    assert_eq!(
        unknown(r#"    /// Upgrade only packages that `shall audit` reports as vulnerable"#),
        ["audit"],
        "a dead command inside the very file `help_map_tests.rs` gates"
    );
    assert_eq!(
        unknown(r#"                 shall status        see every destination first\n  \"#),
        ["status"],
        "a line of a multi-line refusal message, which no closing delimiter precedes"
    );
    assert_eq!(
        unknown("shall undo               # interactive snapshot gallery"),
        ["undo"],
        "a line in one of README.md's fenced blocks"
    );

    // Depth: a wrong word in a nested path is the same defect one level down.
    assert_eq!(unknown("`shall git stauts`"), ["stauts"]);
    assert_eq!(unknown("`shall snapshot delete`"), ["delete"]);

    // And the controls, without which every assertion above would hold for a scan that
    // reports everything.
    assert!(unknown("`shall check drift`").is_empty(), "a live section");
    assert!(unknown("`shall git status`").is_empty(), "a live sub-verb");
    assert!(unknown("`shall tui`").is_empty(), "an alias clap declares");
    assert!(unknown("`shall help sync`").is_empty(), "clap's own verb");
    assert!(
        unknown("`shall adopt service`").is_empty(),
        "a positional argument that happens to share a verb's name"
    );
    assert!(
        unknown("`shall lock scripts` approves every hook").is_empty(),
        "a value enum, followed by prose inside the same backticks"
    );
    assert!(
        unknown("`shall upgrade --profile dev`").is_empty(),
        "a flag's value"
    );
    assert!(
        unknown("`shall --dry-run sync`").is_empty(),
        "a global flag before the subcommand"
    );
    assert!(
        unknown("`shall <command> --allow-mass-removal`").is_empty(),
        "a placeholder, which cannot be resolved and must not be guessed at"
    );
    assert!(
        unknown("cd $(shall path)").is_empty(),
        "a command substitution"
    );

    // Prose, which is the whole reason the product is spelled `Shall` in a sentence.
    assert!(unknown("Every hook actually invokes the shall binary.").is_empty());
    assert!(unknown("on startup shall reads the name it was called by").is_empty());
    assert!(unknown("this shall speaks schema 2").is_empty());
    assert!(unknown("Rebuilding shall from {repo} via cargo").is_empty());
    assert!(
        unknown("the shall-plan.json written by an earlier run").is_empty(),
        "`shall` inside a longer word"
    );
    assert!(
        unknown(r#"A "shall" shim would overwrite shall itself with itself"#).is_empty(),
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
        found.iter().any(|i| i.file == "README.md"),
        "README.md is the only user-facing document in the repo"
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
        "nested subcommands must be read, or `shall git status` would look like `shall status`"
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
