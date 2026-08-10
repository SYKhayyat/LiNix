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
                if candidate.is_file() {
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
            .filter_map(|l| l.trim().strip_prefix("ENV LINIX_IT_IMAGE="))
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
/// It is the same shape as the four release assets that were all named `linix`, and it survived
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
        .filter(|l| l.contains("dist/linix-"))
        .flat_map(|l| {
            l.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
                .filter_map(|w| w.strip_prefix("linix-"))
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
