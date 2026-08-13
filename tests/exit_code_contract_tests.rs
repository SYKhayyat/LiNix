//! The exit-code contract `readme.md` publishes, asserted against the real binary.
//!
//! `readme.md` says "the same four everywhere, so a script can branch on them": 0 converged,
//! 1 failed, 2 differences, 3 refused. Nothing tested it, and three of the four commands a
//! script is most likely to get wrong — a mistyped subcommand, a mistyped flag, a bad flag on
//! a real subcommand — all returned 2, which the table defines as *a read-only command looked
//! and found work to do*. A CI job following the documentation read a typo as a drifted
//! machine (Q3, II.8, V.92).
//!
//! These run the built binary rather than a parser, because the defect was that clap exits
//! before Shall's own mapping runs — a test against the mapping would have passed throughout.

use std::process::Command;

fn exit_code(args: &[&str]) -> i32 {
    exit_code_in(env!("CARGO_TARGET_TMPDIR"), args)
}

fn exit_code_in(dir: &str, args: &[&str]) -> i32 {
    run_in(dir, args).0
}

/// The exit code **and** what the command said, because a test that asserts a refusal has to be
/// able to check that the refusal was reachable at all.
fn run_in(dir: &str, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(args)
        // A missing config must not turn a usage error into a different failure, and a real
        // one must not let the machine's state decide the code.
        .env("SHALL_CONFIG_DIR", format!("{dir}/config"))
        .env("SHALL_DATA_DIR", format!("{dir}/data"))
        // Piped, so `is_terminal` is false and the non-interactive refusals are reachable.
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    let mut said = String::from_utf8_lossy(&out.stdout).into_owned();
    said.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), said)
}

/// A config directory with a repo in it, so the commands below have something to refuse about.
fn scratch(name: &str) -> String {
    let dir = format!("{}/{}", env!("CARGO_TARGET_TMPDIR"), name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(format!("{dir}/config/modules")).unwrap();
    std::fs::create_dir_all(format!("{dir}/config/profiles")).unwrap();
    std::fs::write(format!("{dir}/config/priority"), "scoop\n").unwrap();
    std::fs::write(format!("{dir}/config/active"), "").unwrap();
    dir
}

/// Exit 2 is reserved for "a read-only command looked and found work to do". A typo has not
/// looked at the machine at all, so it cannot report on it.
#[test]
fn a_usage_error_is_a_failure_and_never_a_finding() {
    for args in [
        vec!["nosuchcommand"],
        vec!["--nosuchflag"],
        vec!["sync", "--badflag"],
        vec!["install", "--into"], // a flag whose value is missing
        vec![],                    // no subcommand at all
    ] {
        let code = exit_code(&args);
        assert_ne!(
            code,
            2,
            "`shall {}` exits 2, which the published table means as `the machine has drifted`",
            args.join(" ")
        );
        assert_eq!(code, 1, "`shall {}` should exit 1 (failed)", args.join(" "));
    }
}

/// The other half: asking for help is an answer, not a failure. A test that only pushed usage
/// errors to 1 could have taken `--help` with it and nothing would have noticed.
#[test]
fn asking_for_help_or_version_still_succeeds() {
    assert_eq!(exit_code(&["--help"]), 0);
    assert_eq!(exit_code(&["--version"]), 0);
    assert_eq!(exit_code(&["sync", "--help"]), 0);
}

/// Exit 3 exists so a script that retries on failure does not retry a decision. Shall worked
/// correctly and declined on purpose in both of these, and both returned 1 — indistinguishable
/// from a crash. Neither harness could see it: they assert refusals with `nok`, which takes any
/// non-zero code and therefore cannot tell 1 from 3.
///
/// **The premise is established rather than assumed, which it was not.** The
/// `purge-undeclared` half asserted exit 3 unconditionally. That refusal only exists when the
/// crawl finds undeclared packages — on a machine where the managers named in `priority` are
/// absent or empty, the command correctly reports a clean bill and exits 0. So the test passed
/// on a developer machine *because* it is used, and failed on the clean CI Windows runner: 534
/// passed, 1 failed, and it was the sole reason the Windows build was red.
///
/// The suite already found this class in the shell harness and wrote it down — *"the gate's
/// coverage is therefore inversely proportional to how much the machine is used, and it never
/// noticed"* — without anyone asking whether the Rust suite had the same defect. It did, in the
/// mirror image: greenest exactly where it is least trustworthy.
///
/// Which case the host is in is now read from the output instead of assumed, and both branches
/// are asserted. The ratio rule itself is pinned deterministically by
/// `cleanup::tests::the_ratio_catches_the_small_machine_a_count_misses`, so what is host-shaped
/// here is only whether this path can be reached — never what it must return when it is.
#[test]
fn a_refusal_exits_three_and_not_one() {
    let dir = scratch("refusal-purge");
    let (code, said) = run_in(&dir, &["purge-undeclared", "--yes"]);
    // The command's own words for "the crawl came back empty", which is the state in which
    // there is nothing to refuse about.
    let nothing_found = said.contains("Nothing to do") || said.contains("Nothing to delete");
    if nothing_found {
        assert_eq!(
            code, 0,
            "no manager in `priority` had anything undeclared, so this is a clean bill and \
             not a refusal — but it did not exit 0:\n{said}"
        );
    } else {
        assert_eq!(
            code, 3,
            "the unadopted-machine ratio refusal is a decision, not a failure:\n{said}"
        );
    }

    // This half establishes its own premise: `scratch` writes a config repo, and refusing to
    // reset over one is unconditional.
    let dir = scratch("refusal-reset");
    let (code, said) = run_in(&dir, &["reset"]);
    assert_eq!(
        code, 3,
        "refusing to reset while a config repo exists is a decision, not a failure:\n{said}"
    );
}

/// Every interactive prompt must refuse a non-interactive shell by name, and the list of
/// prompts is read out of the source rather than written down here.
///
/// A list of prompts is an assertion about the ones that are absent from it, and nothing
/// verifies that half — which is how two of the eight came to be missing the check, including
/// `purge-undeclared`, the most destructive command in the program, eighty lines above a
/// sibling that does it correctly.
#[test]
fn every_prompt_refuses_a_non_interactive_shell() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut unguarded = Vec::new();

    for entry in walk(std::path::Path::new(root)) {
        let src = std::fs::read_to_string(&entry).unwrap_or_default();
        for (n, line) in src.lines().enumerate() {
            let opens_a_prompt = line.contains("Confirm::new()")
                || line.contains("Confirm::with_theme")
                || line.contains("Input::new()")
                || line.contains("Select::new()")
                || line.contains("Password::new()");
            if !opens_a_prompt {
                continue;
            }
            // The guard belongs above the prompt in the same function, so the search starts at
            // the function's own opening line. A fixed window of lines is the wrong unit and
            // said so on the first run: `interactive_init` guards correctly and its prompt sits
            // 26 lines below the check.
            let from = src
                .lines()
                .take(n)
                .enumerate()
                .filter(|(_, l)| {
                    let t = l.trim_start();
                    t.starts_with("fn ")
                        || t.starts_with("pub fn ")
                        || t.starts_with("async fn ")
                        || t.starts_with("pub async fn ")
                        || t.starts_with("pub(crate) fn ")
                        || t.starts_with("pub(crate) async fn ")
                })
                .map(|(i, _)| i)
                .last()
                .unwrap_or(0);
            let guarded = src.lines().skip(from).take(n - from).any(|l| {
                l.contains("is_terminal")
                    || l.contains("unattended")
                    || l.contains("config.yes")
                    || l.contains("interactive")
            });
            if !guarded {
                unguarded.push(format!("{}:{}", entry.display(), n + 1));
            }
        }
    }

    assert!(
        unguarded.is_empty(),
        "these prompts hang or die with a bare IO error when nobody is at the keyboard, \
         instead of naming the flag that gets past them:\n  {}",
        unguarded.join("\n  ")
    );
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    out
}
