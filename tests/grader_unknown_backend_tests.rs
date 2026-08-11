//! GRADER, 2026-07-28 — RED. `list --backend <typo>` succeeds silently.
//!
//! Principle I is "fail loud, never silent", and `install` obeys it perfectly:
//!
//!     $ shall install nosuchbackend:foo -y
//!     Error: Configuration error: <argument>: `nosuchbackend` is not a backend Shall uses
//!       add `nosuchbackend` to your `priority` file, or check the spelling.
//!     rc=1
//!
//! `list` is asked the same question and answers nothing at all:
//!
//!     $ shall list -b nosuchbackend ; echo $?
//!     0
//!     $ shall list -b aptt ; echo $?          # a typo
//!     0
//!     $ shall list -b APT  ; echo $?          # wrong case
//!     0
//!     $ shall list -b ''   ; echo $?          # empty
//!     0
//!
//! Zero rows, exit 0, no message. Which is byte-identical to what a real backend with nothing
//! installed prints — so a user who mistypes `--backend` is told, in the program's own voice,
//! that the manager is empty.
//!
//! It also quietly disarms a check the readiness rubric asks for. "Every `[READY]` backend can
//! answer `list`" is §8.1's A bar, and running `shall list -b <b>` over all 24 READY backends on
//! this host returns rc=0 for every one — but 13 of them return zero rows, which this test shows
//! is exactly what a name that does not exist returns. The assertion cannot distinguish a
//! backend that answered from one that was never consulted. That is the same shape as the
//! harness assertion that deleted its own evidence, in the check written to replace it.
//!
//! `install`'s message is the model to copy; it names the file and the fix.

use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(args)
        .env("SHALL_CONFIG_DIR", dir.join("config"))
        .env("SHALL_DATA_DIR", dir.join("data"))
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

    // The control: `install` is asked the same question and refuses loudly, so "Shall cannot
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

/// The family, not the finding. `list` was the one with a reproduction attached; every verb
/// that takes a backend name answered the same question and had to be checked.
///
/// `repo` is the interesting one: it took its backend positionally and answered `Backend not
/// found` — true, but naming neither the file to edit nor the spelling to check, which is E18's
/// shape (one condition, two message families) on a different pair of verbs.
#[test]
fn every_verb_that_takes_a_backend_name_refuses_one_that_does_not_exist() {
    let dir = fixture("unknown-backend-family");

    let cases: &[(&str, &[&str])] = &[
        ("list", &["list", "--backend", "nosuchbackend"]),
        ("upgrade", &["upgrade", "--backend", "nosuchbackend"]),
        ("rebuild", &["rebuild", "--backend", "nosuchbackend"]),
        ("repo list", &["repo", "list", "--backend", "nosuchbackend"]),
    ];

    let mut silent = Vec::new();
    for (name, argv) in cases {
        let (out, rc) = run(&dir, argv);
        let says_so = out.to_lowercase().contains("not a backend")
            || out.to_lowercase().contains("check the spelling");
        if rc == 0 || !says_so {
            silent.push(format!(
                "`shall {}` -> rc {rc}, output {:?}",
                argv.join(" "),
                out.trim()
            ));
        }
        let _ = name;
    }

    assert!(
        silent.is_empty(),
        "a verb taking a backend name accepted one that does not exist:\n  {}",
        silent.join("\n  ")
    );
}

/// A real backend that is not installed here is a different fact from a typo, and until now
/// both produced the same silence.
///
/// This is the half that is easy to skip: making the typo loud is worth little if "apt, on
/// Windows" still prints nothing and exits 0, because the user cannot tell which of the two
/// they are looking at — and the first is their mistake while the second is not.
#[test]
fn a_real_backend_that_cannot_run_here_says_that_instead() {
    let dir = fixture("unknown-backend-absent");

    // A backend registered on every platform Shall builds for, and installed on very few
    // machines — so it is registered here and cannot run here. `apt` would NOT do: the
    // registry is platform-scoped, so on Windows `apt` is not registered at all and is a
    // genuine typo. That distinction is the finding, so the fixture has to respect it.
    let absent = "flatpak";
    let (out, rc) = run(&dir, &["list", "--backend", absent]);

    assert_eq!(
        rc, 0,
        "`{absent}` is a real backend, so naming it is not an error — it is a fact about this \
         machine:\n{out}"
    );
    assert!(
        out.to_lowercase().contains("not installed on this machine"),
        "`shall list -b {absent}` said nothing about why it had nothing to report. Silence \
         here is indistinguishable from an empty manager, which is the whole finding.\n{out}"
    );
    assert!(
        !out.to_lowercase().contains("not a backend"),
        "`{absent}` IS a backend; refusing it as a typo would trade one wrong answer for \
         another.\n{out}"
    );
}

/// Test the oracle. §8.1's A bar is "every `[READY]` backend can answer `list`", and the
/// grader measured 24 of 24 passing before discovering the check could not fail: a backend
/// that does not exist answered `list` exactly the way a real empty one did.
///
/// So before that measurement means anything again, feed the check something it must reject.
#[test]
fn the_can_answer_list_check_can_actually_fail() {
    let dir = fixture("unknown-backend-oracle");

    let (real_out, real_rc) = run(&dir, &["list", "--backend", "web"]);
    let (fake_out, fake_rc) = run(&dir, &["list", "--backend", "webb"]);

    assert_ne!(
        (real_rc, real_out.trim().to_string()),
        (fake_rc, fake_out.trim().to_string()),
        "a backend that exists and one that does not are still indistinguishable through \
         `list`, so 'every READY backend can answer list' remains an assertion that cannot \
         fail.\n  web  -> rc {real_rc}: {:?}\n  webb -> rc {fake_rc}: {:?}",
        real_out.trim(),
        fake_out.trim()
    );
    assert_eq!(
        real_rc, 0,
        "the control failed: `web` is a real backend:\n{real_out}"
    );
    assert_ne!(fake_rc, 0, "the typo was accepted:\n{fake_out}");
}
