//! GRADER, 2026-07-28 — RED. `Retryability::Transient` is a claim nothing tests.
//!
//! `Transient` means "a second attempt could differ". The container harness understands this
//! and proves it the only way it can be proved — `classify_install` retries once and calls the
//! failure a defect if it repeats. The product decides the same question by matching a string,
//! and no test ever asks whether the string was right.
//!
//! Measured on this host, with `lua 5.5.0` / `luarocks 3.13.0` from scoop:
//!
//!     $ curl -o /dev/null -w '%{http_code}' https://luarocks.org/manifest-5.5
//!     200
//!     $ luarocks install luafilesystem      # three times, identical each time
//!     Warning: Failed searching manifest: Failed downloading https://luarocks.org/manifest-5.5
//!     Error: No results matching query were found for Lua 5.5.
//!
//! The manifest is reachable; luarocks' own downloader is what fails, because the `wget` first
//! on this PATH is a scoop shim that does not take the flags luarocks passes. `exit_policy.rs`
//! lists `"failed downloading"` and `"failed searching manifest"` as transient markers, so
//! Shall calls this Transient, keeps the declaration, and tells the user `sync` will try it
//! again. It will fail identically forever.
//!
//! The policy's own doc comment names this exact cause — "a machine whose only problem is that
//! the `wget` on its PATH is BusyBox's applet, which rejects the GNU flags luarocks passes" —
//! and then classifies it as the network anyway. That is READINESS §3.4's defect (a real
//! failure reported as ecosystem variance) moved from the harness into the product.
//!
//! This test states the property the classification is making: if Shall says a failure is
//! worth retrying, retrying it must be capable of a different answer.

use std::process::Command;

fn shall(args: &[&str], cfg: &std::path::Path) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(args)
        .env("SHALL_CONFIG_DIR", cfg.join("config"))
        .env("SHALL_DATA_DIR", cfg.join("data"))
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

#[test]
fn a_failure_called_transient_can_actually_differ_on_a_second_attempt() {
    if which::which("luarocks").is_err() {
        eprintln!("luarocks is not on this host; nothing to measure");
        return;
    }

    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("transient-claim");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (out, code) = shall(&["init"], &dir);
    assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");

    let (first, rc1) = shall(&["install", "luarocks:luafilesystem", "-y"], &dir);
    if rc1 == 0 {
        eprintln!("luarocks works on this host; there is no failure to classify");
        return;
    }

    // Shall keeps the declaration and says `sync` will retry exactly when it judged the
    // failure retryable. That sentence is the claim under test.
    let called_transient = first.contains("is still declared") && first.contains("try it again");
    if !called_transient {
        // Not "nothing to falsify, move on". Shall declining to call this transient is the
        // fix, and a test that merely returns here would go green just as readily on a build
        // that had stopped saying anything at all. So assert the honest alternative: it must
        // still keep the declaration (the cause is a broken `wget` on the PATH, which is
        // fixable — deleting the line would be the wrong cure) AND say that retrying will not
        // help, rather than silently dropping the subject.
        assert!(
            first.contains("is still declared"),
            "the failure was not called transient, but the declaration was not accounted for \
             either — a line written by `install` must always be reported as kept or \
             withdrawn:\n{}",
            tail(&first)
        );
        assert!(
            first.contains("repeated on every retry"),
            "Shall stopped promising a retry, but did not say why. `Transient` was falsified \
             by evidence the program already had — it retried and got the same answer — and \
             the user needs that sentence, not silence:\n{}",
            tail(&first)
        );
        return;
    }

    let (second, rc2) = shall(&["install", "luarocks:luafilesystem", "-y"], &dir);

    assert!(
        rc2 == 0,
        "Shall classified this failure as worth retrying and told the user so, then the retry \
         failed identically (rc {rc1} then {rc2}).\n\
         `Transient` is a claim that a second attempt could differ; nothing in the product \
         tests it, and here it is false.\n\
         --- first attempt (tail) ---\n{}\n--- second attempt (tail) ---\n{}",
        tail(&first),
        tail(&second),
    );
}

fn tail(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(6)..].join("\n")
}
