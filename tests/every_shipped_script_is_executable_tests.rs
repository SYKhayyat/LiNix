//! Every script this repository ships is executable **in the index**, which is the only copy a
//! clone receives.
//!
//! **The bug this exists to stop, measured.** `scripts/nix-validate.sh` was committed from
//! Windows, where the filesystem carries no executable bit and git records `100644`. Every local
//! run passed, because the four-step chain and the local invocation both spell it
//! `sh scripts/nix-validate.sh`. CI spells it `./scripts/nix-validate.sh --self-test`, and on
//! `a5d5517` that job died with
//!
//! ```text
//! ./scripts/nix-validate.sh: Permission denied
//! ##[error]Process completed with exit code 126.
//! ```
//!
//! — `main` red on a mode bit, on a commit whose own diff was documentation.
//!
//! **And the reported symptom was one of fourteen.** Every `.sh` in the tree was `100644`: the
//! container entrypoints, `install.sh`, `unix-check.sh`, `release-check.sh`, and the three whose
//! own header comments tell the reader to run them as `./scripts/<name>.sh`. Only the workflow
//! line that used `./` failed, so thirteen siblings were live and invisible — a clone on Linux
//! following the documentation gets `Permission denied` from all of them.
//!
//! This asks git, not the filesystem. On Windows `core.fileMode` is off and the working tree
//! reports whatever it likes; the index is what ships.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `(path, mode)` for everything git tracks. `None` when git cannot answer — the working tree is
/// not a substitute here, because its disagreement with the index is the entire subject.
fn tracked_modes() -> Option<BTreeMap<String, String>> {
    let out = Command::new("git")
        .args(["ls-files", "-s"])
        .current_dir(repo())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let listed: BTreeMap<String, String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            // `<mode> <object> <stage>\t<path>`
            let (meta, path) = line.split_once('\t')?;
            let mode = meta.split_whitespace().next()?;
            Some((path.trim().to_string(), mode.to_string()))
        })
        .collect();
    (!listed.is_empty()).then_some(listed)
}

/// A file whose first two bytes are `#!` is claiming to be run directly. That claim is the test:
/// it does not depend on an extension, so a script added as `.bash`, `.py` or with no extension
/// at all is covered on the day it lands.
fn declares_a_shebang(rel: &str) -> bool {
    std::fs::read(repo().join(rel))
        .map(|bytes| bytes.starts_with(b"#!"))
        .unwrap_or(false)
}

#[test]
fn every_script_that_declares_a_shebang_is_executable_in_the_index() {
    let Some(tracked) = tracked_modes() else {
        eprintln!(
            "script modes: SKIPPED — `git ls-files -s` did not answer, and the working tree is \
             not a substitute for the index here. Nothing was checked."
        );
        return;
    };

    let scripts: Vec<(&String, &String)> = tracked
        .iter()
        .filter(|(path, _)| declares_a_shebang(path))
        .collect();

    // The self-test. A scan that found nothing would make the assertion below vacuous, which is
    // the failure this repository writes down most often.
    assert!(
        scripts.len() >= 10,
        "found {} shebang files among {} tracked paths; the scan is broken, not the tree",
        scripts.len(),
        tracked.len()
    );

    let not_executable: Vec<String> = scripts
        .iter()
        .filter(|(_, mode)| mode.as_str() != "100755")
        .map(|(path, mode)| format!("  {} is {}", path, mode))
        .collect();
    assert!(
        not_executable.is_empty(),
        "these scripts declare a shebang and are not executable in the index, so a clone \
         receives them unrunnable and running one as `./<path>` fails with `Permission denied` \
         (exit 126):\n{}\nFix with: git update-index --chmod=+x <path>",
        not_executable.join("\n")
    );
}

/// The other half, and the reason this is two tests: a mode bit nobody would ever use is noise
/// in a diff and a surprise in a review. Nothing but a script should be `100755`.
#[test]
fn nothing_that_is_not_a_script_is_marked_executable() {
    let Some(tracked) = tracked_modes() else {
        eprintln!("script modes: SKIPPED — `git ls-files -s` did not answer.");
        return;
    };
    let stray: Vec<&String> = tracked
        .iter()
        .filter(|(path, mode)| mode.as_str() == "100755" && !declares_a_shebang(path))
        .map(|(path, _)| path)
        .collect();
    assert!(
        stray.is_empty(),
        "these are executable in the index and are not scripts: {:?}",
        stray
    );
}
