//! A package that installed and cannot be run.
//!
//! `linix install pub:sass` succeeds, `linix list` agrees, and typing `sass` answers "command
//! not found" — because `~/.pub-cache/bin` is not on `PATH` and nothing ever said so. LiNix
//! reported success for a package the user cannot invoke, which is the same event as a failed
//! install everywhere it matters and is reported as the opposite.
//!
//! Every per-user ecosystem has this shape: the manager installs into a directory under `$HOME`
//! and leaves putting it on `PATH` to you. System managers do not — `apt` writes to `/usr/bin`,
//! which is on every `PATH` by definition — so this says nothing about them.
//!
//! One table and one check on the shared path, not a check per backend. Eleven copies of a
//! `~/.x/bin` string is eleven chances to disagree, and the eleventh is the one that stays
//! wrong.

use std::path::PathBuf;

/// Where a manager puts the executables it installs, when that is a directory the user has to
/// put on `PATH` themselves.
///
/// `None` means "not this manager's problem": either it installs into a system directory, or
/// it installs no executables at all.
///
/// The environment variables come first in every case, because a user who set `GOBIN` or
/// `GEM_HOME` has already answered this question and a hard-coded `~/go/bin` would contradict
/// them.
pub fn user_bin_dir(backend: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let env_path = |k: &str| {
        std::env::var(k)
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };

    Some(match backend {
        "go" => env_path("GOBIN")
            .or_else(|| env_path("GOPATH").map(|p| p.join("bin")))
            .unwrap_or_else(|| home.join("go").join("bin")),
        "cargo" => env_path("CARGO_INSTALL_ROOT")
            .or_else(|| env_path("CARGO_HOME"))
            .map(|p| p.join("bin"))
            .unwrap_or_else(|| home.join(".cargo").join("bin")),
        "gem" => env_path("GEM_HOME")
            .map(|p| p.join("bin"))
            .unwrap_or_else(|| {
                home.join(".local")
                    .join("share")
                    .join("gem")
                    .join("ruby")
                    .join("bin")
            }),
        "pub" => env_path("PUB_CACHE")
            .map(|p| p.join("bin"))
            .unwrap_or_else(|| home.join(".pub-cache").join("bin")),
        "nimble" => home.join(".nimble").join("bin"),
        "luarocks" => home.join(".luarocks").join("bin"),
        "mix" => home.join(".mix").join("escripts"),
        "stack" => home.join(".local").join("bin"),
        "krew" => home.join(".krew").join("bin"),
        "pipx" => env_path("PIPX_BIN_DIR").unwrap_or_else(|| home.join(".local").join("bin")),
        "composer" => home
            .join(".config")
            .join("composer")
            .join("vendor")
            .join("bin"),
        "npm" | "yarn" | "pnpm" | "bun" => return None, // shim into a dir their own installer wires up
        _ => return None,
    })
}

/// Is `dir` one of the entries in `PATH`?
///
/// Compared after `canonicalize`, so `~/go/bin` and `/home/u/go/bin/` and a `PATH` entry
/// reached through a symlink are one directory rather than three. A path that cannot be
/// canonicalised has not been created yet, and a directory that does not exist is not on
/// anyone's `PATH` — but the raw comparison still runs first, because a user may well have
/// added the entry before the manager created the directory.
pub fn is_on_path(dir: &std::path::Path) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let canonical = dir.canonicalize().ok();
    std::env::split_paths(&path)
        .any(|p| p == dir || (canonical.is_some() && p.canonicalize().ok() == canonical))
}

/// The command that puts `dir` on `PATH`, in the form this platform actually takes.
///
/// `export PATH=…` on Windows is not a smaller fix, it is a wrong one: it names a shell the
/// user is not in and a syntax cmd and PowerShell both reject. A line someone has to translate
/// before they can run it is the "add it to your PATH" advice this warning exists to replace.
fn how_to_add(dir: &std::path::Path) -> String {
    if cfg!(windows) {
        format!("setx PATH \"%PATH%;{}\"", dir.display())
    } else {
        format!("export PATH=\"{}:$PATH\"", dir.display())
    }
}

/// The sentence a user can act on, or `None` when there is nothing to say.
///
/// Names the directory and the exact command, because "add it to your PATH" is advice and a
/// line you can paste is a fix. A warning, never a refusal: the package really did install,
/// and the machine really is closer to the files than it was.
pub fn unreachable_warning(backend: &str) -> Option<String> {
    let dir = user_bin_dir(backend)?;
    if is_on_path(&dir) {
        return None;
    }
    Some(format!(
        "`{backend}` installs its executables into {}, which is not on your PATH — so what it \
         just installed will answer \"command not found\".\n  Put it on your PATH with:\n    {}",
        dir.display(),
        how_to_add(&dir)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reported case. `pub` installs to `~/.pub-cache/bin`, and the whole finding is that
    /// nothing mentioned it.
    #[test]
    fn a_per_user_ecosystem_names_its_bin_dir() {
        let dir = user_bin_dir("pub").expect("pub installs executables under $HOME");
        assert!(
            dir.ends_with("bin"),
            "a bin dir that is not a bin dir: {}",
            dir.display()
        );
    }

    /// The family, not the finding. Every ecosystem that installs into `$HOME` and leaves the
    /// PATH to you must answer, or it is the next `pub`.
    #[test]
    fn every_per_user_ecosystem_answers() {
        for be in [
            "pub", "nimble", "go", "cargo", "gem", "luarocks", "mix", "stack", "krew", "pipx",
            "composer",
        ] {
            assert!(
                user_bin_dir(be).is_some(),
                "{be} installs into a user directory and this table does not know where"
            );
        }
    }

    /// And the other half: a system manager must NOT warn. `/usr/bin` is on every PATH, so a
    /// warning about it is noise on every install, and noise is how a real warning gets
    /// ignored.
    #[test]
    fn a_system_manager_has_nothing_to_warn_about() {
        for be in [
            "apt", "dnf", "pacman", "apk", "brew", "scoop", "winget", "choco",
        ] {
            assert!(
                user_bin_dir(be).is_none(),
                "{be} installs into a system directory; warning about its PATH is noise"
            );
            assert!(unreachable_warning(be).is_none());
        }
    }

    /// A directory that IS on PATH produces no warning — otherwise every install on a
    /// correctly configured machine nags.
    #[test]
    fn a_reachable_directory_is_not_warned_about() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("bin");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_on_path(&dir), "a fresh temp dir cannot be on PATH");

        let old = std::env::var_os("PATH");
        std::env::set_var("PATH", &dir);
        let seen = is_on_path(&dir);
        match old {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }
        assert!(seen, "a directory that IS on PATH was reported as missing");
    }

    /// The message has to carry the fix, not the diagnosis. A warning naming neither the
    /// directory nor the line to add is one more thing to look up.
    #[test]
    fn the_warning_names_the_directory_and_the_line_to_add() {
        // Pick whichever of these is genuinely unreachable on the machine running the test;
        // asserting on a fixed one makes this pass or fail on the host's PATH rather than on
        // the code.
        let Some((be, msg)) = ["pub", "composer", "mix", "krew", "luarocks", "nimble"]
            .into_iter()
            .find_map(|b| unreachable_warning(b).map(|m| (b, m)))
        else {
            // Every one of them is on PATH here. Nothing to assert, and nothing wrong.
            return;
        };
        let dir = user_bin_dir(be).unwrap();
        // The instruction has to be runnable on THIS platform: `export PATH=` in a cmd window
        // is a wrong answer, not a terse one.
        assert!(
            msg.contains(if cfg!(windows) {
                "setx PATH"
            } else {
                "export PATH="
            }),
            "the fix is not in this platform's syntax: {msg}"
        );
        assert!(
            msg.contains(&dir.display().to_string()),
            "no directory: {msg}"
        );
        assert!(msg.contains(be), "does not say which manager: {msg}");
    }
}
