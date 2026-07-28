//! GRADER, 2026-07-28 — RED. The security refusals exit 1, and the refusal hook never hears them.
//!
//! `readme.md:708` publishes a four-code contract "so a script can branch on them": 0 converged,
//! 1 failed, 2 differences, 3 refused. E25 found one refusal returning 1 instead of 3 and it was
//! fixed for `purge-unmanaged`. The family was not swept.
//!
//! Measured against the release binary:
//!
//!     $ linix install 'web:http://example.com/tool.tar.gz' -y
//!     Error: Validation error: refusing to download … over plain HTTP
//!     EXIT=1                                            <-- contract says 3
//!
//!     $ linix install github:sharkdp/fd -y              # ~/.local/bin/fd.exe already exists
//!     Error: Validation error: refusing to deploy `fd.exe`: … LiNix did not create it.
//!     EXIT=1                                            <-- contract says 3
//!
//!     $ linix reset </dev/null
//!     EXIT=3                                            <-- correct, for contrast
//!
//! Enumerated from the code rather than from the two that were reported, every site whose own
//! message says "refusing to …" and which is NOT built as `Error::Refused`:
//!
//!     src/core/download.rs:46    plain HTTP                      (SEC2)
//!     src/core/download.rs:69    unverified, no @sha256          (SEC2)
//!     src/core/executor.rs:396   a secret nothing protects       (T5)
//!     src/backends/link.rs:68    decrypt into the git repo       (T2)
//!     src/app/hooks.rs:55        unapproved hooks                (II.12)
//!     src/app/shim_manager.rs:98 deploy over a foreign file      (SEC1)
//!     src/utils/file.rs:174      deploy over a foreign file      (SEC1)
//!     src/app/apply/dotfiles.rs:67 files outside $HOME           (SEC3)
//!
//! That list is the entire SEC/T series. **The refusals that exit 3 are the ones about removing
//! packages; the refusals that exit 1 are the ones about security.**
//!
//! Two consequences, and the second is worse than the exit code:
//!
//! 1. A script branching on the documented table reads "LiNix refused to download over plain
//!    HTTP" as "LiNix crashed", and cannot tell it from a network failure.
//! 2. `src/main.rs:185` says, of the `Error::Refused` arm: *"`on_guard_refusal` (XIII.13) fires
//!    here and nowhere else: this is the one point every refusal in the program passes through,
//!    so no command can be added that refuses without the hook hearing about it."* **That is
//!    false for all eight sites above.** A user who wires `on_guard_refusal` to be told when
//!    LiNix refuses something is told about a mass removal and is *not* told when LiNix refuses
//!    an unverified download, an unprotected secret, or an unapproved hook. It is silent exactly
//!    where it matters most — and it is a comment asserting something about paths it never
//!    enumerated, which is the failure mode `spec/history.md` records as costing more than the
//!    rest combined.
//!
//! The harness feels this too: `classify_install` keys its `refused` outcome on rc=3, so a
//! correct refusal arrives as rc=1 and is scored `a defect, not ecosystem variance`. READINESS
//! §3.4 complained that a correct refusal was laundered into a soft pass; it is now laundered
//! into a false hard failure. The harness still cannot see the truth, because the product does
//! not tell it.

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

/// The documented contract: a refusal is 3, whatever it refused.
#[test]
fn a_security_refusal_exits_with_the_documented_refusal_code() {
    let dir = fixture("refusal-exit-http");

    // The control: a refusal that IS `Error::Refused` returns 3 here, so "this binary never
    // returns 3" is not the explanation.
    let (_, control) = run(&dir, &["reset"]);
    assert_eq!(
        control, 3,
        "the control failed: `reset` with no terminal should refuse with 3"
    );

    let (out, code) = run(
        &dir,
        &["install", "web:http://example.com/tool.tar.gz", "-y"],
    );
    assert!(
        out.to_lowercase().contains("refusing to download"),
        "the fixture did not reach the plain-HTTP refusal; got:\n{out}"
    );
    assert_eq!(
        code, 3,
        "LiNix refused (its own word) and exited {code}; readme.md:708 defines 3 as refused \
         and 1 as failed, so a script cannot tell this from a network error.\n\
         `reset` returns 3 from the same binary.\n{out}"
    );
}

/// The comment at src/main.rs:185 claims every refusal passes through the `Error::Refused` arm.
///
/// Checked from the code, because a claim that quantifies over paths is verified by enumerating
/// the paths and never by reading the sentence.
#[test]
fn every_site_that_says_it_is_refusing_is_built_as_a_refusal() {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files.sort();

    let mut offenders = Vec::new();
    let mut found = 0usize;

    for path in &files {
        let body = std::fs::read_to_string(path).unwrap_or_default();
        let lines: Vec<&str> = body.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            // The message, not the comment about it, and not a test asserting on it.
            if t.starts_with("//") || t.contains("assert") {
                continue;
            }
            if !(line.contains("refusing to") || line.contains("Refusing to")) {
                continue;
            }
            found += 1;
            let from = i.saturating_sub(8);
            let ctx = lines[from..=i].join("\n");
            if !ctx.contains("Error::Refused") {
                offenders.push(format!(
                    "{}:{}  {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    i + 1,
                    t.chars().take(72).collect::<String>()
                ));
            }
        }
    }

    // Without a floor this passes on a tree where the scan matched nothing.
    assert!(
        found >= 10,
        "the refusal scan found only {found} sites; it has stopped matching the code it audits"
    );

    assert!(
        offenders.is_empty(),
        "these say they are refusing but are not `Error::Refused`, so they exit 1 instead of 3 \
         and the `on_guard_refusal` hook never fires for them — which src/main.rs:185 promises \
         it does for every refusal in the program:\n  {}",
        offenders.join("\n  ")
    );
}
