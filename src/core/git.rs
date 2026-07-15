// src/core/git.rs
//
// Version control for LiNix's *intent* — the manifest/config directory.
//
// LiNix already versions the *effect* of a change two ways: generations (the realized
// package set + a frozen manifest copy) and filesystem snapshots (the whole disk). Git adds
// the third, complementary layer: the human-readable, diffable, branchable, pushable history
// of what you *asked for*. `git diff` shows "you added ripgrep, removed nano"; a remote backs
// your whole setup up like dotfiles.
//
// This is a thin, dependency-free wrapper that shells out to the system `git` (LiNix already
// shells out to every package manager, so this adds no new dependency and no libgit2 build
// cost). Every method that could fail on a machine without git returns a `Result` the caller
// can degrade gracefully on — auto-commit, for instance, simply no-ops when git is absent.
//
// The repo root is the LiNix config directory (the parent of `groups/`), so a single repo
// captures `config.toml`, `groups/`, `modules/`, and `profiles/` together.

use crate::core::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::Output;

/// A commit as shown by `git log` — the data `linix git log` renders and `Generation`
/// stamping records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommit {
    pub hash: String,
    pub short: String,
    pub date: String,
    pub subject: String,
}

/// A git wrapper scoped to one directory (the LiNix config root).
#[derive(Debug, Clone)]
pub struct GitManager {
    root: PathBuf,
}

impl GitManager {
    /// Wrap the given directory as the repo root.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The repo root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Is `git` available on this machine at all?
    pub fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Is the root already a git repository?
    pub fn is_repo(&self) -> bool {
        self.root.join(".git").exists()
    }

    /// Run a git subcommand in the root, returning the raw `Output`. Identity and signing
    /// flags are injected on every call so commits never fail on a machine that hasn't set
    /// `user.name`/`user.email` or that has commit signing globally enabled.
    fn run(&self, args: &[&str]) -> Result<Output> {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(&self.root);
        // Deterministic identity + no GPG prompt, without mutating the user's global config.
        cmd.args([
            "-c",
            "user.name=linix",
            "-c",
            "user.email=linix@localhost",
            "-c",
            "commit.gpgsign=false",
        ]);
        cmd.args(args);
        cmd.output()
            .map_err(|e| Error::CommandFailed(format!("git {:?} failed to spawn: {}", args, e)))
    }

    /// Like [`run`], but treats a non-zero exit as an error carrying git's stderr.
    fn run_checked(&self, args: &[&str]) -> Result<String> {
        let out = self.run(args)?;
        if !out.status.success() {
            return Err(Error::CommandFailed(format!(
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Pull the latest manifests from the tracking remote (`git pull --ff-only`). Used by
    /// `watch --pull` to reconcile against a remote GitOps manifest repo. Returns git's
    /// summary line; a non-fast-forward or missing remote surfaces as an error the caller
    /// can downgrade to a warning.
    pub fn pull(&self) -> Result<String> {
        self.run_checked(&["pull", "--ff-only"])
    }

    /// Initialize the config directory as a git repo and write a sensible `.gitignore`
    /// (ignoring the per-file backups LiNix drops during rollbacks). Idempotent.
    pub fn init(&self) -> Result<()> {
        if !Self::git_available() {
            return Err(Error::Other(
                "git is not installed; install it to use `linix git`".into(),
            ));
        }
        std::fs::create_dir_all(&self.root).map_err(Error::from)?;
        if !self.is_repo() {
            self.run_checked(&["init"])?;
        }
        // Ignore rollback backups AND the machine-local lock-signing secret — the latter must
        // never be committed (it would defeat lockfile tamper-evidence and leak between hosts).
        let ignore = self.root.join(".gitignore");
        let existing = std::fs::read_to_string(&ignore).unwrap_or_default();
        let mut body = existing.clone();
        for pat in ["*.linix-backup", ".linix-lock.key"] {
            if !existing.lines().any(|l| l.trim() == pat) {
                if !body.is_empty() && !body.ends_with('\n') {
                    body.push('\n');
                }
                body.push_str(pat);
                body.push('\n');
            }
        }
        if body != existing {
            let _ = std::fs::write(&ignore, body);
        }
        Ok(())
    }

    /// Stage everything and commit. Returns `Ok(Some(hash))` when a commit was created, or
    /// `Ok(None)` when there was nothing to commit (a clean tree — not an error). Callers use
    /// the `None` case to stay quiet on no-op runs.
    pub fn commit_all(&self, message: &str) -> Result<Option<String>> {
        if !self.is_repo() {
            return Err(Error::Other(format!(
                "{} is not a git repo; run `linix git init` first",
                self.root.display()
            )));
        }
        self.run_checked(&["add", "-A"])?;
        // If the index has no staged changes, `git commit` exits non-zero. Detect that
        // cleanly rather than surfacing it as a failure.
        let status = self.run_checked(&["status", "--porcelain"])?;
        if status.is_empty() {
            return Ok(None);
        }
        self.run_checked(&["commit", "-m", message])?;
        Ok(Some(self.head()?.unwrap_or_default()))
    }

    /// The current HEAD commit hash, or `None` if the repo has no commits yet.
    pub fn head(&self) -> Result<Option<String>> {
        if !self.is_repo() {
            return Ok(None);
        }
        let out = self.run(&["rev-parse", "HEAD"])?;
        if out.status.success() {
            Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()))
        } else {
            Ok(None) // no commits yet
        }
    }

    /// Restore the working tree of the config directory to a given commit/ref WITHOUT moving
    /// HEAD — i.e. roll back your *manifests* to a past state, leaving installed packages
    /// untouched. This is the "config half" of a rollback.
    pub fn checkout_files(&self, reference: &str) -> Result<()> {
        if !self.is_repo() {
            return Err(Error::Other("not a git repo".into()));
        }
        self.run_checked(&["checkout", reference, "--", "."])?;
        Ok(())
    }

    /// The most recent `limit` commits, newest first.
    pub fn log(&self, limit: usize) -> Result<Vec<GitCommit>> {
        if !self.is_repo() || self.head()?.is_none() {
            return Ok(vec![]);
        }
        // Unit-separated fields, record-separated lines — robust against subjects with spaces.
        let fmt = "--pretty=format:%H%x1f%h%x1f%cs%x1f%s";
        let raw = self.run_checked(&["log", &format!("-{}", limit.max(1)), fmt])?;
        Ok(parse_log(&raw))
    }

    /// Short status (porcelain) of the config repo, or an empty string if clean.
    pub fn status_porcelain(&self) -> Result<String> {
        if !self.is_repo() {
            return Err(Error::Other("not a git repo".into()));
        }
        self.run_checked(&["status", "--porcelain"])
    }
}

/// Pure: parse `git log` output in our `%H\x1f%h\x1f%cs\x1f%s` format into commits.
fn parse_log(raw: &str) -> Vec<GitCommit> {
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let hash = parts.next()?.to_string();
            let short = parts.next()?.to_string();
            let date = parts.next()?.to_string();
            let subject = parts.next().unwrap_or("").to_string();
            Some(GitCommit {
                hash,
                short,
                date,
                subject,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_log_handles_subjects_with_spaces_and_separators() {
        let raw = "abc123\u{1f}abc\u{1f}2026-07-15\u{1f}feat: add ripgrep, remove nano\n\
                   def456\u{1f}def\u{1f}2026-07-14\u{1f}initial commit";
        let commits = parse_log(raw);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "abc123");
        assert_eq!(commits[0].short, "abc");
        assert_eq!(commits[0].date, "2026-07-15");
        assert_eq!(commits[0].subject, "feat: add ripgrep, remove nano");
        assert_eq!(commits[1].subject, "initial commit");
    }

    #[test]
    fn parse_log_skips_blank_lines() {
        assert!(parse_log("\n\n").is_empty());
    }

    // The following tests exercise real git; they self-skip when git is unavailable so the
    // suite still passes in a minimal environment.
    #[test]
    fn init_commit_head_and_log_round_trip() {
        if !GitManager::git_available() {
            eprintln!("skipping: git not installed");
            return;
        }
        let tmp = tempdir().unwrap();
        let git = GitManager::new(tmp.path());
        assert!(!git.is_repo());
        git.init().unwrap();
        assert!(git.is_repo());
        assert!(git.head().unwrap().is_none(), "no commits yet");

        // Nothing but the .gitignore init wrote — commit it.
        std::fs::write(tmp.path().join("local.txt"), "apt:curl\n").unwrap();
        let first = git.commit_all("add curl").unwrap();
        assert!(first.is_some());
        let head = git.head().unwrap().unwrap();
        assert_eq!(head, first.unwrap());

        // A clean tree commits to nothing.
        assert!(git.commit_all("noop").unwrap().is_none());

        let log = git.log(10).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].subject, "add curl");
    }

    #[test]
    fn checkout_files_restores_manifest_without_new_commit() {
        if !GitManager::git_available() {
            eprintln!("skipping: git not installed");
            return;
        }
        let tmp = tempdir().unwrap();
        let git = GitManager::new(tmp.path());
        git.init().unwrap();
        let manifest = tmp.path().join("local.txt");

        std::fs::write(&manifest, "apt:curl\n").unwrap();
        let c1 = git.commit_all("v1").unwrap().unwrap();
        std::fs::write(&manifest, "apt:curl\napt:htop\n").unwrap();
        git.commit_all("v2").unwrap();

        // Roll the manifest back to v1's content; installed packages are irrelevant here.
        git.checkout_files(&c1).unwrap();
        // Normalize line endings: git on Windows may apply autocrlf on checkout. LiNix reads
        // manifests via `.lines()`, which tolerates CRLF, so this is cosmetic.
        let restored = std::fs::read_to_string(&manifest)
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(restored, "apt:curl\n", "working tree restored to the v1 manifest");
    }
}
