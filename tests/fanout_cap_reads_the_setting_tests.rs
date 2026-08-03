//! Every concurrency cap in the tree reads a config field (AU9).
//!
//! `planner.rs` states the rule in its own words — *"`max_parallel`, and a cap that ignores the
//! setting is a cap the user cannot move"* — and one fan-out ignored it: `guard.rs` hard-coded
//! `.buffer_unordered(8)` for the per-backend OS-essential query, which is on **every removal
//! path**, which is exactly where somebody who has turned the parallelism down most wants it
//! honoured.
//!
//! A rule written in a comment beside one fan-out is a rule about that fan-out. This is the same
//! rule asked of all of them, which is the only form that survives the next one being added.
//!
//! **Why a source scan.** Same reason as `removal_guard_enumeration_tests.rs`: the finding is
//! about a site that exists and is not covered, and no behaviour can enumerate the sites nobody
//! tested. Adding a literal cap anywhere in `src/` fails this until it reads a setting.

use std::path::{Path, PathBuf};

/// The stream combinators that take a width. `buffered` and `buffer_unordered` are futures';
/// `Semaphore::new` is tokio's, and it is a cap by another name.
const CAPS: &[&str] = &["buffer_unordered(", "buffered(", "Semaphore::new("];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
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

#[test]
fn no_fan_out_in_the_tree_hard_codes_its_width() {
    let mut files = Vec::new();
    rust_files(Path::new("src"), &mut files);
    assert!(
        files.len() > 50,
        "only {} source files found; the walk is broken, not the tree",
        files.len()
    );

    let mut offenders: Vec<String> = Vec::new();
    let mut found_any = 0usize;

    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // Its own doc comment naming the mistake is not the mistake.
            if trimmed.starts_with("//") {
                continue;
            }
            for cap in CAPS {
                let Some(rest) = trimmed.split_once(cap).map(|(_, r)| r) else {
                    continue;
                };
                found_any += 1;
                // A width taken from a config field mentions one; a literal starts with a
                // digit. `max(1)` around either is a clamp, not a width.
                if rest.starts_with(|c: char| c.is_ascii_digit()) {
                    offenders.push(format!(
                        "{}:{}  {}",
                        file.display(),
                        n + 1,
                        trimmed.trim_end()
                    ));
                }
            }
        }
    }

    assert!(
        found_any > 3,
        "found only {} fan-outs at all; the scan matches nothing and would pass over anything",
        found_any
    );
    assert!(
        offenders.is_empty(),
        "these fan-outs hard-code their width instead of reading `max_parallel` / \
         `network_parallel` / `max_concurrent`. A cap that ignores the setting is a cap the \
         user cannot move:\n  {}",
        offenders.join("\n  ")
    );
}
