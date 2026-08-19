//! A `locks/` ledger is read and written under one lock (II.8, V.196).
//!
//! Every ledger is written whole. Two processes that each load it, each change their own copy
//! and each save it back leave one of the two changes gone — and the changes are approvals, pins
//! and resolutions, so the one that loses is a hook that has to be approved again or a name that
//! resolves to a different manager. Taking a lock around the *save* closes nothing: the copy
//! being written was read before the lock was taken.
//!
//! `LockFile::update` is the door. It holds one lock across the load, the change and the save.
//!
//! **What kept this correct until now was an accident, which is the reason for a gate rather
//! than a comment.** Every remaining `save` below is reached from a `Writer` verb, and a `Writer`
//! holds the data lock for its whole run — so the load and the save are already inside one
//! critical section. That is one reason, not eleven, and it stops being true the moment somebody
//! reaches one of these paths from a `Reader`. `shall check` writing `locks/regex.toml` under no
//! lock at all is what that looked like the last time it happened.
//!
//! So the list is closed: a new ledger write either goes through `update`, or it is added here
//! with the sentence that says why it does not have to.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` under `src/`, the same walk its five sibling gates use.
fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    rust_sources(&repo_root().join("src"), &mut out);
    out.sort();
    out
}

/// The path as the tables here spell it: relative to the repo, forward slashes.
fn named(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The ledger types this gate is about. `locks/versions.json` is not among them — it is JSON
/// rather than TOML and carries its own reader and writer — and its pins are covered by the same
/// `Writer` argument.
const LEDGERS: &[&str] = &[
    "HookLedger",
    "ExecLedger",
    "ExtrasLedger",
    "RegexLock",
    "BareLock",
    "ArtifactLedger",
];

/// Every `save` that is not `update`, and the sentence that excuses it.
///
/// Each entry is `(file, function, why)`. The `why` is not decoration: it is what a reader
/// checks when the function moves under a different verb.
const SAVED_WITHOUT_UPDATE: &[(&str, &str, &str)] = &[
    (
        "src/app/apply/execs.rs",
        "apply",
        "reached only from the apply path, which runs under `sync`/`apply` — both Writers, so \
         the ledger is loaded and saved inside one held lock. The load also decides whether a \
         script runs at all, against its recorded count, so the read cannot move.",
    ),
    (
        "src/app/apply/execs.rs",
        "undo_departed_execs",
        "the same ledger value, arriving by `&mut` from `apply` — one read-modify-write split \
         across two functions rather than a second one.",
    ),
    (
        "src/app/apply/extras.rs",
        "reconcile",
        "under `sync`/`apply`. The write is a whole-set replacement computed from the declared \
         set and from which teardowns succeeded, so it is not expressible as an insert against \
         a freshly loaded copy.",
    ),
    (
        "src/app/sync/resolver.rs",
        "expand_regexes",
        "under `sync`, and the save prunes every key the model no longer declares — a decision \
         about the whole file, taken from a resolution that spans catalogue queries.",
    ),
    (
        "src/app/sync/resolver.rs",
        "probe_bare_names",
        "under `sync`. Same shape as the regex expansion: the loaded entry decides whether a \
         name is asked at all, and the prune is whole-model.",
    ),
    (
        "src/backends/github.rs",
        "commit_state",
        "already one critical section, and a wider one than `update` could give: the backend's \
         own state file and the artifact ledger are written under a single mutex, because they \
         must agree with each other.",
    ),
    (
        "src/verbs/plan.rs",
        "lock_backends",
        "the `lock` verb, a Writer. The write is a diff of two snapshots taken either side of a \
         whole model resolution, which is what makes it a scope-limited replacement rather than \
         a delta.",
    ),
    (
        "src/verbs/plan.rs",
        "lock_scripts",
        "the `lock` verb, a Writer. It approves everything and then restores every out-of-scope \
         entry from a snapshot taken before the approvers ran, so its critical section has to \
         span theirs.",
    ),
    (
        "src/verbs/plan.rs",
        "approve_adapters",
        "the `lock` verb, a Writer. Writes only when something was approved, which `update` \
         cannot express — it always writes.",
    ),
    (
        "src/verbs/plan.rs",
        "unlock_backends",
        "the `unlock` verb, a Writer. Returns without writing when there is nothing to release, \
         which `update` cannot express.",
    ),
    (
        "src/verbs/plan.rs",
        "unlock_scripts",
        "the `unlock` verb, a Writer, and the same early return as its sibling above.",
    ),
];

/// `(file, function)` for every ledger `save` outside a test module.
fn saves_in_the_tree() -> BTreeSet<(String, String)> {
    let mut found = BTreeSet::new();
    for path in sources() {
        let text = std::fs::read_to_string(&path).expect("a source file reads");
        // The convention here is tests at the bottom of the file; everything from the first
        // `#[cfg(test)]` is fixtures building ledgers of their own, which is not this rule.
        let production = text.split("#[cfg(test)]").next().unwrap_or_default();

        let mut current = String::new();
        let mut body_since_fn = String::new();
        for line in production.lines() {
            if let Some(name) = function_name(line) {
                current = name;
                body_since_fn.clear();
            }
            body_since_fn.push_str(line);
            body_since_fn.push('\n');

            if line.contains(".save(") && !line.contains("fn save") {
                let names_a_ledger = LEDGERS.iter().any(|l| body_since_fn.contains(l));
                if names_a_ledger && !current.is_empty() {
                    found.insert((named(&path), current.clone()));
                }
            }
        }
    }
    found
}

fn function_name(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("pub(crate) ")
        .or_else(|| trimmed.strip_prefix("pub(super) "))
        .or_else(|| trimmed.strip_prefix("pub "))
        .unwrap_or(trimmed);
    let rest = rest.strip_prefix("async ").unwrap_or(rest);
    let rest = rest.strip_prefix("fn ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

#[test]
fn every_ledger_write_is_either_one_step_or_named_here() {
    let expected: BTreeSet<(String, String)> = SAVED_WITHOUT_UPDATE
        .iter()
        .map(|(file, function, _)| (file.to_string(), function.to_string()))
        .collect();
    let found = saves_in_the_tree();

    let unlisted: Vec<_> = found.difference(&expected).collect();
    assert!(
        unlisted.is_empty(),
        "these write a `locks/` ledger without holding one lock across the read and the write, \
         and are not on the list that says why that is safe: {unlisted:?}\n\n\
         Use `LockFile::update`, which holds one lock across the load, the change and the save. \
         If this write genuinely cannot — because it must not write at all in some cases, or \
         because its critical section has to be wider — add it to SAVED_WITHOUT_UPDATE with the \
         sentence that says so."
    );

    let departed: Vec<_> = expected.difference(&found).collect();
    assert!(
        departed.is_empty(),
        "these are excused in SAVED_WITHOUT_UPDATE and no longer exist. An excuse for code that \
         is gone is an excuse the next reader will believe: {departed:?}"
    );
}

/// The excuses are the whole value of the list, so an empty one is a missing one.
#[test]
fn every_excuse_says_something() {
    for (file, function, why) in SAVED_WITHOUT_UPDATE {
        assert!(
            why.len() > 40,
            "{file}::{function} is excused without a reason anybody can check"
        );
    }
}

/// Non-vacuity, and it has to be its own control: this gate passes when it finds *nothing*
/// unlisted, so "the scan matched something" is the only thing that separates a working gate
/// from one whose walk is broken.
#[test]
fn the_scan_still_finds_the_writes_it_is_about() {
    let found = saves_in_the_tree();
    assert!(
        found.len() >= SAVED_WITHOUT_UPDATE.len(),
        "the scan found {} ledger writes and the list names {} — a scan that has stopped \
         reading the tree would pass this gate silently",
        found.len(),
        SAVED_WITHOUT_UPDATE.len()
    );
    assert!(
        found.contains(&(
            "src/app/sync/resolver.rs".to_string(),
            "probe_bare_names".to_string()
        )),
        "the scan must still see a write it is known to contain"
    );
}

/// And the reader half: nobody spells the `locks/` directory by hand.
///
/// `Layout` names it once. Twelve call sites had each spelled `config_root().join("locks")`
/// themselves, which is twelve places for the directory to move in eleven.
#[test]
fn every_locks_path_comes_from_the_layout() {
    let mut offenders = Vec::new();
    for path in sources() {
        let normalised = named(&path);
        if normalised.ends_with("src/model/layout.rs") {
            continue; // where it is spelled, once
        }
        let text = std::fs::read_to_string(&path).expect("a source file reads");
        let production = text.split("#[cfg(test)]").next().unwrap_or_default();
        for (number, line) in production.lines().enumerate() {
            if line.contains(r#"join("locks")"#) {
                offenders.push(format!("{normalised}:{}", number + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these build the `locks/` path by hand instead of asking `Layout`: {offenders:?}\n\
         Use `layout.locks_dir()`, `layout.lock_file(backend)` or `layout.version_lock_file()`."
    );
}
