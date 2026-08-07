//! The map in `--help` names every command, and only commands that exist (AU11).
//!
//! `--help` listed 61 subcommands as one flat wall; the map groups them by what a person is
//! trying to do. A hand-written grouping is a second list of names beside the enum, and this
//! repo has already paid for one of those: `undo` sat in two harness exemption lists for months
//! after the command was renamed, because nothing compared the list to the program.
//!
//! So the map is compared to `--help` in both directions. Neither half is optional — a map that
//! may omit commands drifts into naming a third of them, and a map that may invent commands is
//! documentation for a program nobody shipped.

use std::collections::BTreeSet;
use std::process::Command;

fn help() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_linix"))
        .arg("--help")
        .output()
        .expect("the binary should run");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The names clap prints in its own `Commands:` block — the program's own answer, never a
/// list maintained beside it.
fn declared(help: &str) -> BTreeSet<String> {
    help.lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .take_while(|l| !l.starts_with("Options:"))
        .filter_map(|l| {
            let trimmed = l.strip_prefix("  ")?;
            if trimmed.starts_with(' ') {
                return None; // a wrapped description line
            }
            let name = trimmed.split_whitespace().next()?;
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c == '-')
                .then(|| name.to_string())
        })
        .collect()
}

/// The names inside the map, which begins after the `Options:` block.
fn mapped(help: &str) -> BTreeSet<String> {
    help.lines()
        .skip_while(|l| !l.starts_with("The map ("))
        .flat_map(|l| l.split(&[' ', '\t'][..]))
        .map(|t| t.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-'))
        // `--help` in the closing sentence is a flag, not a verb.
        .filter(|t| !t.starts_with('-'))
        .filter(|t| {
            !t.is_empty()
                && t.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-' || c == '_')
        })
        .map(|t| t.to_string())
        .collect()
}

#[test]
fn every_command_appears_in_the_map() {
    let help = help();
    let declared = declared(&help);
    assert!(
        declared.len() > 50,
        "only {} commands were parsed out of `--help`; the parser above is broken, not the map",
        declared.len()
    );

    let mapped = mapped(&help);
    let missing: Vec<&String> = declared
        .iter()
        // `help` is clap's own and is not a thing anyone looks up in a map of what to do.
        .filter(|n| n.as_str() != "help" && !mapped.contains(*n))
        .collect();

    assert!(
        missing.is_empty(),
        "these commands exist and the map in `--help` does not name them: {:?}",
        missing
    );
}

#[test]
fn the_map_names_no_command_that_does_not_exist() {
    let help = help();
    let declared = declared(&help);

    // Prose words in the map are not commands; only tokens that look like a verb are checked,
    // and a real verb is by definition one clap declared. Anything verb-shaped and undeclared
    // is the `undo` case.
    let invented: Vec<String> = mapped(&help)
        .into_iter()
        .filter(|t| t.contains('-') || declared.contains(t) || t.len() > 3)
        .filter(|t| !declared.contains(t))
        // Words that appear in the map's own sentences rather than in its lists.
        //
        // **`undo` is in this list, and it is the deleted command this file's header cites as
        // the reason the file exists.** That is not a mistake and it is not safe on its own: the
        // map's prose carries the section heading *"Undo and time travel"*, so the word appears
        // here as English rather than as a verb — and **nothing in this list can tell those two
        // apart.** A gate drawn around one copy of a fact, with the fact escaping into the gate,
        // is precisely the shape the header is about.
        //
        // What makes it safe is a second gate, not this one:
        // `named_commands_exist_tests.rs` resolves every `linix <word>` in `src/`, `tests/`,
        // `scripts/`, `docker/`, `examples/`, `.github/` and `readme.md` against clap's live
        // command tree, so a *live invocation* of `undo` fails there regardless of what this
        // list says. Since 2026-08-07 it also covers `docs/`, against `target-state.md` II.17's
        // register of what was deleted — and `undo` is in that register.
        //
        // So: this list may exempt an English word. It cannot exempt a command, because it is
        // not the thing that decides whether a command is real.
        .filter(|t| {
            ![
                "map",
                "every",
                "command",
                "above",
                "what",
                "you",
                "are",
                "doing",
                "make",
                "the",
                "machine",
                "match",
                "your",
                "files",
                "change",
                "declare",
                "look",
                "things",
                "undo",
                "and",
                "time",
                "travel",
                "profiles",
                "modules",
                "cleaning",
                "fleet",
                "itself",
                "start",
                "with",
                "then",
                "explains",
                "any",
                "one",
                "them",
                "linix",
                "linix-plan",
                "modules/starter.txt",
            ]
            .contains(&t.as_str())
        })
        .collect();

    assert!(
        invented.is_empty(),
        "the map names commands the program does not have: {:?}",
        invented
    );
}
