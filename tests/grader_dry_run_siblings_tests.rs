//! GRADER, 2026-07-28 — RED. Four more commands that `--dry-run` performs for real.
//!
//! `tests/dry_run_tests.rs` fixed the reported case (`uninstall`) and its nearest neighbours
//! (`unmanage`, `module create`, `schedule add`). The flag is still consulted per-verb, so the
//! question was never "is uninstall fixed" but "which verbs remembered". Measured against the
//! real binary on a fresh config, these four did not:
//!
//! | command                      | what it did during a preview                      |
//! |------------------------------|---------------------------------------------------|
//! | `--dry-run activate <p>`     | switched the active profile, and printed nothing   |
//! | `--dry-run deactivate <p>`   | emptied `active`, and printed nothing              |
//! | `--dry-run lock`             | wrote `locks/versions.json` and `locks/hooks.toml` |
//! | `--dry-run git init`         | created the repo and committed                     |
//! | `--dry-run config init`      | wrote `preferences.toml`                           |
//!
//! `activate` and `deactivate` are the ones that matter. They decide which modules are in the
//! model, so they decide what the next `sync` installs and removes. A user who previews
//! "what happens if I switch to Work" has switched to Work, and the command said nothing.
//!
//! Every case is paired with a control that runs the same command *without* the flag. An
//! assertion that a file did not change is worth nothing if the setup could not have changed
//! it — which is how the first draft of this file scored `activate` as passing, on a fixture
//! where the profile it activated was already active.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Fresh {
    dir: PathBuf,
}

impl Fresh {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fresh = Self { dir };
        let (out, code) = fresh.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        fresh
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
            .env("LINIX_CONFIG_DIR", self.dir.join("config"))
            .env("LINIX_DATA_DIR", self.dir.join("data"))
            .stdin(std::process::Stdio::null())
            .output()
            .expect("the binary should run");
        (
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            out.status.code().unwrap_or(-1),
        )
    }

    fn config(&self, rel: &str) -> PathBuf {
        self.dir.join("config").join(rel)
    }

    /// Every readable file under the config repo, so a preview is asserted against the whole
    /// repo rather than the one file the test author thought of.
    fn snapshot(&self) -> Vec<(PathBuf, String)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if let Ok(body) = std::fs::read_to_string(&p) {
                    out.push((p, body));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.dir.join("config"), &mut out);
        out.sort();
        out
    }
}

fn unchanged(before: &[(PathBuf, String)], after: &[(PathBuf, String)]) -> Option<String> {
    if before == after {
        return None;
    }
    let names = |v: &[(PathBuf, String)]| {
        v.iter()
            .map(|(p, _)| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>()
    };
    Some(format!(
        "before: {:?}\nafter:  {:?}",
        names(before),
        names(after)
    ))
}

#[test]
fn a_previewed_activate_switches_nothing() {
    let fresh = Fresh::new("dry-run-activate");
    let (out, _) = fresh.run(&["profile", "create", "Work"]);
    assert!(
        fresh.config("profiles/Work").exists(),
        "fixture setup:\n{out}"
    );

    let before = fresh.snapshot();
    let (preview, code) = fresh.run(&["--dry-run", "activate", "Work"]);
    let after = fresh.snapshot();

    // The control: without the flag the same command MUST change `active`, or this test is
    // asserting over a command that could not have done anything either way.
    let (_, ctl) = fresh.run(&["activate", "Work"]);
    assert_eq!(ctl, 0, "the control `activate Work` did not succeed");
    assert!(
        std::fs::read_to_string(fresh.config("active"))
            .unwrap_or_default()
            .contains("Work"),
        "the control did not change `active`, so the preview assertion proves nothing"
    );

    if let Some(diff) = unchanged(&before, &after) {
        panic!(
            "`--dry-run activate Work` changed the config (rc={code}).\n{diff}\n\
             It also printed nothing: {:?}\n\
             The active profile decides which modules are in the model, so this preview \
             changes what the next `sync` installs and removes.",
            preview.trim()
        );
    }
}

#[test]
fn a_previewed_deactivate_deactivates_nothing() {
    let fresh = Fresh::new("dry-run-deactivate");
    let active_before = std::fs::read_to_string(fresh.config("active")).unwrap_or_default();
    assert!(
        active_before.contains("Main"),
        "fixture: expected Main active, got {active_before:?}"
    );

    let before = fresh.snapshot();
    let (preview, code) = fresh.run(&["--dry-run", "deactivate", "Main"]);
    let after = fresh.snapshot();

    if let Some(diff) = unchanged(&before, &after) {
        let now = std::fs::read_to_string(fresh.config("active")).unwrap_or_default();
        panic!(
            "`--dry-run deactivate Main` changed the config (rc={code}).\n{diff}\n\
             `active` went from {active_before:?} to {now:?}, and the command printed {:?}.",
            preview.trim()
        );
    }
}

#[test]
fn a_previewed_lock_pins_nothing() {
    let fresh = Fresh::new("dry-run-lock");
    let before = fresh.snapshot();
    let (preview, code) = fresh.run(&["--dry-run", "lock"]);
    let after = fresh.snapshot();

    let (_, ctl) = fresh.run(&["lock"]);
    assert_eq!(ctl, 0, "the control `lock` did not succeed");
    assert!(
        fresh.config("locks/versions.json").exists(),
        "the control wrote no lockfile, so the preview assertion proves nothing"
    );

    if let Some(diff) = unchanged(&before, &after) {
        panic!(
            "`--dry-run lock` wrote the lockfile (rc={code}).\n{diff}\n\
             It reported the write in the past tense: {:?}",
            preview.trim()
        );
    }
}

#[test]
fn a_previewed_git_init_commits_nothing() {
    let fresh = Fresh::new("dry-run-git-init");
    let (preview, code) = fresh.run(&["--dry-run", "git", "init"]);
    let dotgit = fresh.config(".git");
    assert!(
        !dotgit.exists(),
        "`--dry-run git init` created a real repository at {} (rc={code}).\n\
         It reported the commit in the past tense: {:?}",
        dotgit.display(),
        preview.trim()
    );
}

#[test]
fn a_previewed_config_init_writes_nothing() {
    let fresh = Fresh::new("dry-run-config-init");
    let prefs = fresh.config("preferences.toml");
    let existed = prefs.exists();
    let (preview, code) = fresh.run(&["--dry-run", "config", "init"]);
    assert!(
        existed || !prefs.exists(),
        "`--dry-run config init` wrote {} (rc={code}): {:?}",
        prefs.display(),
        preview.trim()
    );
}
