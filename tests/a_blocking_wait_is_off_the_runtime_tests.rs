//! **The second half of II.52: a blocking wait does not sit on a runtime worker.**
//!
//! II.52 has two halves and only one of them had a gate.
//! `a_spawned_child_has_an_owner_tests` fails on a `Command` that reaches
//! `spawn`/`output`/`status` outside the executor — that is the *process* half, and it is
//! complete. The *blocking-wait* half had nothing at all: nothing in the tree enumerated an
//! `fsync` under a lock, a synchronous `walkdir` inside an `async fn`, or a synchronous
//! interpreter eval on a shared task, and `core::blocking::off_the_runtime` — the door II.52
//! names for "work that can move" — had exactly **one** caller in the whole codebase.
//!
//! Eight sites were found by hand on 2026-08-18, in five files:
//!
//! - eleven call sites wrote the registry while holding the global state mutex, so every one of
//!   the sixty other `state.lock()` sites waited out a physical disk flush;
//! - the WAL flushed once per package, un-batched, under the journal mutex, on the path that
//!   opens every wave — ~298 flushes before a single manager was invoked on a 298-package
//!   config;
//! - `github:` and `web:` flushed inside the install wave, under their own lock;
//! - `model::cache` ran a `walkdir` over `~/.cache` and `/var/cache` on a runtime worker, once
//!   per artifact;
//! - `#rhai` hooks evaluated synchronously from an `async fn`, with Rhai's own `http_get`
//!   blocking on a channel recv for a whole HTTP round trip.
//!
//! Every one of those is a *correct call to a correct door made in the wrong scope*, which is
//! why `a_writer_that_reaches_the_disk_goes_through_one` passes all of them: it asks whether
//! the write goes through `durable_write`, and says nothing about what the caller is holding
//! while it happens. V.182 makes the argument for closing this better than this comment can —
//! *"a list of sites fixed is a fact about one afternoon; a predicate that fails the build is a
//! fact about every afternoon after it."*
//!
//! **Why it matters more here than in most codebases.** This tree deliberately multiplexes its
//! hottest fan-outs onto a *single task* (`planner.rs`: *"the futures borrow `&self` so this
//! stays on one task (no spawn)"*). A blocking call reached from inside a wave therefore does
//! not cost one task's latency; it costs the whole wave's.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Blocking calls that must not be reached from an `async fn` without a door.
///
/// Two classes, both named by II.52. A **durable write** ends in `fsync`, which parks a thread
/// on the disk for as long as the disk takes. A **filesystem walk** is thousands of `stat`
/// calls with nothing to await between them. Neither is slow because of anything Shall does,
/// which is exactly why neither belongs on a worker that has other futures to drive.
const BLOCKING_CALLS: &[&str] = &[
    // Durable writes. `persist` and `durable_write` both end in `sync_all`; `append_line` and
    // `append_lines` end in `sync_data`. Named individually rather than as "fsync", because the
    // whole point of S59's fix is that callers reach the disk through those names.
    "file::persist(",
    "durable_write(",
    "append_line(",
    "append_lines(",
    ".sync_all()",
    ".sync_data()",
    // Filesystem walks.
    "WalkDir::new(",
    "std::fs::read_dir(",
];

/// The doors that make a blocking call legitimate.
///
/// `off_the_runtime` is the one II.52 names. `spawn_blocking` is what it is built on, and
/// several paths reach it directly — correct, if less legible. `block_in_place` is the third
/// and the weakest: it moves the runtime's *other tasks* off this worker and does nothing for
/// futures sharing this task, so it is right for a prompt and wrong for a wave.
const DOORS: &[&str] = &["off_the_runtime(", "spawn_blocking(", "block_in_place("];

/// Files whose blocking calls sit inside an `async fn` and are right there, with the sentence.
///
/// **Empty, and that is the point.** The eight sites this gate was written for are all fixed,
/// and every one-shot write at the end of a command went through `persist_off_the_runtime`
/// rather than into this table — because a rule with five exceptions is a rule everybody has to
/// remember, and the fix costs nothing at a site that was not hot to begin with. A short reason
/// is not a reason: an entry here is checked for length below, because "it's fine" is what all
/// eight would have said.
const ON_THE_RUNTIME_BY_DESIGN: &[(&str, &str)] = &[];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

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

/// The file's own code, without its `#[cfg(test)]` tail.
///
/// A fixture that flushes on a worker flushes for the length of one test, which is a different
/// problem from a sync that stalls its whole wave.
fn body_of(source: &str) -> &str {
    match source.find("\n#[cfg(test)]") {
        Some(i) => &source[..i],
        None => source,
    }
}

/// The function a line belongs to: the nearest `fn` declaration above it.
///
/// Crude on purpose. A full parse would be more accurate and would also be a second Rust
/// front-end living in the test suite; what this has to distinguish is `async fn` from `fn`,
/// and the declaration line says which.
fn enclosing_fn<'a>(lines: &[&'a str], at: usize) -> Option<&'a str> {
    lines[..=at]
        .iter()
        .rev()
        .find(|l| {
            let t = l.trim_start();
            (t.starts_with("fn ")
                || t.starts_with("async fn ")
                || t.starts_with("pub fn ")
                || t.starts_with("pub async fn ")
                || t.starts_with("pub(crate) fn ")
                || t.starts_with("pub(crate) async fn ")
                || t.starts_with("pub(super) fn ")
                || t.starts_with("pub(super) async fn "))
                && l.contains('(')
        })
        .copied()
}

/// Is this line a blocking call that is not already inside a door on the same line?
///
/// `off_the_runtime(move || crate::utils::file::persist(..))` is one line and is correct, so
/// the same-line check is not a shortcut — it is the common shape of a fixed site.
fn blocking_call(line: &str) -> Option<&'static str> {
    let t = line.trim_start();
    if t.starts_with("//") || t.starts_with("///") || t.starts_with("//!") {
        return None;
    }
    if DOORS.iter().any(|d| line.contains(d)) {
        return None;
    }
    BLOCKING_CALLS.iter().copied().find(|c| line.contains(c))
}

/// Whether the door was opened somewhere between the function's declaration and this line.
///
/// The multi-line shape, where the closure spans lines:
///
/// ```ignore
/// crate::core::off_the_runtime(move || {
///     crate::utils::file::persist(&path, &data)
/// })
/// ```
fn behind_a_door(lines: &[&str], from: usize, at: usize) -> bool {
    lines[from..=at]
        .iter()
        .any(|l| DOORS.iter().any(|d| l.contains(d)))
}

#[test]
fn a_durable_write_or_a_filesystem_walk_leaves_the_runtime() {
    let root = repo_root().join("src");
    let mut files = Vec::new();
    rust_sources(&root, &mut files);

    let mut scanned = 0usize;
    let mut on_the_runtime: BTreeSet<String> = BTreeSet::new();
    let mut detail_of: Vec<(String, String)> = Vec::new();

    for path in &files {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        scanned += 1;
        let relative = path
            .strip_prefix(repo_root())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        // `utils::file` and `core::blocking` are the mechanism: the flush and the door are
        // defined there, and a definition is not a call site.
        if relative == "src/utils/file.rs" || relative == "src/core/blocking.rs" {
            continue;
        }

        let body = body_of(&source);
        let lines: Vec<&str> = body.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let Some(call) = blocking_call(line) else {
                continue;
            };
            let Some(decl) = enclosing_fn(&lines, i) else {
                continue;
            };
            if !decl.contains("async fn") {
                continue;
            }
            // Where the function starts, so the door can be looked for in between.
            let start = lines[..=i]
                .iter()
                .rposition(|l| std::ptr::eq(*l, decl))
                .unwrap_or(0);
            if behind_a_door(&lines, start, i) {
                continue;
            }
            on_the_runtime.insert(relative.clone());
            detail_of.push((
                relative.clone(),
                format!("{}  ({} in `{}`)", line.trim(), call, decl.trim()),
            ));
        }
    }

    // **Not `Ledger::audit`, and the reason is the same one that makes this gate different
    // from its five siblings.** Every one of those asserts the walk matched *something*, on the
    // sound argument that a predicate which has stopped recognising its subject passes by
    // finding nothing. That control cannot work here: the answer this gate wants is zero, and a
    // clean tree is indistinguishable from a broken predicate by that measure. The non-vacuity
    // proof is `the_predicate_catches_what_it_is_for_and_nothing_else` below, which runs this
    // same predicate over a planted file holding one of each shape and requires all three to be
    // caught — a control the walk cannot fake by reading the wrong directory.
    //
    // The floor stays, because "read the wrong directory" is still a way to be silent: well
    // under the files in `src/`, and far above the handful a broken walk reads.
    const FLOOR: usize = 120;
    assert!(
        scanned >= FLOOR,
        "the walk read only {scanned} file(s), under its floor of {FLOOR}; it is looking in the \
         wrong place, so neither its findings nor its silence mean anything"
    );

    let excused: BTreeSet<&str> = ON_THE_RUNTIME_BY_DESIGN.iter().map(|(s, _)| *s).collect();
    for (site, reason) in ON_THE_RUNTIME_BY_DESIGN {
        assert!(
            reason.len() >= 80,
            "{site}'s exemption is {} characters. That is an assertion, not a reason — say what \
             about this site makes a blocking wait on a runtime worker correct.",
            reason.len()
        );
        assert!(
            on_the_runtime.contains(*site),
            "ON_THE_RUNTIME_BY_DESIGN excuses {site}, which the walk no longer finds to block. \
             Delete the entry: a permission granted to nothing still reads as one guarding \
             something."
        );
    }

    let unexplained: Vec<&String> = on_the_runtime
        .iter()
        .filter(|s| !excused.contains(s.as_str()))
        .collect();
    if !unexplained.is_empty() {
        let mut msg = String::from(
            "these sites reach a durable write or a filesystem walk from an `async fn`, on a \
             runtime worker, and are not in ON_THE_RUNTIME_BY_DESIGN:",
        );
        for site in &unexplained {
            msg.push_str(&format!("\n    {site}"));
            for (s, line) in &detail_of {
                if s == *site {
                    msg.push_str(&format!("\n        {line}"));
                }
            }
        }
        msg.push_str(
            "\n\nMove it through `core::blocking::off_the_runtime` — the door II.52 names for \
             work that can move — or `tokio::task::spawn_blocking` directly. \
             `utils::file::persist_off_the_runtime` is the ready-made one for a durable write. \
             Do NOT reach for `block_in_place`: it moves the runtime's other *tasks* off this \
             worker and does nothing for futures sharing this task, and this tree deliberately \
             multiplexes its hottest fan-outs onto one task. If the work genuinely cannot move, \
             take the bytes under the lock and write them after it — `StateRegistry::snapshot` \
             plus `StateSnapshot::write_off_the_runtime` is that shape.\n\
             Or add it to ON_THE_RUNTIME_BY_DESIGN with the sentence explaining why it must be \
             that way.",
        );
        panic!("{msg}");
    }
}

/// **And the door is still there.** The gate above passes trivially if the names it looks for
/// stop existing: every site would read as "does not block", the walk would find nothing, and a
/// vacuous pass looks exactly like a clean one.
#[test]
fn the_door_exists_and_the_flush_still_goes_through_one_name() {
    let blocking = std::fs::read_to_string(repo_root().join("src/core/blocking.rs"))
        .expect("core::blocking is where II.52 is decided");
    assert!(
        blocking.contains("pub async fn off_the_runtime"),
        "`off_the_runtime` is gone; the gate above is vacuous"
    );

    let file = std::fs::read_to_string(repo_root().join("src/utils/file.rs"))
        .expect("utils::file is the one durable write");
    for flush in [".sync_all()", ".sync_data()"] {
        assert!(
            file.contains(flush),
            "{flush} no longer happens in utils::file, so the names this gate scans for are \
             no longer where the disk is reached — re-derive BLOCKING_CALLS before deleting \
             this assertion"
        );
    }

    // The one that made eleven sites wrong. If `snapshot` stops existing, the remedy this gate
    // prints names a function nobody can call.
    let state = std::fs::read_to_string(repo_root().join("src/core/state.rs"))
        .expect("core::state holds the registry");
    assert!(
        state.contains("pub fn snapshot") && state.contains("pub async fn write_off_the_runtime"),
        "the serialise-under-the-lock / write-after-it pair is gone; B2's family has no fix to \
         point at"
    );
}

/// **The counter-example, so the predicate is known to fire.**
///
/// A gate that has never failed is a gate nobody has tested. This runs the same predicate over
/// a synthetic file holding one of each shape and requires it to catch every one — and over
/// the fixed forms of the same shapes and requires it to catch none.
#[test]
fn the_predicate_catches_what_it_is_for_and_nothing_else() {
    let caught = |src: &str| -> usize {
        let lines: Vec<&str> = src.lines().collect();
        let mut n = 0;
        for (i, line) in lines.iter().enumerate() {
            if blocking_call(line).is_none() {
                continue;
            }
            let Some(decl) = enclosing_fn(&lines, i) else {
                continue;
            };
            if !decl.contains("async fn") {
                continue;
            }
            let start = lines[..=i]
                .iter()
                .rposition(|l| std::ptr::eq(*l, decl))
                .unwrap_or(0);
            if !behind_a_door(&lines, start, i) {
                n += 1;
            }
        }
        n
    };

    let broken = r#"
async fn writes_on_the_runtime(&self) -> Result<()> {
    crate::utils::file::persist(&self.path, &data)?;
    Ok(())
}

async fn crawls_on_the_runtime(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root).max_depth(4).into_iter().collect()
}

async fn reads_a_dir_on_the_runtime(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|e| e.count()).unwrap_or(0)
}
"#;
    assert_eq!(
        caught(broken),
        3,
        "the predicate missed one of the three shapes it exists for"
    );

    let fixed = r#"
async fn writes_off_the_runtime(&self) -> Result<()> {
    let path = self.path.clone();
    crate::core::off_the_runtime(move || crate::utils::file::persist(&path, &data)).await?
}

async fn crawls_off_the_runtime(root: PathBuf) -> Vec<PathBuf> {
    crate::core::off_the_runtime(move || {
        WalkDir::new(&root).max_depth(4).into_iter().collect()
    })
    .await
    .unwrap_or_default()
}

fn writes_synchronously(path: &Path) -> Result<()> {
    crate::utils::file::persist(path, "not async, not this gate's business")
}
"#;
    assert_eq!(
        caught(fixed),
        0,
        "the predicate fired on a site that is already correct, which is how a gate gets \
         switched off"
    );
}
