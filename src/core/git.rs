// Version control for LiNix's *intent* — the manifest/config directory.
//
// Filesystem snapshots version the *effect* of a change — the whole disk. Git is the other
// half and the complementary one: the human-readable, diffable, branchable, pushable history
// of what you *asked for* (II.13). `git diff` shows "you added ripgrep, removed nano"; a
// remote backs your whole setup up like dotfiles. There is no generation format; a generation
// IS a commit.
//
// This is a thin, dependency-free wrapper that shells out to the system `git` (LiNix already
// shells out to every package manager, so this adds no new dependency and no libgit2 build
// cost). Every method that could fail on a machine without git returns a `Result` the caller
// can degrade gracefully on — auto-commit, for instance, simply no-ops when git is absent.
//
// The repo root is the LiNix config directory, so a single repo captures `preferences.toml`,
// `modules/`, `profiles/`, `active`, `priority` and `locks/` together.

use crate::core::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::Output;

/// A commit as shown by `git log` — the data `linix git log` renders. A generation IS a
/// commit (II.13), so this is the whole of the history record.
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
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn is_repo(&self) -> bool {
        self.root.join(".git").exists()
    }

    /// Identity and signing flags are injected on every call so commits never fail on a
    /// machine that hasn't set `user.name`/`user.email` or that has signing globally on.
    fn run(&self, args: &[&str]) -> Result<Output> {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(&self.root);
        // A deterministic identity, without mutating the user's global config. Signing is
        // NOT forced off here: II.13 makes integrity `git commit -S`, and an override that
        // guaranteed every LiNix commit was unsigned made that unreachable by construction.
        // Whether a commit is signed is the user's `commit.gpgsign` to answer.
        cmd.args([
            "-c",
            "user.name=linix",
            "-c",
            "user.email=linix@localhost",
        ]);
        cmd.args(args);
        cmd.output()
            .map_err(|e| Error::CommandFailed(format!("git {:?} failed to spawn: {}", args, e)))
    }

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

    /// A non-fast-forward or missing remote surfaces as an error the caller can downgrade
    /// to a warning — `watch --pull` must not abort a sync because a remote moved.
    pub fn pull(&self) -> Result<String> {
        self.run_checked(&["pull", "--ff-only"])
    }

    /// Idempotent. The written `.gitignore` excludes the per-file backups LiNix drops
    /// during rollbacks, which would otherwise be committed as manifest content.
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
        let ignore = self.root.join(".gitignore");
        let existing = std::fs::read_to_string(&ignore).unwrap_or_default();
        let mut body = existing.clone();
        for pat in ["*.linix-backup"] {
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
            Ok(Some(
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
            ))
        } else {
            Ok(None) // no commits yet
        }
    }

    /// The content of a tracked file as of HEAD, or `None` when the file was not tracked there
    /// or the repo has no commits yet. Since LiNix commits only on a successful sync (V.30),
    /// HEAD is the last-synced state — the baseline for showing what a working-tree edit changed.
    pub fn show_at_head(&self, relpath: &str) -> Result<Option<String>> {
        if self.head()?.is_none() {
            return Ok(None);
        }
        let out = self.run(&["show", &format!("HEAD:{}", relpath)])?;
        if out.status.success() {
            Ok(Some(String::from_utf8_lossy(&out.stdout).to_string()))
        } else {
            Ok(None)
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

    /// The manifest lines a commit added or removed — the package-level story of that commit.
    /// `git show` limited to the config files, keeping only the
    /// `+`/`-` content lines (diff headers and comments dropped). Empty for a commit that
    /// touched no manifests. This is what replaced the generation format's stored package sets:
    /// git already records exactly what each change did to your manifests.
    pub fn commit_manifest_changes(&self, reference: &str) -> Result<Vec<String>> {
        if !self.is_repo() {
            return Ok(vec![]);
        }
        let raw = self.run_checked(&[
            "show",
            "--format=",
            "--no-color",
            reference,
            "--",
            "modules",
            "profiles",
            "active",
            "priority",
            "schedules",
            // `vars`, `vars.linix`, `vars.py` … — the file that explains a change must be in the
            // change view, or a variable edit that removed a hundred packages is invisible (W14).
            "vars*",
        ])?;
        Ok(parse_manifest_changes(&raw))
    }

    /// Write a `git bundle` of the whole repo to `dest` — every commit and ref in one file,
    /// for an air-gapped transfer. `git clone <dest>` on the far side reconstructs the repo
    /// with its full history, so the recipient can `rollback` to any past commit, not just
    /// restore the current manifests. Returns `Ok(false)` (nothing written) when there is no
    /// repo or no commits yet — a bundle honestly reports what it could not include.
    pub fn bundle(&self, dest: &Path) -> Result<bool> {
        if !self.is_repo() || self.head()?.is_none() {
            return Ok(false);
        }
        let dest = dest.to_string_lossy().to_string();
        self.run_checked(&["bundle", "create", &dest, "--all"])?;
        Ok(true)
    }

    /// The manifest lines that differ between two commits — `linix diff <from> <to>` in
    /// packages, not text (Phase 4). `from` is the older baseline; pass `to = None` to diff
    /// `from` against the working tree (committed + uncommitted). Limited to the config files,
    /// keeping only the `+`/`-` content lines. Because manifests are package declarations, this
    /// diff IS the package-level story: what you'd add or remove going from one to the other.
    pub fn diff_manifest_changes(&self, from: &str, to: Option<&str>) -> Result<Vec<String>> {
        if !self.is_repo() {
            return Ok(vec![]);
        }
        let range = match to {
            Some(to) => format!("{}..{}", from, to),
            None => from.to_string(),
        };
        let raw = self.run_checked(&[
            "diff",
            "--no-color",
            &range,
            "--",
            "modules",
            "profiles",
            "active",
            "priority",
            "schedules",
            "vars*",
        ])?;
        Ok(parse_manifest_changes(&raw))
    }
}

/// Extract the `+`/`-` content lines from a `git show` diff — the added and removed manifest
/// lines — skipping the `+++`/`---` file headers and blank/comment lines.
fn parse_manifest_changes(diff: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in diff.lines() {
        let keep_plus = line.starts_with('+') && !line.starts_with("+++");
        let keep_minus = line.starts_with('-') && !line.starts_with("---");
        if keep_plus || keep_minus {
            let (sign, body) = line.split_at(1);
            let body = body.trim();
            if !body.is_empty() && !body.starts_with('#') {
                out.push(format!("{} {}", sign, body));
            }
        }
    }
    out
}

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
    fn parse_manifest_changes_keeps_content_lines_drops_headers() {
        // A realistic `git show` diff body: file headers, hunk header, context, +/- lines.
        let diff = "diff --git a/modules/dev.txt b/modules/dev.txt\n\
                    index 111..222 100644\n\
                    --- a/modules/dev.txt\n\
                    +++ b/modules/dev.txt\n\
                    @@ -1,3 +1,3 @@\n\
                    \x20apt:curl\n\
                    -apt:nano\n\
                    +cargo:ripgrep\n\
                    +# a comment line, not a package\n";
        let changes = parse_manifest_changes(diff);
        // Kept: the real +/- package lines. Dropped: ---/+++ headers, context, and the comment.
        assert_eq!(changes, vec!["- apt:nano".to_string(), "+ cargo:ripgrep".to_string()]);
    }

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
    fn diff_manifest_changes_reports_package_level_delta() {
        if !GitManager::git_available() {
            eprintln!("skipping: git not installed");
            return;
        }
        let tmp = tempdir().unwrap();
        let git = GitManager::new(tmp.path());
        git.init().unwrap();
        std::fs::create_dir_all(tmp.path().join("modules")).unwrap();

        std::fs::write(tmp.path().join("modules/dev.txt"), "apt:curl\napt:nano\n").unwrap();
        git.commit_all("base").unwrap();
        std::fs::write(tmp.path().join("modules/dev.txt"), "apt:curl\ncargo:ripgrep\n").unwrap();
        git.commit_all("swap nano for ripgrep").unwrap();

        let changes = git.diff_manifest_changes("HEAD~1", Some("HEAD")).unwrap();
        // Package-level delta: nano removed, ripgrep added, curl untouched (in neither).
        assert!(changes.contains(&"- apt:nano".to_string()), "{:?}", changes);
        assert!(changes.contains(&"+ cargo:ripgrep".to_string()), "{:?}", changes);
        assert!(
            !changes.iter().any(|c| c.contains("apt:curl")),
            "unchanged lines must not appear: {:?}",
            changes
        );
    }

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

        std::fs::write(tmp.path().join("local.txt"), "apt:curl\n").unwrap();
        let first = git.commit_all("add curl").unwrap();
        assert!(first.is_some());
        let head = git.head().unwrap().unwrap();
        assert_eq!(head, first.unwrap());

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
        assert_eq!(
            restored, "apt:curl\n",
            "working tree restored to the v1 manifest"
        );
    }
}
