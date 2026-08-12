//! **One `Fixture`.**
//!
//! The same twenty-five lines — a temp root, `shall init`, `run(&["…"])`, collect stdout and
//! stderr — were written **sixteen times**, and they had already drifted three ways:
//!
//! - **`current_dir`** was set by 3 of 16. The other thirteen ran the binary in the *repository
//!   root*, where a stray `shall.txt` is a project-local shell manifest the product reads.
//! - **`HOME`/`USERPROFILE`** was set by 3 of 16. The other thirteen resolved `~` to the machine's
//!   real home directory — so a test that placed a `link:` at `~/.vimrc` was writing to the
//!   developer's actual dotfiles, and passed or failed depending on where the checkout happened
//!   to sit relative to `$HOME`.
//! - **`cfg()`** existed in 11 of 16, and the five without it spelled `root.join("config")` inline.
//!
//! This is the union, and the union is the correct one in every case: a test process that cannot
//! see the developer's home directory or the repository's own working directory is the only kind
//! whose result means anything.
//!
//! **Bespoke helpers stay in their own files.** An inherent `impl` may live in any module of the
//! crate, so a test that needs `dest_mtimes()` or `seed_ledger()` keeps writing
//! `impl Fixture { … }` beside the tests that use it. What moved here is only what was identical.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub struct Fixture {
    pub root: PathBuf,
}

impl Fixture {
    /// A fresh root under `CARGO_TARGET_TMPDIR` with `shall init` already run in it.
    ///
    /// The root is removed first: `CARGO_TARGET_TMPDIR` persists between runs, so a fixture that
    /// only creates is a fixture carrying yesterday's state into today's assertion.
    pub fn new(name: &str) -> Self {
        let f = Self::bare(name);
        let (out, code) = f.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        f
    }

    /// The same root, without `init` — for the tests that are *about* an unwritten repository.
    pub fn bare(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub fn cfg(&self) -> PathBuf {
        self.root.join("config")
    }

    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    /// Write a file into the config repo, creating its parents.
    pub fn write(&self, rel: &str, body: &str) {
        let p = self.cfg().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// Run the binary and return everything it said, plus its exit code.
    ///
    /// stdout and stderr are joined because almost every caller wants "what did it say"; the two
    /// that need them apart use [`run_split`](Self::run_split).
    pub fn run(&self, args: &[&str]) -> (String, i32) {
        let (out, err, code) = self.run_split(args);
        (format!("{out}{err}"), code)
    }

    /// The two streams, kept apart — for the tests asserting that a document goes to stdout and
    /// its diagnostics do not.
    pub fn run_split(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_shall"))
            .args(args)
            // Every one of these four is here because a test that omitted it was measuring the
            // developer's machine rather than the fixture.
            .current_dir(&self.root)
            .env("SHALL_CONFIG_DIR", self.cfg())
            .env("SHALL_DATA_DIR", self.data())
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root)
            // A test that blocks on a prompt is a test that hangs CI rather than failing it.
            .stdin(std::process::Stdio::null())
            .output()
            .expect("the binary should run");
        (
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
            out.status.code().unwrap_or(-1),
        )
    }

    /// `run`, and how long it took in milliseconds.
    pub fn timed(&self, args: &[&str]) -> ((String, i32), u128) {
        let started = Instant::now();
        let answer = self.run(args);
        (answer, started.elapsed().as_millis())
    }

    /// Write the starter module. Two files had this, byte for byte.
    pub fn write_module(&self, body: &str) {
        std::fs::write(self.cfg().join("modules/starter.txt"), body).unwrap();
    }

    /// Put rows into the applied-extras ledger, so a test can start from "this was already
    /// placed" without placing it. Two files had this, byte for byte.
    pub fn seed_ledger(&self, keys: &[&str]) {
        let locks = self.cfg().join("locks");
        std::fs::create_dir_all(&locks).unwrap();
        let body = format!(
            "applied = [{}]\n",
            keys.iter()
                .map(|k| format!("{:?}", k))
                .collect::<Vec<_>>()
                .join(", ")
        );
        std::fs::write(locks.join("extras.toml"), body).unwrap();
    }
}

/// A path as a module line may spell it.
///
/// **Forward slashes, always.** The grammar reads `\` as an escape, so a Windows path written
/// raw into a module does not survive the parse. Three files had this function, byte for byte,
/// and each explained it in its own words.
pub fn decl(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// The registry's source, which is a **directory** rather than a file.
///
/// `src/backends/registry.rs` was 4,237 lines of which 1,800 were `fn register_*` bodies in
/// declaration order, so "what else is like apt" had no answer but scrolling. It became
/// `registry/mod.rs` plus one module per manager family — and eight scanning gates that read it
/// by path failed in the same second.
///
/// **That is the cost of a split in a repo whose gates are source scans, and it is the argument
/// for having them, not against it.** Every one of those failures was a gate correctly reporting
/// that it could no longer see what it was built to watch. They read this instead, so the next
/// split moves one line rather than eight.
pub fn registry_source() -> String {
    // **Normalised twice, and both are the split's artifacts rather than the code's.**
    //
    // `pub(super) fn register_apt` is the same registrar `fn register_apt` was; the visibility
    // exists only because the parent module now has to call across a file boundary. Six scans
    // keyed on a leading `fn register_` found nothing and reported the *code* broken.
    //
    // And the `#[cfg(test)]` module goes, because these gates read production registrations and
    // a test was never their subject. It mattered the moment `register_generic` stopped being
    // followed by another registrar: the scan that ends one registrar at the next `fn` ran on
    // into the test module, found a `ManagerConfig {` there, and reported a helper as a backend
    // that drops its version pin.
    without_test_modules(&source_of("src/backends/registry")).replace("pub(super) fn ", "fn ")
}

/// The body of the argv table — the fixture that names every backend's real command line.
///
/// A separate reader because it lives *inside* `#[cfg(test)]`, which [`registry_source`] strips:
/// three gates count the build's backends from these rows, and stripping the test module took
/// the table with it. Two questions, two functions, rather than one function that answers
/// whichever the caller happened to want.
pub fn registry_argv_table() -> String {
    let src = source_of("src/backends/registry");
    src.split_once("    fn argv_cases() -> Vec<ArgvCase> {")
        .expect("the argv table moved or was renamed")
        .1
        .split_once(
            "
    }",
        )
        .expect("the argv table has no end")
        .0
        .to_string()
}

/// Drop every `#[cfg(test)]` module, by matching braces rather than by looking for the next
/// blank line — the distinction between production and test code is a nesting question, and
/// every scan in this repo that got it wrong got it wrong by guessing at a delimiter.
fn without_test_modules(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(at) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        let Some(open) = after.find('{') else {
            break;
        };
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in after[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        match end {
            Some(e) => rest = &after[e..],
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// The executor's source, which is three files: the executors themselves, [how a program is
/// found and launched](../../src/core/launch.rs), and [how a child is watched and
/// killed](../../src/core/supervise.rs).
pub fn executor_source() -> String {
    [
        read_source("src/core/executor.rs"),
        read_source("src/core/launch.rs"),
        read_source("src/core/supervise.rs"),
    ]
    .join("\n")
}

fn read_source(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
        .replace("\r\n", "\n")
}

/// Every `.rs` file in a directory, concatenated in name order so the result is the same twice
/// running.
fn source_of(rel: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    assert!(
        !files.is_empty(),
        "{} holds no Rust source; a scan over it would pass over nothing",
        dir.display()
    );
    files.sort();
    files
        .iter()
        .map(|p| {
            std::fs::read_to_string(p)
                .unwrap_or_default()
                .replace("\r\n", "\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
