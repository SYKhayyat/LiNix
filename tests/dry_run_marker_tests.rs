//! The `[DRY-RUN]` marker has one definition.
//!
//! It had sixty-eight, one per call site, and by the time anyone counted them they had drifted
//! four ways:
//!
//! - `Would` beside `would` — nine sites capitalised, the rest not.
//! - `Go: [DRY-RUN] would delete …` in `backends/go.rs`, with the marker in *second* place, so a
//!   `^\[DRY-RUN\]` grep — the obvious way to ask "what would this run do?" — did not see it.
//! - `debug!` at two sites (`core/snapshot.rs` retention pruning, `core/git.rs` committing),
//!   which is **below the default log level**. Those two announced themselves to nobody: a
//!   preview that silently omits work it would do fails at the one thing a preview is for.
//! - `warn!` at two more (`utils/file.rs`), which reads as *something is wrong* when the only
//!   thing that happened is that a preview previewed.
//!
//! None of that is a spelling problem. Each one is a user asking "what will you do" and getting
//! an answer that is incomplete, or that they cannot find.
//!
//! So `core::dry_run` owns the string and three macros deliver it — `would!` to the log at
//! `info`, `would_warn!` for a line that is *also* drift the run found, `would_print!` to stdout
//! for the verbs whose printed output is itself the answer. This test is what keeps that true:
//! **the literal appears in `core/dry_run.rs` and nowhere else in `src/`.**
//!
//! A comment may mention it — a comment is prose about the marker, not an emission of it.

use std::path::{Path, PathBuf};

/// Where the marker is allowed to be spelled out.
const HOME: &str = "core/dry_run.rs";

const MARKER: &str = "[DRY-RUN]";

fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
}

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn the_dry_run_marker_is_written_in_exactly_one_place() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(
        files.len() > 100,
        "the scan found {} files under src/, which means it is not scanning",
        files.len()
    );

    let mut strays = Vec::new();
    let mut home_hits = 0usize;

    for file in &files {
        let text = std::fs::read_to_string(file).expect("read a source file");
        let is_home = slash(file).ends_with(HOME);
        for (n, line) in text.lines().enumerate() {
            if !line.contains(MARKER) {
                continue;
            }
            if is_home {
                home_hits += 1;
                continue;
            }
            // Prose about the marker is not an emission of it.
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("*") {
                continue;
            }
            strays.push(format!(
                "{}:{}  {}",
                slash(file.strip_prefix(root.parent().unwrap()).unwrap_or(file)),
                n + 1,
                line.trim()
            ));
        }
    }

    assert!(
        home_hits > 0,
        "no source line in {HOME} spells `{MARKER}` — either the marker moved or this test is \
         now asserting nothing"
    );
    assert!(
        strays.is_empty(),
        "the `{MARKER}` marker is spelled out {} time(s) outside {}:\n  {}\n\nUse the macros \
         instead — `would!(\"would install {{}}\", name)` for the log, `would_print!(…)` for a \
         verb's own output, `would_warn!(…)` when the line is also drift the run found. A \
         literal at the call site is free to drift, and the last time nobody checked it drifted \
         four ways, twice invisibly.",
        strays.len(),
        HOME,
        strays.join("\n  ")
    );
}

/// The other half, and the half that would rot silently: the macros must be reachable and must
/// emit the marker. A gate that only forbids the literal is satisfied by deleting every dry-run
/// line in the program.
#[test]
fn the_macros_that_replaced_the_literal_actually_carry_it() {
    let home = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(HOME);
    let text = std::fs::read_to_string(&home).expect("read core/dry_run.rs");
    for macro_name in ["would", "would_warn", "would_print"] {
        assert!(
            text.contains(&format!("macro_rules! {macro_name}")),
            "`{macro_name}!` is gone from {HOME}, so every call site that used it is either \
             broken or has gone back to a literal"
        );
    }
    assert_eq!(
        shall::core::dry_run::MARKER,
        MARKER,
        "the marker changed; this test and the fixtures that grep for it need to change with it"
    );
}
