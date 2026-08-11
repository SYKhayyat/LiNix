//! Does a coloured manager get parsed, or does its colour end up welded to the package names?
//!
//! Until 2026-08-04 the answer was "it depends which backend". All sixteen table-driven
//! backends sanitized; the parsers hand-rolled inside `src/backends/` mostly did not — `brew`,
//! `nix`, `cargo`, `go`, `yarn` and `storage` parsed raw, with `flatpak` and `snap` the two
//! that remembered. Thirty backends, one rule, no single place to state it.
//!
//! **Nothing had gone wrong, and that is the point.** These managers do not colour a pipe when
//! stdout is not a terminal, so the inconsistency cost nothing on the day. What it cost was the
//! next backend: a rule enforced by remembering is a rule with a failure rate, and the failure
//! is silent — a name with escape bytes in it matches nothing the installed listing reports,
//! which is permanent phantom drift rather than a crash.
//!
//! So sanitizing moved to `CommandExecutor::run_output` / `search_output`, where every backend's
//! stdout becomes a `String`. This file asserts the boundary holds, and scans for the ways
//! around it.

use dashmap::DashMap;
use shall::core::executor::MockExecutor;
use shall::core::CommandExecutor;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::ledger::Ledger;

fn coloured() -> String {
    // A bold name, a reset, a CRLF line ending, and a trailing blank line: what a manager
    // writes when it believes it is talking to a terminal.
    "\u{1b}[1mripgrep\u{1b}[0m 14.1.0\r\n\u{1b}[1mfd\u{1b}[0m 10.2.0\r\n\r\n".to_string()
}

fn executor_returning(cmd: &str, body: &str) -> CommandExecutor {
    let vfs = Arc::new(DashMap::new());
    let mock = Arc::new(MockExecutor::new(vfs.clone()));
    mock.set_response(
        cmd,
        Ok(std::process::Output {
            status: Default::default(),
            stdout: body.as_bytes().to_vec(),
            stderr: Vec::new(),
        }),
    );
    CommandExecutor::with_layer(true, false, mock, vfs, Arc::new(DashMap::new()))
}

#[tokio::test]
async fn run_output_hands_the_parser_clean_text() {
    let exec = executor_returning("anything list", &coloured());
    let out = exec.run_output("anything", &["list"], false).await.unwrap();
    assert!(
        !out.contains('\u{1b}'),
        "run_output passed an escape byte through to the parser: {out:?}"
    );
    assert!(
        !out.contains('\r'),
        "run_output passed a CRLF through to the parser: {out:?}"
    );
    let names: Vec<&str> = out
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(names, ["ripgrep", "fd"]);
}

#[tokio::test]
async fn search_output_hands_the_parser_clean_text() {
    let exec = executor_returning("anything search rg", &coloured());
    let out = exec
        .search_output("anything", &["search", "rg"], false)
        .await
        .unwrap();
    assert!(
        !out.contains('\u{1b}') && !out.contains('\r'),
        "search_output passed raw terminal output through: {out:?}"
    );
}

/// **The ratchet.** Reading stdout without sanitizing is how the rule was lost the first time,
/// so the ways around the boundary are enumerated and each carries a reason.
///
/// A source scan, because the defect is a *new* call site — nothing the program does can
/// enumerate a read that has not been written yet. Same shape as
/// `os_native_argv_coverage_tests.rs`, for the same reason.
#[test]
fn no_raw_stdout_read_escapes_the_rule() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut raw: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut files_seen = 0usize;

    visit(&root, &mut |path, src| {
        files_seen += 1;
        let rel = path
            .strip_prefix(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let hits = raw_reads_in(&rel, src);
        if !hits.is_empty() {
            raw.insert(rel, hits);
        }
    });

    let found: BTreeSet<String> = raw.keys().cloned().collect();
    Ledger::of(
        "reading a command's stdout without sanitizing it",
        "ALLOWED",
    )
    .pairs(ALLOWED)
    .scanning_at_least(100)
    .detailing(|site| raw.get(site).map(|lines| lines.join("\n        ")))
    .remedy(
        "Read through `CommandExecutor::run_output`/`search_output`, which sanitize, or wrap \
             the read in `crate::utils::text::sanitize`. If the bytes are a FILE rather than a \
             report, that is the one case where sanitizing is wrong — trimming a file changes it.",
    )
    .audit(files_seen, &found);
}

/// Does this line read a command's stdout without sanitizing it?
///
/// A named function rather than four `continue`s inside the walk, because a predicate that
/// only exists inside a loop can only be checked by reading it — and the oracle below has to
/// be able to hand it something it must catch.
fn is_raw_stdout_read(line: &str) -> bool {
    line.contains("from_utf8_lossy(&")
        // stderr is a message for a human, not input to a parser, and it is already
        // trimmed at every site that shows one.
        && !line.contains(".stderr")
        && line.contains(".stdout")
        && !line.contains("text::sanitize")
}

/// Every unsanitized stdout read in one file, as `path:line: source`.
///
/// **`ALLOWED` is not consulted here.** A finding set with the excused files already removed
/// cannot tell a live exemption from a dead one; that subtraction belongs to [`Ledger`], which
/// does it in both directions.
fn raw_reads_in(rel: &str, src: &str) -> Vec<String> {
    let mut raw = Vec::new();
    for (n, line) in src.lines().enumerate() {
        // Everything from the first `#[cfg(test)]` on is test code, which inspects raw bytes
        // on purpose. This codebase puts test modules last, without exception.
        if line.trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        if !is_raw_stdout_read(line) {
            continue;
        }
        raw.push(format!("{rel}:{}: {}", n + 1, line.trim()));
    }
    raw
}

/// Reads of stdout that must NOT be sanitized, and why.
const ALLOWED: &[(&str, &str)] = &[(
    "src/core/git.rs",
    "`show_at_head` returns a FILE's contents, not a report. Trimming it changes the file, and \
     stripping escapes corrupts any file that legitimately contains them. The other two reads \
     in this module are sanitized; this scan is line-based and cannot tell them apart, so the \
     module is listed and `git.rs` is the one file to re-read when this exemption is revisited.",
)];

fn visit(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path, &str)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path, f);
        } else if path.extension().is_some_and(|e| e == "rs") {
            // A whole file of tests, included by its parent behind `#[cfg(test)]`, so the
            // in-file marker this scan stops at never appears in it.
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.ends_with("_test.rs") || name.ends_with("_tests.rs") {
                continue;
            }
            if let Ok(src) = std::fs::read_to_string(&path) {
                f(&path, &src);
            }
        }
    }
}

/// A gate that has never failed is a claim, not a check.
///
/// This test used to assert that three string literals declared two lines above it contained
/// substrings they visibly contained. It never called the scan, so replacing the scan's
/// predicate with `false` left it green — the exact failure its own name warns about. It now
/// drives the real predicate over a planted offender and every shape that must not read as one.
#[test]
fn the_raw_read_scan_can_actually_fail() {
    let offender = "        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();";

    let found = raw_reads_in("src/planted.rs", offender);
    assert_eq!(
        found.len(),
        1,
        "the scan did not see the one line it exists to find: {found:?}"
    );
    assert!(
        found[0].starts_with("src/planted.rs:1: "),
        "a caught site must name the file and line a reader can open: {}",
        found[0]
    );

    // The controls. Each is a way a line can look like the offender and not be one.
    for (label, src) in [
        (
            "a sanitized read is the correct shape, not a finding",
            "    Ok(crate::utils::text::sanitize(&String::from_utf8_lossy(&out.stdout)))",
        ),
        (
            "stderr is a message for a human and is deliberately out of scope",
            "    let e = String::from_utf8_lossy(&out.stderr).trim().to_string();",
        ),
        (
            "reading bytes that are not a command's stdout is not this rule's business",
            "    let body = String::from_utf8_lossy(&file_bytes).to_string();",
        ),
        (
            "test code inspects raw bytes on purpose, and the scan stops at the marker",
            "#[cfg(test)]\nlet s = String::from_utf8_lossy(&out.stdout).trim().to_string();",
        ),
    ] {
        assert!(
            raw_reads_in("src/planted.rs", src).is_empty(),
            "{label} — the scan reported it anyway: {:?}",
            raw_reads_in("src/planted.rs", src)
        );
    }

    // And the exemption mechanism itself: the same offender, in the one file ALLOWED excuses,
    // is found by the scan and subtracted by the ledger. Both halves matter — a scan that stops
    // finding it cannot notice when the exemption goes stale.
    let excused: BTreeSet<String> = ["src/core/git.rs".to_string()].into_iter().collect();
    assert_eq!(
        raw_reads_in("src/core/git.rs", offender).len(),
        1,
        "the scan must still see the excused site; ALLOWED subtracts it, the walk does not"
    );
    assert!(
        Ledger::of("planted", "ALLOWED")
            .pairs(ALLOWED)
            .unexplained_in(&excused)
            .is_empty(),
        "ALLOWED did not excuse the file it names, so the exemption list is not being consulted"
    );
}
