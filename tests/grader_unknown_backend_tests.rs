//! GRADER, 2026-07-28 — RED. `list --backend <typo>` succeeds silently.
//!
//! Principle I is "fail loud, never silent", and `install` obeys it perfectly:
//!
//!     $ linix install nosuchbackend:foo -y
//!     Error: Configuration error: <argument>: `nosuchbackend` is not a backend LiNix uses
//!       add `nosuchbackend` to your `priority` file, or check the spelling.
//!     rc=1
//!
//! `list` is asked the same question and answers nothing at all:
//!
//!     $ linix list -b nosuchbackend ; echo $?
//!     0
//!     $ linix list -b aptt ; echo $?          # a typo
//!     0
//!     $ linix list -b APT  ; echo $?          # wrong case
//!     0
//!     $ linix list -b ''   ; echo $?          # empty
//!     0
//!
//! Zero rows, exit 0, no message. Which is byte-identical to what a real backend with nothing
//! installed prints — so a user who mistypes `--backend` is told, in the program's own voice,
//! that the manager is empty.
//!
//! It also quietly disarms a check the readiness rubric asks for. "Every `[READY]` backend can
//! answer `list`" is §8.1's A bar, and running `linix list -b <b>` over all 24 READY backends on
//! this host returns rc=0 for every one — but 13 of them return zero rows, which this test shows
//! is exactly what a name that does not exist returns. The assertion cannot distinguish a
//! backend that answered from one that was never consulted. That is the same shape as the
//! harness assertion that deleted its own evidence, in the check written to replace it.
//!
//! `install`'s message is the model to copy; it names the file and the fix.

use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_linix"))
        .args(args)
        .env("LINIX_CONFIG_DIR", dir.join("config"))
        .env("LINIX_DATA_DIR", dir.join("data"))
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

fn fixture(name: &str) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (out, code) = run(&dir, &["init"]);
    assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
    dir
}

#[test]
fn listing_a_backend_that_does_not_exist_says_so() {
    let dir = fixture("unknown-backend-list");

    // The control: `install` is asked the same question and refuses loudly, so "LiNix cannot
    // tell" is not the explanation.
    let (ctl, ctl_rc) = run(&dir, &["install", "nosuchbackend:foo", "-y"]);
    assert_ne!(
        ctl_rc, 0,
        "the control failed: `install nosuchbackend:foo` was expected to refuse, got:\n{ctl}"
    );

    let mut silent = Vec::new();
    for name in ["nosuchbackend", "aptt", "APT", ""] {
        let (out, rc) = run(&dir, &["list", "--backend", name]);
        let says_so = out.to_lowercase().contains("not a backend")
            || out.to_lowercase().contains("unknown backend")
            || out.to_lowercase().contains("check the spelling");
        if rc == 0 && !says_so {
            silent.push(format!(
                "--backend {name:?} -> rc 0, output {:?}",
                out.trim()
            ));
        }
    }

    assert!(
        silent.is_empty(),
        "`list` accepted names that are not backends and reported success:\n  {}\n\n\
         `install` refuses the same name with:\n  {}\n\n\
         Zero rows and exit 0 is what a real, empty backend prints, so a typo in --backend is \
         indistinguishable from `that manager has nothing installed`.",
        silent.join("\n  "),
        ctl.lines().next().unwrap_or("").trim(),
    );
}
