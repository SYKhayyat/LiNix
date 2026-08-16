//! The repo's rules about its own scripts, in the language the rules are checked in everywhere
//! else.
//!
//! Six of `harness-logic-test.sh`'s predicates never ran a script or entered a container: they
//! read `ci.yml`, the release scripts, the Dockerfiles and the harnesses as *text* and asserted
//! properties of them. Written in shell, each one paid for that twice — once in `grep | sed |
//! awk` pipelines whose failure modes are silent (`grep -c` printing `0` and exiting 1 is what
//! made the mutation gate report success on total collapse), and once in *when* they run: at the
//! end of a release script or in CI, rather than in `cargo test` beside the other twenty-seven
//! gates that do exactly this kind of reading.
//!
//! **Five of the six are here. The sixth already had a Rust successor** — see the note above
//! `every_script_is_run_by_something…`, which is the whole argument of this file arriving as a
//! near-miss on its own author.
//!
//! What stayed in shell is the half that lifts function bodies out of the harnesses and drives
//! them. That is not portable to Rust and should not be: it tests the actual bytes CI runs, in
//! the actual interpreter, which is the only technique that answers the question it asks.
//!
//! **Every scan here carries a floor.** The defect these replace is a check that stopped
//! matching the thing it audits and went on reporting `ok` — II.23. A scan whose input list came
//! back empty must fail, not pass.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Every file directly under `dir` whose name ends in one of `exts`.
fn files_in(dir: &str, exts: &[&str]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(root().join(dir))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && exts.iter().any(|x| {
                            p.file_name()
                                .map(|n| n.to_string_lossy().ends_with(x))
                                .unwrap_or(false)
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn base(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into()
}

// ---------------------------------------------------------------------------

// Gate parity is NOT here, and finding that out is the reason this note exists. The shell
// predicate had a Rust successor already — `grader_gate_parity_tests::
// every_gate_ci_runs_is_run_locally_with_the_same_target` — written when the shell one was
// caught comparing basenames, and it is the stronger of the two: it keys on the whole
// invocation, script plus the arguments that decide what is measured. Porting the shell
// version would have made three implementations of one question, which is the defect this
// whole file is a response to. `grade6_gate_parity_sees_whole_jobs_tests` covers the other
// half: a CI job whose steps run a command directly, naming no script at all.

/// **No gate script may sit in the repo with nothing running it** (G-5).
///
/// `grader-red-tests.sh` was 131 lines of source-text greps run by no CI job and neither release
/// script, whose first check could never pass because it reproduced the bug it tested. A
/// permanently-red file nobody runs is worse than no file, and it is invisible precisely because
/// nothing runs it.
///
/// `docker/integration/` is in the sweep, and that is not incidental: the rule iterated
/// `scripts/*.sh` only, so the repo's one real orphan sat one directory outside the rule written
/// to catch orphans.
#[test]
fn every_script_is_run_by_something_or_is_declared_not_to_be_a_gate() {
    /// Not gates, with what each one is instead. A name here is a claim, not a silence.
    const NOT_GATES: &[(&str, &str)] = &[
        ("install.sh", "what a user pipes from the web"),
        ("install.ps1", "what a user pipes from the web"),
        ("release-check.sh", "the top of the chain; a person runs it"),
        (
            "release-check.ps1",
            "the top of the chain; a person runs it",
        ),
        (
            "measure-batching.sh",
            "a measuring instrument, run by hand against a real container when a batching \
             claim needs evidence",
        ),
    ];

    let mut scripts = files_in("scripts", &[".sh", ".ps1"]);
    scripts.extend(files_in("docker/integration", &[".sh"]));
    assert!(
        scripts.len() >= 8,
        "the sweep found {} scripts; it is not reading the tree",
        scripts.len()
    );

    // Everything that could name a script: the workflows, the scripts themselves, the container
    // plumbing. A hand-written search set is the defect this gate looks for — `stall-snapshot.ps1`
    // is called from `integration-windows.sh` and was being reported as an orphan.
    let mut haystack: Vec<(String, String)> = Vec::new();
    for dir in [".github/workflows", "scripts", "docker"] {
        let mut stack = vec![root().join(dir)];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(body) = std::fs::read_to_string(&p) {
                    haystack.push((base(&p), body));
                }
            }
        }
    }
    assert!(
        haystack.len() >= 15,
        "only {} files to search for references; the search set has collapsed",
        haystack.len()
    );

    let mut orphans: Vec<String> = Vec::new();
    for s in &scripts {
        let name = base(s);
        if NOT_GATES.iter().any(|(n, _)| *n == name) {
            continue;
        }
        // Its own file naming itself is not a reference.
        let referenced = haystack
            .iter()
            .any(|(other, body)| *other != name && body.contains(&name));
        if !referenced {
            orphans.push(name);
        }
    }

    assert!(
        orphans.is_empty(),
        "these scripts are run by nothing — wire them in, or name them in NOT_GATES with what \
         they are instead:\n  {}",
        orphans.join("\n  ")
    );

    // The exemption list is itself audited: a name that no longer exists is a claim about
    // nothing, and it is how an exemption outlives the thing it excused.
    let present: BTreeSet<String> = scripts.iter().map(|p| base(p)).collect();
    let stale: Vec<&str> = NOT_GATES
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !present.contains(*n))
        .collect();
    assert!(
        stale.is_empty(),
        "NOT_GATES names scripts that are gone: {stale:?}"
    );
}

/// **A harness function must be defined ABOVE the first place the script calls it.**
///
/// Shell reads top to bottom: a function called before its `f() {` has been evaluated is not a
/// quiet no-op, it is `command not found` on stderr — and the harness keeps going. Measured on
/// CI, 2026-07-29: three PATH helpers sat beside `assert_binary_gone` and were called from
/// section 5, so one check reported `rc=127` and one vanished entirely.
///
/// This is ShellCheck's `SC2218`, and shellcheck does now run — in CI's `shell` job and in
/// `release-check.sh`. It is kept because it runs here, in `cargo test`, on a developer machine
/// with no shellcheck installed, which is where the harness is being edited.
///
/// **Calls inside another function body do not count**: a body runs after the whole file is
/// read, so `classify_install` calling `refused` is correct however they are ordered. A checker
/// that cannot tell the difference reports three false positives and gets switched off.
#[test]
fn every_harness_function_is_defined_before_it_is_called() {
    let harnesses = [
        "docker/integration/run-in-container.sh",
        "scripts/integration-windows.sh",
    ];

    let mut offenders: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for h in harnesses {
        let body = read(h);
        let lines: Vec<&str> = body.lines().collect();

        // (name, 1-based definition line)
        let defs: Vec<(String, usize)> = lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| {
                let name = l.strip_suffix('{')?.trim_end().strip_suffix("()")?;
                (!name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
                .then(|| (name.to_string(), i + 1))
            })
            .collect();
        assert!(
            defs.len() >= 5,
            "{h}: found {} function definitions; the scan has stopped matching the file",
            defs.len()
        );
        checked += defs.len();

        for (name, def_line) in &defs {
            let mut inside = false;
            for (i, raw) in lines.iter().enumerate() {
                let opens = raw
                    .strip_suffix('{')
                    .map(|s| s.trim_end().ends_with("()"))
                    .unwrap_or(false);
                if opens {
                    inside = true;
                    continue;
                }
                if inside {
                    if *raw == "}" {
                        inside = false;
                    }
                    continue;
                }
                // A name inside a description is not a call. Every description in these
                // harnesses is double-quoted; the single-quoted text is `sh -c` bodies, which
                // name no functions.
                let mut line = raw.split('#').next().unwrap_or("").to_string();
                while let (Some(a), Some(b)) = (line.find('"'), line.rfind('"')) {
                    if a >= b {
                        break;
                    }
                    line.replace_range(a..=b, "");
                }
                if !mentions(&line, name) {
                    continue;
                }
                if i + 1 < *def_line {
                    offenders.push(format!(
                        "{h}: `{name}` called at line {}, defined at line {def_line}",
                        i + 1
                    ));
                }
                break;
            }
        }
    }

    assert!(
        checked >= 20,
        "only {checked} functions across both harnesses; the scan is not reading them"
    );
    assert!(
        offenders.is_empty(),
        "these calls run before the function exists — `command not found`, and the harness \
         carries on and reports a verdict:\n  {}",
        offenders.join("\n  ")
    );
}

/// `name` as a whole word, where a shell word may not contain `-`, alphanumerics or `_`.
fn mentions(line: &str, name: &str) -> bool {
    let boundary = |c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let bytes = line.as_bytes();
    line.match_indices(name).any(|(i, _)| {
        let before = i == 0 || boundary(line[..i].chars().next_back().unwrap_or(' '));
        let after = i + name.len() >= bytes.len()
            || boundary(line[i + name.len()..].chars().next().unwrap_or(' '));
        before && after
    })
}

/// **Every shell script this repo runs must have LF endings, in the working tree.**
///
/// Not a style rule. `run.sh` bind-mounts the host's copy of the harness into the container,
/// where `/bin/sh` is dash; dash reads `set -u<CR>`, aborts with `set: Illegal option -`, and no
/// check runs. `.gitattributes` pins `*.sh text eol=lf` and the committed blobs are LF, so CI is
/// unaffected and the gate never fired — `eol=lf` governs what checkout writes, not what an
/// editor writes afterwards. On 2026-07-29 four scripts in a working tree were CRLF and the
/// entire local container gate was silently unavailable (N-6).
///
/// Reading bytes rather than shelling out to `grep`: MSYS grep opens a file in text mode and
/// normalises CRLF before matching, so the shell version of this was blind on the one platform
/// where the bug occurs, and needed a self-test of its own detector to know that.
#[test]
fn every_shell_script_the_repo_runs_has_lf_endings() {
    let mut files: Vec<PathBuf> = files_in("scripts", &[".sh"]);
    files.extend(files_in("docker/integration", &[".sh"]));
    // Every file in `.githooks`, matched on the empty suffix because git fixes a hook's filename
    // and none of them carry an extension for a glob to catch. A CRLF hook aborts on `set -eu<CR>`
    // and so refuses every commit in the clone it is installed in.
    files.extend(files_in(".githooks", &[""]));

    // Plus every file bind-mounted into a container, read off the mounts themselves.
    // `scripts/lifecycle-floor.txt` is data, not a script, so no glob covered it — and it is
    // parsed in-container with `awk '{print $2}'`, which over a CRLF line yields `7<CR>`, so
    // `[ -lt ]` errors on a non-integer and the shell takes the branch that reports the ratchet
    // satisfied.
    for src in [".github/workflows/ci.yml", "docker/integration/run.sh"] {
        for line in read(src).lines() {
            let mut rest = line;
            while let Some(i) = rest.find("$PWD/") {
                rest = &rest[i + 5..];
                let end = rest
                    .find(|c: char| c == ':' || c == '"' || c.is_whitespace())
                    .unwrap_or(rest.len());
                let candidate = root().join(&rest[..end]);
                // **A file, and a file made of text.** The sweep collects any `$PWD/`-rooted
                // path the workflow names, and `ci.yml` names one that is not a script at all:
                // `SHALL="$PWD/target/release/shall.exe"`. On a machine that has done a release
                // build that path exists, the scan reads it, an executable contains `\r` in the
                // ordinary course of being an executable, and the gate reported the release
                // binary as a CRLF shell script. A check that fails for a reason unrelated to
                // its own sentence proves nothing when it passes.
                if candidate.is_file() && std::fs::read_to_string(&candidate).is_ok() {
                    files.push(candidate);
                }
                rest = &rest[end..];
            }
        }
    }
    files.sort();
    files.dedup();

    assert!(
        files.len() >= 10,
        "the CRLF sweep found {} files; it is not reading the tree",
        files.len()
    );

    let crlf: Vec<String> = files
        .iter()
        .filter(|p| std::fs::read(p).is_ok_and(|b| b.contains(&b'\r')))
        .map(|p| base(p))
        .collect();

    assert!(
        crlf.is_empty(),
        "CRLF line endings in the working tree — dash aborts on `set -u\\r` before any check \
         runs, so the container gate reports nothing at all:\n  {}\n\nfix: `git add \
         --renormalize . && git checkout -- .`",
        crlf.join("\n  ")
    );
}

/// **The pre-commit hook must run CI's formatting gate, spelled the way CI spells it** (E3).
///
/// The gate this hook exists for was fixed twice and run neither time. Formatting is the one CI
/// gate a change containing no logic can break, and it did: renaming `nexus::` to `shall::`
/// re-sorted two import groups past `petgraph`, and `cargo fmt --check` failed on main and on
/// every open dependabot PR branched from it — nine red runs, one reordered line each.
///
/// **Both directions matter.** A hook that runs something weaker than CI passes commits CI
/// rejects, which is the asymmetry E3/E4 found in the release scripts. A hook stricter than CI
/// refuses commits CI would take, and a gate that refuses good work is a gate people learn to
/// pass `--no-verify` to. So the command is read out of `ci.yml` rather than written down here.
#[test]
fn the_pre_commit_hook_runs_the_formatting_gate_ci_runs() {
    let ci = read(".github/workflows/ci.yml");
    let fmt_gate = ci
        .lines()
        .skip_while(|l| !l.contains("name: Check formatting"))
        .find_map(|l| l.trim().strip_prefix("run: "))
        .map(str::trim)
        .expect(
            "this test's premise is gone: `ci.yml` has no `Check formatting` step with a `run:` \
             command. Either the gate moved, in which case point this scan at where it went, or \
             the gate was deleted, in which case the hook has nothing to mirror",
        );
    assert!(
        fmt_gate.contains("cargo fmt"),
        "`ci.yml`'s `Check formatting` step runs `{fmt_gate}`, which is not a `cargo fmt` \
         invocation; this scan has stopped matching the gate it reads"
    );

    let hook = read(".githooks/pre-commit");

    // Named in an error message is not run. The hook prints the command it failed on, so a scan
    // for the bare string passes over a hook whose actual invocation has been deleted.
    let invocations = hook
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && !l.starts_with("echo") && !l.starts_with("printf"))
        .filter(|l| l.contains(fmt_gate))
        .count();
    assert!(
        invocations > 0,
        "`.githooks/pre-commit` never runs `{fmt_gate}` — the command `ci.yml` gates formatting \
         with — outside a comment or an `echo`. A hook weaker than CI is how a green local run \
         becomes a red push"
    );

    assert!(
        hook.contains("exit 1"),
        "`.githooks/pre-commit` runs the formatting gate but never exits non-zero, so git takes \
         the commit either way. A hook that only warns is a hook that gates nothing"
    );
}

/// **Every container leg that runs the harness must also mount the ratchet's floor file.**
///
/// `.dockerignore` excludes `scripts/` deliberately — editing a host script must not bust the
/// image's cargo cache — so `scripts/lifecycle-floor.txt` is in no image and reaches a container
/// only by being mounted. It was not, on any leg: the ratchet was in force on the Windows sweep,
/// which has the least coverage, and absent from the four distro legs and the `tools` image,
/// which have the most. Every one of those runs was green (N-5).
#[test]
fn every_container_leg_that_runs_the_harness_mounts_the_lifecycle_floor() {
    let ci = read(".github/workflows/ci.yml");
    let harness = ci
        .matches("run-in-container.sh:/src/docker/integration/run-in-container.sh")
        .count();
    let floor = ci
        .matches("lifecycle-floor.txt:/src/scripts/lifecycle-floor.txt")
        .count();
    assert!(
        harness > 0,
        "no container leg mounts the harness; this check has stopped matching ci.yml"
    );
    assert_eq!(
        harness, floor,
        "{harness} container leg(s) mount the harness, {floor} mount the floor. A leg without \
         the floor runs the ratchet's else branch, which measures nothing."
    );
}

/// **Every integration image declares its own identity, and declares it correctly.**
///
/// The ratchet keys its floor on the image, and `/etc/os-release` cannot supply that: `tools` is
/// built on Ubuntu, so it and the ubuntu image answered the same name and shared one record
/// while doing 25 and 7 real lifecycles. A Dockerfile that forgets the ENV silently rejoins
/// whatever distro it is based on, which is a collision rather than a new host class.
#[test]
fn every_integration_image_declares_its_own_identity() {
    let dockerfiles: Vec<PathBuf> = std::fs::read_dir(root().join("docker/integration"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| base(p).starts_with("Dockerfile."))
                .collect()
        })
        .unwrap_or_default();

    assert!(
        dockerfiles.len() >= 3,
        "found {} integration Dockerfiles; the scan is not reading the directory",
        dockerfiles.len()
    );

    let mut wrong: Vec<String> = Vec::new();
    for df in &dockerfiles {
        let want = base(df).trim_start_matches("Dockerfile.").to_string();
        let got = read(&format!("docker/integration/{}", base(df)))
            .lines()
            .filter_map(|l| l.trim().strip_prefix("ENV SHALL_IT_IMAGE="))
            .map(|v| v.trim().to_string())
            .next_back();
        if got.as_deref() != Some(want.as_str()) {
            wrong.push(format!(
                "Dockerfile.{want} declares {}",
                got.unwrap_or_else(|| "nothing".into())
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "image identity missing or wrong:\n  {}\n\nThe ratchet then files this image under its \
         base distro's record.",
        wrong.join("\n  ")
    );
}

/// **A workflow that does not parse fails the run, not a job** — so nothing in this repo could
/// see it (`S79`).
///
/// `S67` ended a step with a module filter, `--test suite pty_tests::`, and YAML read the
/// trailing colon as a mapping key. GitHub answered by refusing the whole file: no jobs, no
/// steps, no log, a red dot with a zero-second duration and the words *"likely failed because of
/// a workflow file issue"*. **Ten commits landed on top of it** — each of them reporting a local
/// build, test and clippy run as its verification, each of them correct about that and wrong
/// about CI, because a workflow that never starts produces no failing check to notice.
///
/// This is a text scan and says so: the repo has no YAML parser and is not acquiring one for a
/// gate. It checks the class the defect belongs to — a plain (unquoted) scalar that YAML will
/// re-read as a key, which is any value ending in `:` or containing `: `. That is not every way
/// to write invalid YAML; it is the way this repo has actually written it.
#[test]
fn every_workflow_value_that_yaml_would_read_as_a_key_is_quoted() {
    let workflows = files_in(".github/workflows", &[".yml", ".yaml"]);
    assert!(
        !workflows.is_empty(),
        "no workflow files found; the scan is not reading the directory"
    );

    /// The offending values in one file, as `line number: text`.
    fn offenders(body: &str) -> Vec<String> {
        let mut out = Vec::new();
        // A block scalar's body is shell, not YAML, and shell is full of colons. Everything
        // indented under `run: |` is skipped until the indentation returns.
        let mut block_indent: Option<usize> = None;
        for (i, line) in body.lines().enumerate() {
            let indent = line.len() - line.trim_start().len();
            if let Some(open) = block_indent {
                if line.trim().is_empty() || indent > open {
                    continue;
                }
                block_indent = None;
            }
            let trimmed = line.trim_start().trim_start_matches("- ").trim_start();
            if trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }
            let Some((key, value)) = trimmed.split_once(':') else {
                continue;
            };
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                continue;
            }
            let value = value.trim();
            if value.starts_with('|') || value.starts_with('>') {
                block_indent = Some(indent);
                continue;
            }
            // Quoted is the fix, so quoted is not a finding. `#` starts a comment, and a value
            // that is only a comment is an empty value.
            if value.is_empty()
                || value.starts_with('"')
                || value.starts_with('\'')
                || value.starts_with('#')
            {
                continue;
            }
            let value = value.split(" #").next().unwrap_or(value).trim();
            if value.ends_with(':') || value.contains(": ") {
                out.push(format!("{}: {}", i + 1, line.trim()));
            }
        }
        out
    }

    // The floor, and it is not decoration: this scan's whole failure mode is quietly matching
    // nothing. Fed the byte sequence that killed CI, it must object.
    let planted = offenders("jobs:\n  build:\n    steps:\n    - run: cargo test pty_tests::\n");
    assert_eq!(
        planted.len(),
        1,
        "the scan cannot see the defect it exists for: {planted:?}"
    );
    assert!(
        offenders(
            "    - run: \"cargo test pty_tests::\"\n    - if: matrix.os == 'ubuntu-latest'\n"
        )
        .is_empty(),
        "the scan objects to the fix, or to an ordinary conditional"
    );

    let mut found: Vec<String> = Vec::new();
    for w in &workflows {
        for o in offenders(&read(&format!(".github/workflows/{}", base(w)))) {
            found.push(format!("{}:{}", base(w), o));
        }
    }
    assert!(
        found.is_empty(),
        "these values end in a colon or contain `: ` unquoted, which YAML reads as a mapping \
         key and GitHub answers by refusing the entire file:\n  {}\n\nQuote the value.",
        found.join("\n  ")
    );
}

/// **Every target the release publishes is a target something builds.**
///
/// The build matrix declared four and produced **one**. A base `rust: [stable]` above the
/// `include:` gives the matrix exactly one combination, and GitHub merges an include entry into
/// an existing combination whenever it overwrites none of the base values — so all four rows
/// merged into that same job in turn and the last one, Windows, won. Three consecutive runs
/// produced a single `Build for x86_64-pc-windows-msvc` and nothing else: Linux and both Macs
/// were never compiled here at all, while the release job asserts four binaries in `dist/`.
///
/// It is the same shape as the four release assets that were all named `shall`, and it survived
/// the same way — by being a claim about a run nobody read. So the claim is checked: the targets
/// the release step names by hand and the targets the matrix builds are one list, and a matrix
/// that cannot expand to one job per row fails here rather than in six months at a tag.
#[test]
fn every_target_the_release_publishes_is_one_the_matrix_actually_builds() {
    let ci = read(".github/workflows/ci.yml");

    // The matrix rows. `- target:` appears only under an `include:` list; the container legs
    // use `distro:`, so there is nothing else to exclude.
    let built: std::collections::BTreeSet<String> = ci
        .lines()
        .map(str::trim)
        .filter_map(|l| l.strip_prefix("target: "))
        .map(|t| t.trim().to_string())
        .collect();
    assert!(
        built.len() >= 4,
        "the build matrix names {} target(s); it declared four when this was written: {built:?}",
        built.len()
    );

    // **And no base key above the rows**, which is the thing that collapsed them. A `rust:` (or
    // any other) list beside `include:` reintroduces exactly one combination for four rows to
    // overwrite each other in.
    // Line endings are not assumed: this file is checked out CRLF here and LF on the runners,
    // and a marker carrying a `\n` matches on exactly one of the two.
    let matrix_at = ci.find("      matrix:").expect("the build matrix");
    let matrix = &ci[matrix_at..];
    let matrix = &matrix[..matrix.find("    steps:").unwrap_or(matrix.len())];
    assert!(
        collapses_to_one_job(matrix).is_none(),
        "{}",
        collapses_to_one_job(matrix).unwrap_or_default()
    );

    // The targets the release step names by hand, which is the list that must agree.
    let published: std::collections::BTreeSet<String> = ci
        .lines()
        .filter(|l| l.contains("dist/shall-"))
        .flat_map(|l| {
            l.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
                .filter_map(|w| w.strip_prefix("shall-"))
                .map(|t| t.trim_end_matches(".exe").to_string())
                .collect::<Vec<_>>()
        })
        .filter(|t| t.contains('-'))
        .collect();

    let unbuilt: Vec<&String> = published.difference(&built).collect();
    assert!(
        unbuilt.is_empty(),
        "the release publishes {unbuilt:?}, and no matrix row builds them. Every one of these \
         is a binary somebody downloads for a platform CI never compiled for."
    );
}

/// **Every triple the installer asks for is a triple the release publishes.**
///
/// `install.sh` maps `uname` to a target and downloads `shall-<triple>`; when there is no asset
/// it falls back to building from source — 448 crates under fat LTO, on whatever hardware the
/// user has. That fallback is silent, so a triple the installer names and the release does not
/// publish is not a 404 anybody sees: it is a thirty-second promise that takes twenty minutes.
///
/// It has already happened once in the other direction — the matrix could not express
/// `aarch64-apple-darwin`, so every Mac sold since 2020 built from source — and the shape here
/// is identical. Linux on arm64 was the same gap until `v0.8.0`.
#[test]
fn every_triple_the_installer_downloads_is_one_the_release_publishes() {
    let ci = read(".github/workflows/ci.yml");
    let built: std::collections::BTreeSet<String> = ci
        .lines()
        .map(str::trim)
        .filter_map(|l| l.strip_prefix("target: "))
        .map(|t| t.trim().to_string())
        .collect();

    // The triples `target_triple()` can echo. Read from the script rather than restated, or
    // this test pins a list somebody edits the script out from under.
    let installer = read("scripts/install.sh");
    let asked: std::collections::BTreeSet<String> = installer
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split("echo ").nth(1))
        .map(|t| {
            t.split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches(';')
                .to_string()
        })
        .filter(|t| t.contains("-unknown-") || t.contains("-apple-") || t.contains("-pc-"))
        .collect();
    assert!(
        asked.len() >= 4,
        "the installer names {} triple(s); this scan has stopped matching install.sh: {asked:?}",
        asked.len()
    );

    let unpublished: Vec<&String> = asked.difference(&built).collect();
    assert!(
        unpublished.is_empty(),
        "`install.sh` downloads {unpublished:?} and no matrix row builds them, so every user on \
         that platform silently compiles from source instead."
    );
}

/// The **file name** half of the same contract, which the triple check above does not reach.
///
/// `every_triple_the_installer_downloads_is_one_the_release_publishes` proves the release builds
/// each platform the installer asks for. It says nothing about what the artifact is *called*,
/// and a release that publishes the right binary under the wrong name is a 404 for every user —
/// which `install.sh` answers by silently compiling 448 crates instead, the exact failure that
/// test exists to prevent, one step further along.
///
/// Three files hard-code the string `shall-<target>` and nothing joined them: the release job's
/// rename loop and its by-name assertion, `install.sh`'s URL, and `install.ps1`'s `$asset`.
/// **`install.ps1` was not covered at all** — its Windows asset is one literal filename, and the
/// scan above reads only `install.sh`.
#[test]
fn every_asset_name_the_installers_ask_for_is_one_the_release_writes() {
    let ci = read(".github/workflows/ci.yml");

    // The Windows asset is a single literal on both sides, so the two can be compared exactly.
    // Read from the script, not restated here, or this pins a name somebody edits out from
    // under it.
    let ps1 = read("scripts/install.ps1");
    let asset = ps1
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("$asset = '")?
                .strip_suffix('\'')
                .map(str::to_string)
        })
        .expect("`install.ps1` no longer assigns `$asset` as a single-quoted literal");
    assert!(
        ci.contains(&format!("dist/{asset}")),
        "`install.ps1` downloads `{asset}` and the release job never writes `dist/{asset}`, so \
         every Windows user gets a 404 and falls back to a source build. The release job's own \
         assertion cannot catch this: it checks the name it writes against itself."
    );

    // And the Unix half, by prefix. `install.sh` builds its URL as `shall-$triple`; the release
    // job writes `dist/shall-${target}`. Both are the string `shall-` and a triple, and the
    // triples already agree — so the prefix is the whole of what is left to check.
    let sh = read("scripts/install.sh");
    assert!(
        sh.contains("/shall-$triple\""),
        "`install.sh` no longer builds its URL as `shall-$triple`; this check has stopped \
         matching the script it is about"
    );
    assert!(
        ci.contains("dist/shall-${target}"),
        "the release job no longer writes `dist/shall-${{target}}`, and `install.sh` still asks \
         for `shall-<triple>`"
    );
}

/// Why a matrix block would expand to fewer jobs than it has rows, or `None` if it is sound.
///
/// A base key beside `include:` gives the matrix one combination, and GitHub merges an include
/// entry into an existing combination whenever it overwrites none of the base values — so every
/// row lands in that same job in turn and only the last survives. A row without its own copy of
/// the base key is the same defect written the other way round.
fn collapses_to_one_job(matrix: &str) -> Option<String> {
    let include_at = matrix.find("include:")?;
    let before = &matrix[..include_at];
    if before.contains(": [") {
        return Some(format!(
            "the build matrix has a base key above its `include:` rows:\n{before}\nThat makes \
             ONE combination, and every include row merges into it in turn — the last row wins \
             and the rest never run. Put the key on each row instead."
        ));
    }
    let rows = matrix.matches("- os:").count();
    let toolchains = matrix.matches("rust:").count();
    (rows != toolchains).then(|| {
        format!(
            "{rows} matrix row(s) and {toolchains} `rust:` key(s) — a row without one has to \
             borrow from a base key, and a base key is what collapsed this matrix"
        )
    })
}

/// Runner labels GitHub has withdrawn, and what to use instead.
///
/// **A job that asks for a label with no runner behind it does not fail — it queues.** For ever,
/// with no error, no log and no annotation: measured 2026-08-10, where `Build for
/// x86_64-apple-darwin` on `macos-13` sat 83 minutes without starting while the `macos-latest`
/// row beside it started in three seconds. `Create Release` needs every build, so the release
/// simply could not be cut, and the only visible symptom was a run that never finished.
///
/// Kept as data because the failure is silent by construction: nothing in a workflow run says
/// "that label is gone", so the only place it can be said is here.
const WITHDRAWN_RUNNERS: &[(&str, &str)] = &[
    (
        "macos-13",
        "the last Intel image, retired — build `x86_64-apple-darwin` on `macos-latest`, which \
         cross-compiles to it",
    ),
    ("macos-12", "retired — use `macos-latest`"),
    (
        "ubuntu-20.04",
        "retired — use `ubuntu-latest` or `ubuntu-22.04`",
    ),
    ("ubuntu-18.04", "retired — use `ubuntu-latest`"),
    ("windows-2019", "retired — use `windows-latest`"),
];

fn withdrawn_runners_named_in(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("- os: ").or_else(|| l.strip_prefix("os: ")))
        .map(|l| l.trim().trim_matches(|c| c == '\'' || c == '"').to_string())
        .filter(|label| WITHDRAWN_RUNNERS.iter().any(|(dead, _)| dead == label))
        .collect()
}

/// No job may ask for a runner that no longer exists.
#[test]
fn no_workflow_asks_for_a_runner_github_has_withdrawn() {
    let workflows = files_in(".github/workflows", &[".yml", ".yaml"]);
    assert!(
        !workflows.is_empty(),
        "no workflow files found; this scan has stopped matching the repo"
    );
    for w in &workflows {
        let body = read(&format!(".github/workflows/{}", base(w)));
        let dead = withdrawn_runners_named_in(&body);
        assert!(
            dead.is_empty(),
            "{} asks for {:?}, which GitHub no longer provides. Such a job queues for ever \
             instead of failing, and any job that needs it can never run. {}",
            base(w),
            dead,
            WITHDRAWN_RUNNERS
                .iter()
                .filter(|(l, _)| dead.iter().any(|d| d == l))
                .map(|(l, why)| format!("`{l}`: {why}"))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
}

/// **The predicate above, shown failing** — on the exact row that shipped, because a scan for
/// a *withdrawn* label has nothing to match in a healthy tree and would otherwise pass by
/// having found nothing at all.
#[test]
fn the_withdrawn_runner_scan_sees_the_row_that_queued_for_ever() {
    let shipped = "      matrix:\n        include:\n          - os: macos-13\n            \
                   target: x86_64-apple-darwin\n          - os: macos-latest\n            \
                   target: aarch64-apple-darwin\n";
    assert_eq!(
        withdrawn_runners_named_in(shipped),
        vec!["macos-13".to_string()],
        "the scan cannot see the row that blocked the release"
    );
    assert!(
        withdrawn_runners_named_in("          - os: macos-latest\n").is_empty(),
        "a live runner label was reported withdrawn"
    );
    // A label named only inside a comment is history, not a request for a runner. Three of the
    // comments in `ci.yml` explain why `macos-13` is gone, and a scan that could not tell the
    // difference would make writing that explanation impossible.
    assert!(
        withdrawn_runners_named_in("      # `macos-13` was the last Intel image\n").is_empty(),
        "a label mentioned in a comment was read as a job asking for it"
    );
}

/// **The predicate above, shown failing.** A scan that has never objected to anything is
/// indistinguishable from a clean tree, and three of this repo's gates once passed for exactly
/// that reason. So it is fed the shape CI actually shipped for three runs.
#[test]
fn the_matrix_scan_objects_to_the_shape_that_shipped() {
    let collapsed = r"      matrix:
        rust: [stable]
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - os: windows-latest
            target: x86_64-pc-windows-msvc
";
    let why = collapses_to_one_job(collapsed).expect(
        "the scan cannot see the defect it exists for - this is the exact matrix that built one          target out of four for three consecutive runs",
    );
    assert!(why.contains("base key"), "{why}");

    // A row missing its toolchain is the same defect the other way round.
    let uneven = r"      matrix:
        include:
          - os: ubuntu-latest
            target: a
            rust: stable
          - os: windows-latest
            target: b
";
    assert!(
        collapses_to_one_job(uneven).is_some(),
        "an uneven matrix passed"
    );

    // And the control, so a green run above is not explained by "it objects to everything".
    let sound = r"      matrix:
        include:
          - os: ubuntu-latest
            target: a
            rust: stable
          - os: windows-latest
            target: b
            rust: stable
";
    assert_eq!(collapses_to_one_job(sound), None);
}

/// **A `--test <name>` that names nothing is a job that fails in three seconds, every night.**
///
/// `Cargo.toml` declares exactly one test target — `suite`, with `autotests = false` — because
/// 101 auto-discovered targets each fat-LTO-linked against a 100k-line crate filled a 944 GB
/// disk. That conversion renamed every target to `suite` and updated the workflow step that
/// happened to be read at the time. It missed three others, in the nightly jobs, which have
/// invoked deleted targets ever since:
///
/// ```text
/// error: no test target named `argv_drift_tests` in default-run packages
/// ```
///
/// Five consecutive scheduled runs were red before anyone looked, and they were red instantly —
/// not one assertion in those jobs has been evaluated since the conversion. The upstream-drift
/// sweep, which is the only thing that notices a manager changing its flags, has been off.
///
/// Cargo already fails loudly on this; what it cannot do is fail *where somebody is reading*.
#[test]
fn every_test_target_ci_invokes_is_one_that_exists() {
    let manifest = read("Cargo.toml");
    let declared: BTreeSet<String> = manifest
        .split("[[test]]")
        .skip(1)
        .filter_map(|block| {
            block
                .lines()
                .find_map(|l| l.trim().strip_prefix("name = "))
                .map(|n| n.trim().trim_matches('"').to_string())
        })
        .collect();
    assert!(
        declared.contains("suite"),
        "no `[[test]]` target named `suite` in Cargo.toml — this scan has stopped matching it: \
         {declared:?}"
    );

    let mut invoked: Vec<(String, String)> = Vec::new();
    for w in files_in(".github/workflows", &[".yml", ".yaml"]) {
        let body = read(&format!(".github/workflows/{}", base(&w)));
        for name in test_targets_named(&body) {
            invoked.push((base(&w), name));
        }
    }
    assert!(
        !invoked.is_empty(),
        "no `--test` invocation found in any workflow; this scan is reading nothing"
    );

    let missing: Vec<String> = invoked
        .iter()
        .filter(|(_, n)| !declared.contains(n))
        .map(|(f, n)| format!("{f}: --test {n}"))
        .collect();
    assert!(
        missing.is_empty(),
        "these CI steps name a cargo test target that does not exist, so the job fails in \
         seconds having evaluated nothing:\n  {}\n\nThe only target is `suite`; a file inside \
         it is selected as `--test suite <module>::`.",
        missing.join("\n  ")
    );
}

/// Every `--test <name>` in a workflow body, in order.
fn test_targets_named(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let mut rest = line;
        while let Some(at) = rest.find("--test ") {
            rest = &rest[at + "--test ".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if !name.is_empty() {
                out.push(name);
            }
        }
    }
    out
}

/// **The scan, shown catching the line that shipped.** Three nightly jobs invoked
/// `--test argv_drift_tests` for weeks after that target was deleted, and a scan that has never
/// objected to anything reads exactly like a clean tree.
#[test]
fn the_test_target_scan_sees_the_line_that_was_broken() {
    let broken = "      run: cargo test --release --test argv_drift_tests -- --nocapture";
    assert_eq!(
        test_targets_named(broken),
        vec!["argv_drift_tests".to_string()]
    );

    // Two on one line, which is how the tools nightly wrote it.
    let two = "  \"cd /src && cargo test --test argv_drift_tests --test terminator_probe_tests\"";
    assert_eq!(
        test_targets_named(two),
        vec![
            "argv_drift_tests".to_string(),
            "terminator_probe_tests".to_string()
        ]
    );

    // The fixed form names the real target and selects a module inside it.
    assert_eq!(
        test_targets_named("run: cargo test --test suite -- argv_drift_tests:: --nocapture"),
        vec!["suite".to_string()]
    );

    // And a line with no `--test` at all contributes nothing, so the scan is not matching
    // everything it reads.
    assert!(test_targets_named("run: cargo build --release").is_empty());
}

/// **Every image the argv gates are pointed at is asked in a way that image can answer.**
///
/// The nightly probe step ran `cargo test` against whatever image the matrix named, on the
/// assumption every one carries a toolchain. `metacall/guix` does not — it is a runtime stage
/// that receives a binary from a builder — so the guix leg was added, appeared in the job list,
/// and would have died on `cargo: not found`. The matrix now carries a `probe` field, and this
/// keeps the field and the Dockerfile from drifting apart.
///
/// **What this can and cannot see.** It checks the *pairing*: an image driven as a prebuilt
/// `suite` binary must ship one, and an image that ships one must be driven that way. It cannot
/// tell whether a new image has a toolchain — that is a fact about a base image, not about text
/// in this repository — and the honest backstop for that is the step itself, which fails loudly
/// on a missing `cargo` rather than quietly measuring nothing.
#[test]
fn every_probed_image_is_asked_in_a_form_it_can_run() {
    let ci = read(".github/workflows/ci.yml");

    let rows: Vec<&str> = ci
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("- { distro:"))
        .collect();
    assert!(
        rows.len() >= 4,
        "found {} matrix rows declaring a distro in ci.yml; this reader has stopped matching the \
         file it reads",
        rows.len()
    );

    let field = |row: &str, key: &str| -> Option<String> {
        let at = row.find(&format!("{key}: "))? + key.len() + 2;
        let rest = &row[at..];
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        Some(rest[..end].trim().trim_matches('\'').to_string())
    };

    for row in rows {
        let distro = field(row, "distro").expect("a row that starts `- { distro:` has one");
        // Only the nightly matrix carries `probe`; the fast one shares a single step. A row
        // without it is one of those, and it is the step's own hardcoded command that applies.
        let Some(probe) = field(row, "probe") else {
            continue;
        };
        let dockerfile = read(&format!("docker/integration/Dockerfile.{distro}"));
        let ships_binary = dockerfile.contains("/usr/local/bin/suite");
        // `contains`, not `starts_with`: a probe may now be a compound command — guix's starts
        // its daemon before running anything — and `sh setup.sh && cargo test` starts with `sh`
        // while being every bit the cargo form.
        let driven_as_binary = !probe.contains("cargo");

        assert_eq!(
            ships_binary,
            driven_as_binary,
            "`{distro}` is probed with `{probe}` and its Dockerfile {} a prebuilt suite binary. \
             An image that ships one and is asked with cargo compiles a second copy for no \
             reason; an image asked for the binary without shipping it cannot run the gate at all.",
            if ships_binary {
                "ships"
            } else {
                "does not ship"
            }
        );
    }
}
