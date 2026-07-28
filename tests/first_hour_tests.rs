//! What a new user meets in their first five minutes, asserted against the real binary.
//!
//! None of this was covered. CI never runs a first command on an empty machine, so the paths
//! only a human takes were the ones with the damage in them: a first `sync` that explained how
//! to hand-write a file rather than naming the command that writes it, and an `init --help`
//! promising a starter module that `init` did not create.

use std::path::Path;
use std::process::Command;

struct Fresh {
    dir: std::path::PathBuf,
}

impl Fresh {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
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

    fn config(&self, rel: &str) -> std::path::PathBuf {
        self.dir.join("config").join(rel)
    }
}

/// The first command a new user runs, on a machine with no config.
///
/// It explained the `priority` file format and asked them to write it by hand, and never
/// mentioned `linix init` — which exists to do exactly that, detects the managers actually on
/// the machine, and is one word. Describing a format well is not the same as naming the
/// command that fills it in.
#[test]
fn the_first_sync_names_the_command_that_fixes_it() {
    let fresh = Fresh::new("first-sync");
    let (out, _) = fresh.run(&["sync"]);
    assert!(
        out.contains("linix init"),
        "the first thing a new user sees does not mention the command that fixes it:\n{out}"
    );
}

/// `init --help` says it scaffolds "a starter module". It created `modules/` empty.
///
/// Either the promise or the behaviour had to go; the promise is the useful one, so `init`
/// now keeps it.
#[test]
fn init_creates_everything_its_help_promises() {
    let fresh = Fresh::new("init-promises");
    let (out, code) = fresh.run(&["init"]);
    assert_eq!(code, 0, "init failed:\n{out}");

    for promised in ["priority", "active"] {
        assert!(
            fresh.config(promised).exists(),
            "`init --help` promises {promised} and did not write it:\n{out}"
        );
    }
    assert!(fresh.config("profiles").is_dir());

    let modules = fresh.config("modules");
    let files: Vec<_> = std::fs::read_dir(&modules)
        .expect("modules dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        !files.is_empty(),
        "`init --help` promises a starter module and `modules/` is empty"
    );

    // A module nothing reaches is inert (II.3), so the promise is only kept if the starter is
    // actually wired into the active profile.
    let main = std::fs::read_to_string(fresh.config("profiles/Main")).expect("Main");
    let starter = files[0].trim_end_matches(".txt").to_string();
    assert!(
        main.contains(&format!("use {starter}")),
        "the starter module exists but no active profile reaches it, so it does nothing:\n{main}"
    );
}

/// Whatever `init` writes, the next command has to be able to read it. A scaffold that does
/// not resolve is worse than no scaffold: it fails on the second command instead of the first.
#[test]
fn what_init_writes_parses() {
    let fresh = Fresh::new("init-parses");
    let (out, code) = fresh.run(&["init"]);
    assert_eq!(code, 0, "{out}");
    let (out, code) = fresh.run(&["check", "config"]);
    assert!(
        code == 0 || code == 2,
        "`check config` could not read what `init` just wrote (exit {code}):\n{out}"
    );
}
