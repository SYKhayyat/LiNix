//! GRADER, 2026-07-28 — RED. The security refusals exit 1, and the refusal hook never hears them.
//!
//! `readme.md` publishes a four-code contract "so a script can branch on them": 0 converged,
//! 1 failed, 2 differences, 3 refused. E25 found one refusal returning 1 instead of 3 and it was
//! fixed for `purge-undeclared`. The family was not swept.
//!
//! Measured against the release binary:
//!
//!     $ shall install 'web:http://example.com/tool.tar.gz' -y
//!     Error: Validation error: refusing to download … over plain HTTP
//!     EXIT=1                                            <-- contract says 3
//!
//!     $ shall install github:sharkdp/fd -y              # ~/.local/bin/fd.exe already exists
//!     Error: Validation error: refusing to deploy `fd.exe`: … Shall did not create it.
//!     EXIT=1                                            <-- contract says 3
//!
//!     $ shall reset </dev/null
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
//! 1. A script branching on the documented table reads "Shall refused to download over plain
//!    HTTP" as "Shall crashed", and cannot tell it from a network failure.
//! 2. `src/main.rs:185` says, of the `Error::Refused` arm: *"`on_guard_refusal` (XIII.13) fires
//!    here and nowhere else: this is the one point every refusal in the program passes through,
//!    so no command can be added that refuses without the hook hearing about it."* **That is
//!    false for all eight sites above.** A user who wires `on_guard_refusal` to be told when
//!    Shall refuses something is told about a mass removal and is *not* told when Shall refuses
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
        "Shall refused (its own word) and exited {code}; readme.md defines 3 as refused \
         and 1 as failed, so a script cannot tell this from a network error.\n\
         `reset` returns 3 from the same binary.\n{out}"
    );
}

/// The half of G-10 that the exit code is only a symptom of: **does the hook actually fire?**
///
/// `src/main.rs:185` promises `on_guard_refusal` fires for every refusal in the program. Someone
/// wires that hook precisely so they are told when Shall refuses something — and until now they
/// were told about a mass package removal and *not* about a refused plain-HTTP download, an
/// unverified binary, an unprotected secret or an unapproved hook. Silent where it matters most.
///
/// Nothing tested it in either direction. This runs the security refusal with a real approved
/// hook attached and asserts the hook ran, with a control proving the harness could tell.
#[test]
fn a_security_refusal_fires_the_refusal_hook() {
    let dir = fixture("refusal-hook");
    let cfg = dir.join("config");
    let marker = dir.join("fired.txt");

    std::fs::create_dir_all(cfg.join("hooks")).unwrap();
    let script = if cfg!(windows) {
        format!("'fired' | Out-File -FilePath '{}'\n", marker.display())
    } else {
        format!("echo fired > '{}'\n", marker.display())
    };
    std::fs::write(cfg.join("hooks").join("on_guard_refusal"), &script).unwrap();

    // II.12: an unapproved hook does not run, so approving it is part of the setup and not
    // part of what is under test. Without this the assertion below would pass for the wrong
    // reason on a tree where the hook fired perfectly.
    let (out, code) = run(&dir, &["lock"]);
    assert_eq!(code, 0, "the fixture's own `lock` failed:\n{out}");
    assert!(
        out.contains("event hook") || out.contains("hook(s)"),
        "`lock` did not report approving the hook, so this test would prove nothing:\n{out}"
    );

    assert!(
        !marker.exists(),
        "the marker existed before anything refused; the test cannot tell a fire from a leftover"
    );

    let (out, code) = run(
        &dir,
        &["install", "web:http://example.com/tool.tar.gz", "-y"],
    );
    assert_eq!(code, 3, "the refusal did not reach the Refused arm:\n{out}");

    assert!(
        marker.exists(),
        "`on_guard_refusal` did not fire for a security refusal.\n\
         src/main.rs:185 says it fires for every refusal in the program — a user who wired this \
         hook to be told when Shall refuses is told about a mass removal and not about a \
         refused plain-HTTP download.\n{out}"
    );
}

/// The comment at src/main.rs:185 claims every refusal passes through the `Error::Refused` arm.
///
/// Checked from the code, because a claim that quantifies over paths is verified by enumerating
/// the paths and never by reading the sentence.
///
/// **The first draft of this test was wrong in the direction that matters**, and the correction
/// is kept here because it is the same mistake the test exists to catch. It looked eight lines
/// above each "refusing to" for `Error::Refused` *in the same file*, and reported five sites
/// that were already correct: `model/firewall.rs`, `model/health.rs` and `model/rehearsal.rs`
/// hold the message and `app/apply/firewall.rs:99`, `app/sync/mod.rs:386` and
/// `verbs/setup.rs:208`/`:218` hold the `Error::Refused` that wraps it. A window that stops at
/// the file boundary cannot see a two-file split, so it scored the split as an offence.
///
/// The fix is not an exemption list. A message builder is followed to **every** one of its call
/// sites, and each has to wrap it — which is strictly stronger than the original, because a
/// builder whose second caller forgets the wrap now fails where before it was invisible.
/// The name of the `fn` a line sits inside, and whether that fn hands back a message rather
/// than an error — i.e. is a builder whose *caller* decides the error type.
///
/// `Option<String>` counts as well as `String`: `health::refusal_if_unrevertable` returns
/// one, and it is the layer that decides *whether* to refuse while `cannot_revert_refusal`
/// decides *what to say*. Two builders in a row is a normal shape, so the follow below has
/// to be transitive or it stops one hop short and reports a correct site.
///
/// At module scope rather than nested in the test, so the oracle can drive it. Nested, the
/// only way to check it was to read it — which is how this file shipped a self-test that
/// asserted a string literal contained a substring it visibly contained.
fn enclosing_builder(lines: &[String], at: usize) -> Option<String> {
    for i in (0..=at).rev() {
        let t = lines[i].trim_start();
        if let Some(rest) = t.strip_prefix("pub fn ").or_else(|| t.strip_prefix("fn ")) {
            let name = rest.split(['(', '<']).next()?.trim().to_string();
            // The signature can wrap; look at the few lines that carry the return type.
            let sig = lines[i..(i + 8).min(lines.len())].join(" ");
            return if sig.contains("-> String") || sig.contains("-> Option<String>") {
                Some(name)
            } else {
                None
            };
        }
    }
    None
}

/// Every place `name(` is called, outside its own definition and outside test modules,
/// as `(index into sources, 0-based line, wrapped in Error::Refused)`.
///
/// The index rather than a rendered `file:line`: an earlier draft looked the caller's file
/// back up by basename to ask whether *it* was a builder, and basenames collide across a
/// tree with fourteen `mod.rs` files. It found the wrong file and indexed past its end.
fn call_sites(
    sources: &[(std::path::PathBuf, Vec<String>)],
    name: &str,
) -> Vec<(usize, usize, bool)> {
    let needle = format!("{}(", name);
    let mut out = Vec::new();
    for (fi, (_, lines)) in sources.iter().enumerate() {
        let mut in_tests = false;
        for (i, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                in_tests = true;
            }
            if in_tests {
                continue;
            }
            let t = line.trim_start();
            if t.starts_with("//") || t.starts_with("pub fn ") || t.starts_with("fn ") {
                continue;
            }
            if !t.contains(&needle) {
                continue;
            }
            // Above *and* below: the wrap is above for `Err(Error::Refused(build(..)))`
            // and below for `match build(..) { Some(m) => Err(Error::Refused(m)) }`, which
            // is how `sync/mod.rs:381` reads. A window that only looked up scored that
            // second shape as unwrapped.
            let from = i.saturating_sub(3);
            let to = (i + 6).min(lines.len() - 1);
            let wrapped = lines[from..=to].join("\n").contains("Error::Refused");
            out.push((fi, i, wrapped));
        }
    }
    out
}

/// The vocabulary, and it is a list — which is the shape this repo distrusts, so the list is
/// widened when a member is found rather than assumed complete. G-8 found the second entry:
/// `bundle.rs`'s restore said "it refuses unless you pass --force", returned `Error::Other`,
/// exited 1, and the round-2 sweep of this exact class could not see it because it matched
/// only the first phrasing.
///
/// `refuses to` is deliberately NOT here: it is the phrasing Shall uses about somebody ELSE
/// refusing — "Windows Task Scheduler refuses to register one otherwise" is an
/// `Error::Permission` and correctly so, and `shall protected`'s heading is "what Shall
/// refuses to remove". Both were measured as offenders when the phrase was included, and both
/// are right as they are.
const REFUSAL_VOCABULARY: &[&str] = &["refusing to", "Refusing to", "refuses unless"];

/// Does this line claim to be refusing something?
///
/// A refusal is RETURNED, never printed: `shall protected`'s heading says "what Shall refuses
/// to remove" and is not itself a refusal. Comments and assertions about a refusal are not
/// refusals either.
fn says_it_is_refusing(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("//") || t.contains("assert") {
        return false;
    }
    if ["println!", "print!", "info!", "warn!", "eprintln!"]
        .iter()
        .any(|m| t.starts_with(m))
    {
        return false;
    }
    REFUSAL_VOCABULARY.iter().any(|v| line.contains(v))
}

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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();

    let sources: Vec<(std::path::PathBuf, Vec<String>)> = files
        .iter()
        .map(|p| {
            let body = std::fs::read_to_string(p).unwrap_or_default();
            (p.clone(), body.lines().map(|l| l.to_string()).collect())
        })
        .collect();

    let mut offenders = Vec::new();
    let mut found = 0usize;

    for (path, lines) in &sources {
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            if !says_it_is_refusing(line) {
                continue;
            }
            found += 1;
            let file = path.file_name().unwrap_or_default().to_string_lossy();
            let from = i.saturating_sub(8);
            let nearby = lines[from..=i].join("\n");
            // `Unattended::Refuse` is the second way to build one, and it is a real one:
            // `core::prompt::confirm` is the only thing that reads it and turns it into
            // `Error::Refused` — asserted by `a_refusing_prompt_with_nobody_there_refuses_by_name`
            // in that module, which matches on the variant rather than on the message.
            if nearby.contains("Error::Refused") || nearby.contains("Unattended::Refuse") {
                continue;
            }

            // Not refused here — it may be a builder whose callers refuse. Follow it, through
            // as many builder hops as the code actually has.
            let Some(builder) = enclosing_builder(lines, i) else {
                offenders.push(format!(
                    "{}:{}  {}",
                    file,
                    i + 1,
                    t.chars().take(72).collect::<String>()
                ));
                continue;
            };

            let mut frontier = vec![builder.clone()];
            let mut seen: std::collections::BTreeSet<String> = frontier.iter().cloned().collect();
            let mut unwrapped = Vec::new();
            let mut any_site = false;

            while let Some(name) = frontier.pop() {
                for (fi, at, wrapped) in call_sites(&sources, &name) {
                    any_site = true;
                    if wrapped {
                        continue;
                    }
                    let (caller_path, caller_lines) = &sources[fi];
                    let where_ = format!(
                        "{}:{}",
                        caller_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        at + 1
                    );
                    // The caller did not wrap — but if the caller is itself a builder, the
                    // decision is one layer further out. Chase it rather than accuse it.
                    match enclosing_builder(caller_lines, at) {
                        Some(outer) if seen.insert(outer.clone()) => frontier.push(outer),
                        Some(_) => {}
                        None => unwrapped.push(format!(
                            "{}:{}  via `{}`, whose call site {} does not wrap it in \
                             Error::Refused",
                            file,
                            i + 1,
                            name,
                            where_
                        )),
                    }
                }
            }

            if !any_site {
                offenders.push(format!(
                    "{}:{}  `{}` builds a refusal message and nothing calls it",
                    file,
                    i + 1,
                    builder
                ));
            }
            offenders.extend(unwrapped);
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

/// Test the oracle before trusting it: the scan above must reject a builder whose caller drops
/// the wrap, or "no offenders" would mean "the scan stopped looking".
///
/// GRADE §"Do not test your own oracle by assuming it works": 24 of 24 READY backends answered
/// `list`, which was true and meaningless because a backend that does not exist answers the
/// same way. So this feeds the check something it must reject.
fn planted(name: &str, body: &str) -> (std::path::PathBuf, Vec<String>) {
    (
        std::path::PathBuf::from(name),
        body.lines().map(|l| l.to_string()).collect(),
    )
}

#[test]
fn the_refusal_scan_rejects_an_unwrapped_builder() {
    // This test used to assert that three string literals declared four lines above it
    // contained substrings they visibly contained — under a doc comment quoting the standard
    // "do not test your own oracle by assuming it works". It never called the scan. Gutting
    // `call_sites` or `enclosing_builder` left it green, which is the failure it warns about,
    // committed inside the warning.

    // --- `says_it_is_refusing`: the vocabulary, and the four shapes that are not refusals.
    assert!(says_it_is_refusing(
        "        return Err(Error::Refused(format!(\"refusing to remove {}\", n)));"
    ));
    for phrase in REFUSAL_VOCABULARY {
        assert!(
            says_it_is_refusing(&format!("    let m = format!(\"{phrase} do the thing\");")),
            "the scan stopped recognising `{phrase}`"
        );
    }
    for (label, line) in [
        (
            "a comment about a refusal is not a refusal",
            "        // refusing to remove it would be wrong here",
        ),
        (
            "a test asserting on a refusal is not a refusal",
            "        assert!(msg.contains(\"refusing to\"));",
        ),
        (
            "a refusal is returned, never printed",
            "        println!(\"refusing to remove {}\", n);",
        ),
        (
            "`refuses to` describes somebody else refusing — Task Scheduler, not Shall",
            "        let m = \"Windows Task Scheduler refuses to register one otherwise\";",
        ),
    ] {
        assert!(!says_it_is_refusing(line), "{label}: {line}");
    }

    // --- `enclosing_builder`: a fn returning a message is one; a fn returning an error is not.
    let builder: Vec<String> = "pub fn lockout_refusal(port: u16) -> String {\n    \
         format!(\"refusing to apply the firewall change: port {}\", port)\n}"
        .lines()
        .map(|l| l.to_string())
        .collect();
    assert_eq!(
        enclosing_builder(&builder, 1).as_deref(),
        Some("lockout_refusal"),
        "a fn handing back a String is a builder whose caller decides the error type"
    );

    let optional: Vec<String> = "fn refusal_if_unrevertable(x: &X) -> Option<String> {\n    \
                                 Some(format!(\"refusing to revert {}\", x))\n}"
        .lines()
        .map(|l| l.to_string())
        .collect();
    assert_eq!(
        enclosing_builder(&optional, 1).as_deref(),
        Some("refusal_if_unrevertable"),
        "Option<String> is a builder too — health.rs is why that branch exists"
    );

    let terminal: Vec<String> = "fn apply(x: &X) -> Result<()> {\n    \
                                 Err(Error::Other(format!(\"refusing to apply {}\", x)))\n}"
        .lines()
        .map(|l| l.to_string())
        .collect();
    assert_eq!(
        enclosing_builder(&terminal, 1),
        None,
        "a fn that returns the error itself is the accused, not a builder to chase"
    );

    // --- `call_sites`: the wrap above, the wrap below, the missing wrap, and the exclusions.
    let sources = vec![
        planted(
            "good_above.rs",
            "    return Err(Error::Refused(lockout_refusal(port)));",
        ),
        planted(
            "good_below.rs",
            "    match lockout_refusal(port) {\n        \
             Some(m) => return Err(Error::Refused(m)),\n        None => {}\n    }",
        ),
        planted(
            "bad.rs",
            "    return Err(Error::Validation(lockout_refusal(port)));",
        ),
    ];
    let sites = call_sites(&sources, "lockout_refusal");
    assert_eq!(
        sites.len(),
        3,
        "the scan must find all three call sites, not {sites:?}"
    );
    assert!(sites[0].2, "a wrap on the same line counts as wrapped");
    assert!(
        sites[1].2,
        "a wrap three lines BELOW counts — sync/mod.rs:381 reads this way, and a window that \
         only looked up scored it as unwrapped"
    );
    assert!(
        !sites[2].2,
        "Error::Validation is not Error::Refused, and this is the whole point of the scan"
    );

    let excluded = vec![
        planted("def.rs", "pub fn lockout_refusal(port: u16) -> String {"),
        planted(
            "comment.rs",
            "    // lockout_refusal(port) used to live here",
        ),
        planted(
            "tests.rs",
            "#[cfg(test)]\nmod t {\n    fn x() { lockout_refusal(1); }\n}",
        ),
    ];
    assert!(
        call_sites(&excluded, "lockout_refusal").is_empty(),
        "a definition, a comment and a test module are not call sites: {:?}",
        call_sites(&excluded, "lockout_refusal")
    );

    // And the real tree must still have builders being followed, or the branch that fixed the
    // false positives is dead code and this test guards nothing.
    let health =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/model/health.rs"))
            .expect("model/health.rs exists");
    assert!(
        health.contains("-> String") && health.contains("refusing to start"),
        "the two-file builder split this branch exists for is gone; re-check the scan still \
         needs it before deleting the branch"
    );
}
