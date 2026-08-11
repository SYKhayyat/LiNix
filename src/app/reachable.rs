//! A package that installed and cannot be run.
//!
//! `shall install pub:sass` succeeds, `shall list` agrees, and typing `sass` answers "command
//! not found" — because `~/.pub-cache/bin` is not on `PATH` and nothing ever said so. Shall
//! reported success for a package the user cannot invoke, which is the same event as a failed
//! install everywhere it matters and is reported as the opposite.
//!
//! Every per-user ecosystem has this shape: the manager installs into a directory under `$HOME`
//! and leaves putting it on `PATH` to you. System managers do not — `apt` writes to `/usr/bin`,
//! which is on every `PATH` by definition — so this says nothing about them.
//!
//! One place and one check on the shared path, not a check per backend. Eleven copies of a
//! `~/.x/bin` string is eleven chances to disagree, and the eleventh is the one that stays
//! wrong.
//!
//! **A convention is only usable where there is one.** The first version of this file cleared
//! `npm`/`yarn`/`pnpm`/`bun` as "they shim into a directory their own installer wires up", and
//! CI falsified that for yarn on 2026-07-29 — an install that passed `list` and left the binary
//! unreachable. Those managers are asked instead, because the answer is decided by how the
//! manager itself was installed and no constant is right on two machines.

use crate::config::Config;
use crate::core::CommandExecutor;
use std::path::{Path, PathBuf};

/// Every directory that would answer "where did that executable go", most authoritative first.
///
/// Empty means "not this manager's problem": either it installs into a system directory, or it
/// installs no executables at all.
///
/// Three kinds of answer, and the difference matters:
///
/// - **Shall's own deploy directory** for the artifact backends. It is `[bin_dir]` and nothing
///   else, so the sentence this file prints cannot name a directory the deploy did not use.
/// - **The tool's own answer**, for the managers whose global directory is decided by how they
///   were installed, or by which switch is selected. `yarn global bin` reads `…\scoop\apps\yarn\current\global\bin` on the
///   developer's box and `%LOCALAPPDATA%\Yarn\bin` on a clean runner; a constant would be wrong
///   on one of them, and was.
/// - **The ecosystem's convention**, for the managers that have one, with the environment
///   variable first: a user who set `GOBIN` or `GEM_HOME` has already answered this question and
///   a hard-coded `~/go/bin` would contradict them.
///
/// More than one entry means the answer is genuinely ambiguous (npm's prefix), and any one of
/// them being on `PATH` is enough to stay quiet — a warning that fires on a machine that is
/// fine is how the real one gets ignored.
pub async fn user_bin_dirs(backend: &str, cfg: &Config, exec: &CommandExecutor) -> Vec<PathBuf> {
    if deploys_through_shall(backend) {
        return vec![cfg.bin_dir.clone()];
    }
    if let Some(dirs) = asks_the_tool(backend, exec).await {
        return dirs;
    }
    if backend == "go" {
        // One implementation of "where does `go install` put it", shared with the backend that
        // has to find the binary afterwards. Two readings of `GOPATH` is two answers.
        return crate::backends::go::install_bin_dir(exec)
            .await
            .into_iter()
            .collect();
    }
    conventional_bin_dir(backend).into_iter().collect()
}

/// The backends whose executables Shall itself deploys, into `[bin_dir]`.
fn deploys_through_shall(backend: &str) -> bool {
    matches!(backend, "github" | "web" | "appimage" | "shim")
}

/// The managers that know where their own global directory is, and are the only ones who do.
///
/// A tool that cannot answer — not installed, or a version without the subcommand (yarn berry
/// dropped `global`) — produces no candidates and therefore no warning. Claiming a directory
/// nobody confirmed is how a warning starts being wrong.
async fn asks_the_tool(backend: &str, exec: &CommandExecutor) -> Option<Vec<PathBuf>> {
    let (prog, args): (&str, &[&str]) = match backend {
        "yarn" => ("yarn", &["global", "bin"]),
        "pnpm" => ("pnpm", &["bin", "-g"]),
        "bun" => ("bun", &["pm", "bin", "-g"]),
        "npm" => ("npm", &["prefix", "-g"]),
        // opam's directory belongs to the current switch, so there is no constant to write:
        // `/root/.opam/default/bin` here, something else on a machine with another switch
        // selected, and nothing at all without one. Measured in the `tools` image 2026-07-29,
        // where `opam install -y ocamlfind` succeeded and `ocamlfind` was on nobody's PATH —
        // opam expects `eval $(opam env)`, which a declaration cannot do for you.
        "opam" => ("opam", &["var", "bin"]),
        _ => return None,
    };
    let out = exec.run_output(prog, args, false).await.ok()?;
    let dir = first_absolute_path(&out)?;
    Some(npm_prefix_candidates(backend, dir))
}

/// `npm prefix -g` prints the prefix, not the bin directory: executables land in `<prefix>`
/// on Windows and `<prefix>/bin` everywhere else. Both are candidates, so the platform rule
/// being wrong somewhere cannot produce a warning about a machine that is fine.
fn npm_prefix_candidates(backend: &str, dir: PathBuf) -> Vec<PathBuf> {
    if backend != "npm" {
        return vec![dir];
    }
    if cfg!(windows) {
        vec![dir.clone(), dir.join("bin")]
    } else {
        vec![dir.join("bin"), dir]
    }
}

/// The first line of a tool's output that is an absolute path.
///
/// Line-by-line rather than "trim the whole thing", because `yarn` prefixes its answer with
/// `warning …` lines often enough that reading the first line would report a warning as a
/// directory.
fn first_absolute_path(out: &str) -> Option<PathBuf> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .find(|p| p.is_absolute())
}

/// Where a manager puts the executables it installs, for the managers whose answer is a
/// convention rather than a question you can ask.
pub fn conventional_bin_dir(backend: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let env_path = |k: &str| {
        std::env::var(k)
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };

    Some(match backend {
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
pub fn is_on_path(dir: &Path) -> bool {
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
fn how_to_add(dir: &Path) -> String {
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
pub async fn unreachable_warning(
    backend: &str,
    cfg: &Config,
    exec: &CommandExecutor,
) -> Option<String> {
    let dirs = user_bin_dirs(backend, cfg, exec).await;
    if dirs.iter().any(|d| is_on_path(d)) {
        return None;
    }
    let dir = dirs.into_iter().next()?;
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
    use crate::core::executor::{DryRunOutput, MockExecutor};
    use dashmap::DashMap;
    use std::sync::Arc;

    #[cfg(windows)]
    const SANDBOX: &str = r"C:\shall-test-sandbox";
    #[cfg(not(windows))]
    const SANDBOX: &str = "/shall-test-sandbox";

    /// An executor that answers only what a test told it to. Anything else reads back empty,
    /// which is the same shape as a manager that is not installed.
    fn mocked(answers: &[(&str, &str)]) -> (CommandExecutor, Config) {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        for (cmd, out) in answers {
            mock.set_response(
                cmd,
                Ok(DryRunOutput {
                    stdout: out.as_bytes().to_vec(),
                    stderr: vec![],
                }
                .into()),
            );
        }
        let exec = CommandExecutor::with_layer(false, false, mock, vfs, Arc::new(DashMap::new()));
        (exec, Config::sandboxed(Path::new(SANDBOX)))
    }

    /// The reported case. `pub` installs to `~/.pub-cache/bin`, and the whole finding is that
    /// nothing mentioned it.
    #[test]
    fn a_per_user_ecosystem_names_its_bin_dir() {
        let dir = conventional_bin_dir("pub").expect("pub installs executables under $HOME");
        assert!(
            dir.ends_with("bin"),
            "a bin dir that is not a bin dir: {}",
            dir.display()
        );
    }

    /// The family, not the finding. Every ecosystem that installs into `$HOME` and leaves the
    /// PATH to you must answer, or it is the next `pub`.
    #[tokio::test]
    async fn every_per_user_ecosystem_answers() {
        let (exec, cfg) = mocked(&[]);
        for be in [
            "pub", "nimble", "go", "cargo", "gem", "luarocks", "mix", "stack", "krew", "pipx",
            "composer",
        ] {
            assert!(
                !user_bin_dirs(be, &cfg, &exec).await.is_empty(),
                "{be} installs into a user directory and this table does not know where"
            );
        }
    }

    /// The directory Shall deploys into is a user directory like any other, and it is the one
    /// Shall is answerable for. CI, 2026-07-29: `github installed sharkdp/fd for real`,
    /// `github: list shows fd`, `github: fd is on PATH` FAILED — `~/.local/bin` is not on a
    /// clean Windows runner's PATH and nothing said so.
    ///
    /// Asserted against the *config's* directory, not merely "some directory": the three
    /// backends built `~/.local/bin` from `dirs::home_dir()` themselves until this change, so a
    /// warning could name a directory the deploy never used, and a sandboxed run deployed into
    /// the developer's real home.
    #[tokio::test]
    async fn the_directory_shall_itself_deploys_into_answers() {
        let (exec, cfg) = mocked(&[]);
        for be in ["github", "web", "appimage", "shim"] {
            assert_eq!(
                user_bin_dirs(be, &cfg, &exec).await,
                vec![cfg.bin_dir.clone()],
                "{be} deploys through Shall's own bin dir and this does not say so"
            );
        }
    }

    /// `yarn` was cleared as "shims into a dir its own installer wires up". CI falsified that
    /// on 2026-07-29: `yarn: cowsay is on PATH` FAILED on a clean Windows runner. The answer
    /// cannot be a constant either — measured the same day, `yarn global bin` reads
    /// `…\scoop\apps\yarn\current\global\bin` on the developer's box and `%LOCALAPPDATA%\Yarn\bin`
    /// on the runner — so it is asked.
    #[tokio::test]
    async fn the_node_family_answers_from_the_tool() {
        let dir = Path::new(SANDBOX).join("yarn-global");
        let (exec, cfg) = mocked(&[("yarn global bin", &dir.display().to_string())]);
        assert_eq!(user_bin_dirs("yarn", &cfg, &exec).await, vec![dir]);
    }

    /// A tool answers with a path and sometimes with a paragraph first. Reading the first line
    /// would report `warning …` as a directory.
    #[test]
    fn a_warning_line_is_not_a_directory() {
        let dir = PathBuf::from(SANDBOX).join("yarn-global");
        let noisy = format!(
            "warning package.json: No license field\n{}\n",
            dir.display()
        );
        assert_eq!(first_absolute_path(&noisy), Some(dir));
        assert_eq!(first_absolute_path("warning: nothing to say\n"), None);
    }

    /// And the control: a tool that cannot answer produces no claim. yarn berry has no `global`
    /// subcommand, and a machine may not have yarn at all — inventing a directory there is how
    /// a warning starts naming somewhere the files are not.
    /// opam's answer is the current switch's, so it cannot be a constant either — and it is
    /// the one this family was extended for: measured in the `tools` image, `opam install -y
    /// ocamlfind` succeeded and `ocamlfind` was on nobody's PATH, because opam expects
    /// `eval $(opam env)` and a declaration cannot do that for you.
    #[tokio::test]
    async fn opam_answers_for_the_switch_that_is_selected() {
        let dir = Path::new(SANDBOX).join(".opam").join("default").join("bin");
        let (exec, cfg) = mocked(&[("opam var bin", &dir.display().to_string())]);
        assert_eq!(user_bin_dirs("opam", &cfg, &exec).await, vec![dir]);
    }

    #[tokio::test]
    async fn a_tool_that_cannot_answer_is_not_guessed_at() {
        let (exec, cfg) = mocked(&[]);
        for be in ["yarn", "pnpm", "bun", "npm", "opam"] {
            assert!(
                user_bin_dirs(be, &cfg, &exec).await.is_empty(),
                "{be} did not answer and something answered for it"
            );
            assert!(unreachable_warning(be, &cfg, &exec).await.is_none());
        }
    }

    /// npm prints a prefix, and where the executables sit under it is a platform rule. Both
    /// readings count, so being wrong about the rule cannot warn a user whose machine is fine.
    #[test]
    fn npms_prefix_offers_both_readings() {
        let prefix = PathBuf::from(SANDBOX).join("npm-prefix");
        let candidates = npm_prefix_candidates("npm", prefix.clone());
        assert!(
            candidates.contains(&prefix),
            "the prefix itself is a candidate: {candidates:?}"
        );
        assert!(
            candidates.contains(&prefix.join("bin")),
            "`<prefix>/bin` is a candidate: {candidates:?}"
        );
        // Every other manager answers with the bin directory itself; there is nothing to widen.
        assert_eq!(npm_prefix_candidates("yarn", prefix.clone()), vec![prefix]);
    }

    /// And the other half: a system manager must NOT warn. `/usr/bin` is on every PATH, so a
    /// warning about it is noise on every install, and noise is how a real warning gets
    /// ignored.
    #[tokio::test]
    async fn a_system_manager_has_nothing_to_warn_about() {
        let (exec, cfg) = mocked(&[]);
        for be in [
            "apt", "dnf", "pacman", "apk", "brew", "scoop", "winget", "choco",
        ] {
            assert!(
                user_bin_dirs(be, &cfg, &exec).await.is_empty(),
                "{be} installs into a system directory; warning about its PATH is noise"
            );
            assert!(unreachable_warning(be, &cfg, &exec).await.is_none());
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
    #[tokio::test]
    async fn the_warning_names_the_directory_and_the_line_to_add() {
        let (exec, cfg) = mocked(&[]);
        // Pick whichever of these is genuinely unreachable on the machine running the test;
        // asserting on a fixed one makes this pass or fail on the host's PATH rather than on
        // the code.
        let mut found = None;
        for b in ["pub", "composer", "mix", "krew", "luarocks", "nimble"] {
            if let Some(m) = unreachable_warning(b, &cfg, &exec).await {
                found = Some((b, m));
                break;
            }
        }
        let Some((be, msg)) = found else {
            // Every one of them is on PATH here. Nothing to assert, and nothing wrong.
            return;
        };
        let dir = conventional_bin_dir(be).unwrap();
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
