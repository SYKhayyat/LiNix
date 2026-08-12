//! **`priority`'s own header says "Not listed = Shall does not use it at all." This is where
//! that becomes true of every fan-out, and not only of resolution.**
//!
//! It used to be true of one thing: a declaration naming an unlisted backend was refused.
//! Detection walked PATH for all fifty-two backends' binaries before it knew what was asked, and
//! every fan-out went to whatever happened to be installed. So a machine with `priority = apt`
//! paid for, and reported on, every package manager on the box — `shall list -b apt` cost 3,156
//! failed `statx` against `shall list`'s 3,338, which is to say `--backend` and `priority`
//! together bought nothing.
//!
//! # Why this file is a scan and not a behaviour test
//!
//! Because the risk being managed is *one call site chosen wrongly*, and a behaviour test can
//! only reach the call sites somebody thought to drive. `BackendRegistry::available()` was
//! deleted rather than fixed for the same reason: renaming it out of existence made the compiler
//! visit all twenty sites, and none of them could compile without choosing between the two
//! questions. This file is the other half — the choices themselves, written down, so that
//! "which verbs see the whole machine" is a list somebody signed off rather than a property of
//! wherever `available()` happened to be called.
//!
//! **The exceptions are the whole content.** There are two, and each one had to earn it.

use std::path::{Path, PathBuf};

/// Every file allowed to ask about the machine rather than about Shall, with the argument.
///
/// A file added here is a file that will keep reporting on, and probing, package managers the
/// user told Shall not to use. That is occasionally right and never the default.
const ASKS_ABOUT_THE_MACHINE: &[(&str, &str, &str)] = &[
    (
        "src/verbs/setup.rs",
        "present_on_this_machine",
        "`init` writes the priority file FROM what it detects. Gating detection on `priority` \
         here would read a file that does not exist yet, or gate the answer on the very list it \
         is about to produce — either way an empty priority file and a repo that can do nothing.",
    ),
    (
        "src/verbs/check.rs",
        "registered",
        "`check health` reports on managers that are ABSENT, which the usable set cannot contain \
         by definition. And an absent manager that `priority` names is not absent, it is broken \
         — the one place the whole registry and the priority list are both needed at once.",
    ),
];

/// Files under `src/` that legitimately define these methods rather than call them.
const DEFINES_THEM: &[&str] = &["src/app/backends.rs", "src/backends/registry/mod.rs"];

fn rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn relative(p: &Path) -> String {
    p.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn only_the_verbs_that_ask_about_the_machine_see_past_priority() {
    let mut files = Vec::new();
    rust_files(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(
        files.len() > 50,
        "the scan read {} source file(s); it is not walking the tree",
        files.len()
    );

    let mut found: Vec<String> = Vec::new();
    let mut gated = 0usize;
    for path in &files {
        let rel = relative(path);
        if DEFINES_THEM.contains(&rel.as_str()) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(path) else {
            continue;
        };
        gated += body.matches(".usable()").count() + body.matches(".usable_named(").count();
        for method in ["present_on_this_machine()", "registered()"] {
            if body.contains(method) {
                found.push(format!("{rel} :: {}", method.trim_end_matches("()")));
            }
        }
    }

    // The instrument: if nothing is gated, the scan is matching nothing and the assertion below
    // would pass over an ungated tree.
    assert!(
        gated >= 10,
        "only {gated} gated fan-out(s) found — this scan has stopped matching the tree, so the \
         exception list below proves nothing"
    );

    let allowed: Vec<String> = ASKS_ABOUT_THE_MACHINE
        .iter()
        .map(|(f, m, _)| format!("{f} :: {m}"))
        .collect();
    found.sort();
    let mut allowed_sorted = allowed.clone();
    allowed_sorted.sort();

    assert_eq!(
        found,
        allowed_sorted,
        "\nthe set of places that see past `priority` changed.\n\
         Adding one means that verb will probe and report on package managers the user told \
         Shall not to use — which is occasionally right and never the default. If it is right, \
         add it to ASKS_ABOUT_THE_MACHINE with the argument; if it is not, ask \
         `app.backends().await.usable()` instead.\n\
         The two that earned it:\n{}",
        ASKS_ABOUT_THE_MACHINE
            .iter()
            .map(|(f, m, why)| format!("  {f} :: {m}\n      {why}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The gate is only worth having while the method it replaced is gone.
///
/// `available()` answered "what is on this machine" and was used for "what may Shall use", and
/// the two are not the same question. Its absence is what forced the audit; a helpful soul
/// re-adding it would let the next call site skip the choice.
/// Scoped to the two files that own the backend question. `ModuleLoader::available()` and
/// `ProfileLoader::available()` are about modules and profiles, answer a question with no
/// `priority` in it, and a scan that flagged them would be a scan somebody switches off.
#[test]
fn the_method_that_answered_both_questions_at_once_stays_deleted() {
    for rel in DEFINES_THEM {
        let body = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel))
            .unwrap_or_else(|e| panic!("{rel} is readable: {e}"));
        assert!(
            !body.contains("pub fn available("),
            "{rel} defines `available()` again. It is deleted on purpose: it answered \"what is \
             on this machine\" and every caller used it for \"what may Shall use\", which is \
             what made `priority` decorative everywhere except resolution. Name the question \
             instead — `present_on_this_machine`, `registered`, or `Backends::usable`."
        );
        // …and the questions it was split into are still there to be chosen between.
        assert!(
            body.contains("present_on_this_machine") || body.contains("pub fn usable"),
            "{rel} no longer offers the question it was split into"
        );
    }
}
