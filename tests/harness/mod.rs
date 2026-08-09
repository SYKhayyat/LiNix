//! **One `Fixture`.**
//!
//! The same twenty-five lines — a temp root, `linix init`, `run(&["…"])`, collect stdout and
//! stderr — were written **sixteen times**, and they had already drifted three ways:
//!
//! - **`current_dir`** was set by 3 of 16. The other thirteen ran the binary in the *repository
//!   root*, where a stray `linix.txt` is a project-local shell manifest the product reads.
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
    /// A fresh root under `CARGO_TARGET_TMPDIR` with `linix init` already run in it.
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
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
            // Every one of these four is here because a test that omitted it was measuring the
            // developer's machine rather than the fixture.
            .current_dir(&self.root)
            .env("LINIX_CONFIG_DIR", self.cfg())
            .env("LINIX_DATA_DIR", self.data())
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
