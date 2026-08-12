//! **A `pub fn` nothing calls is invisible to every gate this repository owns.**
//!
//! `cargo` never warns about an unused `pub fn` in a library — it is public, so something
//! outside the crate might call it. Nothing outside this crate does: `shall` is an application
//! whose library half exists so the tests can reach it. So `dead_code` is off for exactly the
//! class of dead code that costs something here, and the build, clippy, fmt and the whole test
//! suite all pass over it.
//!
//! **What it cost.** `MetricsCollector::record_error` was the only writer of `Metrics.errors`
//! and had zero callers anywhere in the tree. `errors.is_empty()` was therefore permanently
//! true, which made the transaction summary's `Status:` line the constant `SUCCESS`, made
//! `DEGRADED` unreachable, and made `print_summary_quiet` — whose entire body printed the
//! unreachable error block — print nothing under any circumstances. A sync in which every
//! package failed reported success, and the same run under `--quiet` printed zero bytes. One
//! uncalled function, three broken outputs, and 517 passing tests (B1).
//!
//! Six more were found by the same scan and deleted with it: `App::new_with_executor`,
//! `Sandbox::is_supported`, `ShimManager::get_bin_dir`, `FormatOrder::rejected_by`,
//! `File::module_blocks`, `Phase::is_work`.
//!
//! **The bar here is zero references, not zero production callers.** A `pub fn` reached only
//! from tests is usually a test double — the fake executor's API alone accounts for a hundred
//! references — and policing those would make this a gate people delete. A function nothing
//! mentions at all is unambiguous.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Every `.rs` file under a directory.
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

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// The identifier a `pub fn` / `pub async fn` declaration names, if the line is one.
///
/// Textual, and that is sound for the question being asked: a trait method cannot carry `pub`
/// in its impl, so every hit is an inherent or free function — the two kinds that can go
/// uncalled without the compiler minding.
fn declared_name(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let rest = t
        .strip_prefix("pub fn ")
        .or_else(|| t.strip_prefix("pub async fn "))
        .or_else(|| t.strip_prefix("pub(crate) fn "))
        .or_else(|| t.strip_prefix("pub(crate) async fn "))?;
    let name = rest
        .split(|c: char| c == '(' || c == '<' || c.is_whitespace())
        .next()?;
    (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some(name)
}

/// Whole-identifier occurrences of `name` in `text`.
///
/// Whole tokens, never `contains`: `run` appears inside `run_test`, `dry_run` and `rerun`, and
/// a substring count would report every function in the tree as called.
fn mentions(text: &str, name: &str) -> usize {
    let bytes = text.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut count = 0;
    let mut from = 0;
    while let Some(at) = text[from..].find(name) {
        let start = from + at;
        let end = start + name.len();
        let before_ok = start == 0 || !ident(bytes[start - 1]);
        let after_ok = end == bytes.len() || !ident(bytes[end]);
        if before_ok && after_ok {
            count += 1;
        }
        from = start + name.len();
    }
    count
}

#[test]
fn no_public_function_is_referenced_by_nothing_at_all() {
    let mut files = Vec::new();
    rust_files(&repo("src"), &mut files);
    let src_count = files.len();
    rust_files(&repo("tests"), &mut files);
    assert!(
        src_count > 50 && files.len() > src_count,
        "the scan found {src_count} source file(s) and {} in total — it is not reading the \
         tree, so every assertion below is vacuous",
        files.len()
    );

    let bodies: Vec<(PathBuf, String)> = files
        .into_iter()
        .filter_map(|p| std::fs::read_to_string(&p).ok().map(|b| (p, b)))
        .collect();

    // name -> where it was declared. A name declared twice (an inherent `new` on two types) is
    // recorded once and its references are counted across both, which is the safe direction:
    // this gate reports a function nothing mentions, and a shared name is mentioned.
    let mut declared: BTreeMap<String, String> = BTreeMap::new();
    for (path, body) in &bodies {
        if !path.starts_with(repo("src")) {
            continue;
        }
        for (i, line) in body.lines().enumerate() {
            if let Some(name) = declared_name(line) {
                declared
                    .entry(name.to_string())
                    .or_insert_with(|| format!("{}:{}", path.display(), i + 1));
            }
        }
    }
    assert!(
        declared.len() > 300,
        "only {} public function(s) found; the declaration scan has stopped matching the tree",
        declared.len()
    );

    let mut orphans: Vec<String> = Vec::new();
    for (name, at) in &declared {
        // `main` is called by the runtime, and a `new` shared by dozens of types is never
        // going to be the finding this gate is for.
        if name == "main" {
            continue;
        }
        let total: usize = bodies.iter().map(|(_, b)| mentions(b, name)).sum();
        // One mention is the declaration itself. Anything more is a caller, a re-export, a
        // test, or a doc link — all of which mean somebody knows it is there.
        if total <= 1 {
            orphans.push(format!("{name}  ({at})"));
        }
    }

    assert!(
        orphans.is_empty(),
        "these public functions are referenced by nothing in the tree — not by production \
         code, not by a test, not by a doc comment. `cargo` cannot see this class, which is \
         how a `record_error` with no callers made `DEGRADED` unreachable and silenced \
         `--quiet` entirely (B1). Call each one, or delete it:\n  {}",
        orphans.join("\n  ")
    );
}
