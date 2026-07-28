//! `--dry-run` performs nothing, checked over **every** subcommand rather than the ones that
//! had a bug report.
//!
//! The history this exists to end: the flag was consulted per verb, so the question was never
//! "is `uninstall` fixed" but "which verbs remembered". Round 1 fixed `uninstall`, `unmanage`,
//! `module create` and `schedule add`. Round 2's audit then found `activate`, `deactivate`,
//! `lock`, `git init` and `config init` still acting — and it had probed 13 of 61 subcommands,
//! so the honest reading of that result was "at least five", never "exactly five".
//!
//! Two properties, and the second is what makes the first worth anything:
//!
//! 1. **Byte-identical.** Snapshot the config directory, run under `--dry-run`, snapshot again,
//!    and require the two to match exactly.
//! 2. **A control that proves the case could have failed.** The same command without the flag
//!    must change the directory. Without this, a case whose fixture made the command a no-op
//!    passes while proving nothing — which is exactly how the grader's first draft scored
//!    `activate` as passing, on a fixture where the profile was already active.
//!
//! And a third, about coverage rather than behaviour: **every subcommand `--help` lists is
//! either exercised here or exempted with a reason, and every exemption names a subcommand
//! that exists.** E29 was a stale exemption for `undo`, a command that had been renamed away;
//! an exemption list nothing validates is a list that grows quietly.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One command to check, with the setup that makes it able to do something.
struct Case {
    /// What to run, after `--dry-run`.
    argv: &'static [&'static str],
    /// Files to place in the config directory first, so the command has work to do.
    setup: &'static [(&'static str, &'static str)],
}

/// The commands that mutate the config directory and can be driven with no network, no package
/// manager and no terminal.
const CASES: &[Case] = &[
    Case {
        argv: &["activate", "Work"],
        setup: &[(
            "profiles/Work",
            "use starter
",
        )],
    },
    Case {
        argv: &["activate", "-a", "Work"],
        setup: &[(
            "profiles/Work",
            "use starter
",
        )],
    },
    Case {
        argv: &["deactivate", "Main"],
        setup: &[],
    },
    Case {
        argv: &["profile", "create", "Newone"],
        setup: &[],
    },
    Case {
        argv: &["profile", "save", "Snapshotted"],
        setup: &[],
    },
    Case {
        argv: &["module", "create", "extra"],
        setup: &[],
    },
    Case {
        argv: &["lock"],
        setup: &[],
    },
    Case {
        argv: &["config", "init", "--force"],
        setup: &[],
    },
    Case {
        argv: &["git", "init"],
        setup: &[],
    },
];

/// Subcommands not driven here, each with the reason. Checked against `--help`, so a name that
/// stops existing fails this file rather than sitting in the list forever (E29).
const EXEMPT: &[(&str, &str)] = &[
    ("sync", "covered by grader_extras_guard_tests and dry_run_tests; needs backends"),
    ("rebuild", "removes and reinstalls through a real manager"),
    ("watch", "runs until interrupted"),
    ("run", "runs a declared command; not a config mutation"),
    ("heal", "repairs the journal in the data dir, not the config dir"),
    ("remove-orphans", "needs a manager that reports orphans"),
    ("clean-cache", "clears the cache dir, not the config dir"),
    ("reset", "refuses without a terminal — asserted in grader_refusal_exit_code_tests"),
    ("check", "read-only — reports, never writes"),
    ("vars", "read-only — prints this host's resolved variables"),
    ("purge-unmanaged", "needs an adopted machine; refuses on the ratio check first"),
    ("protected", "read-only — explains why a name is guarded"),
    ("plan", "writes to a path the user names, not into the config dir"),
    ("apply", "applies a saved plan through a real manager"),
    ("unlock", "covered by lock: same ledger, same writer"),
    ("teleport", "rewrites a line then syncs through a real manager"),
    ("search", "read-only — queries the managers"),
    ("update", "refreshes manager indexes; touches no config"),
    ("upgrade", "needs a real manager"),
    ("list", "read-only — asks each manager what it has"),
    ("info", "read-only — describes one package"),
    ("install", "covered by dry_run_tests; needs a real manager"),
    ("uninstall", "covered by dry_run_tests"),
    (
        "unmanage",
        "covered by dry_run_tests; measured — it needs the package in the state registry,          not merely declared, so it cannot be driven from a config fixture alone",
    ),
    ("repo", "needs a manager that manages repositories"),
    ("adopt", "needs installed packages to adopt"),
    ("add", "covered by install: the same declare path"),
    ("shell", "starts an interactive session that ends on exit"),
    ("history", "read-only — reads the git log of the config"),
    ("snapshot", "needs a snapshot provider"),
    ("rollback", "needs a snapshot to roll back to"),
    ("diff", "read-only — compares two revisions"),
    ("try", "needs a container runtime"),
    ("eval", "read-only — resolves the model and prints it"),
    ("repl", "interactive — needs a terminal"),
    ("schedule", "covered by dry_run_tests; provisions onto the OS scheduler"),
    ("path", "read-only — prints where things live"),
    ("init", "creates the config dir itself, so there is no before-state to compare"),
    ("sbom", "read-only — emits a bill of materials"),
    ("export", "writes to a path the user names"),
    ("bundle", "writes to a path the user names"),
    ("restore", "reads a bundle and installs through real managers"),
    ("why", "read-only — explains a package's provenance"),
    ("service", "needs an init system"),
    ("bisect", "runs many syncs through real managers"),
    ("fleet", "talks to other hosts"),
    ("hooks", "read-only listing of the configured hooks"),
    ("unhold", "holds live in the data dir, not the config dir"),
    ("hold", "holds live in the data dir, not the config dir"),
    ("edit", "opens the user's editor on a manifest"),
    ("policy", "read-only listing of the guard rules"),
    ("completions", "prints a shell script"),
    ("self-upgrade", "replaces the binary, not the config"),
    ("help", "prints help and exits"),
];

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_linix")
}

fn run(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(bin())
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

/// Every file under the config directory, by relative path, with its bytes.
///
/// Content and not just names: `activate` rewrites `active` in place, so a listing would call
/// the switched profile no change at all.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else if let Ok(bytes) = std::fs::read(&p) {
                let rel = p
                    .strip_prefix(base)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, bytes);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn fixture(name: &str, setup: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (out, code) = run(&dir, &["init"]);
    assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
    for (rel, body) in setup {
        let p = dir.join("config").join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
    }
    dir
}

fn slug(argv: &[&str]) -> String {
    argv.join("-").replace([':', '/', '-'], "_")
}

/// What changed between two snapshots, as human-readable lines.
fn diff(before: &BTreeMap<String, Vec<u8>>, after: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in after {
        match before.get(k) {
            None => out.push(format!("created {}", k)),
            Some(old) if old != v => out.push(format!("changed {}", k)),
            Some(_) => {}
        }
    }
    for k in before.keys() {
        if !after.contains_key(k) {
            out.push(format!("deleted {}", k));
        }
    }
    out.sort();
    out
}

#[test]
fn a_preview_leaves_the_config_byte_identical() {
    let mut failures = Vec::new();

    for case in CASES {
        // The preview.
        let dir = fixture(&format!("dryrun-{}", slug(case.argv)), case.setup);
        let cfg = dir.join("config");
        let before = snapshot(&cfg);
        let mut args = vec!["--dry-run"];
        args.extend_from_slice(case.argv);
        let (out, _) = run(&dir, &args);
        let after = snapshot(&cfg);
        let changed = diff(&before, &after);

        // The control, on its own fresh fixture: the same command without the flag has to
        // change something, or the case above proved nothing.
        let ctl_dir = fixture(&format!("control-{}", slug(case.argv)), case.setup);
        let ctl_cfg = ctl_dir.join("config");
        let ctl_before = snapshot(&ctl_cfg);
        let (ctl_out, _) = run(&ctl_dir, case.argv);
        let ctl_changed = diff(&ctl_before, &snapshot(&ctl_cfg));

        if ctl_changed.is_empty() {
            failures.push(format!(
                "`linix {}` — THE CONTROL DID NOTHING. Without a run that changes the config, \
                 the dry-run assertion below cannot fail and proves nothing. Fix the fixture.\n\
                 control output:\n{}",
                case.argv.join(" "),
                ctl_out.trim()
            ));
            continue;
        }

        if !changed.is_empty() {
            failures.push(format!(
                "`linix --dry-run {}` changed the config directory:\n    {}\n  \
                 (the same command without --dry-run changes: {})\n  output:\n{}",
                case.argv.join(" "),
                changed.join("\n    "),
                ctl_changed.join(", "),
                out.trim()
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "--dry-run performed {} of {} commands:\n\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n\n")
    );
}

/// Coverage, asserted rather than assumed — in both directions.
#[test]
fn every_subcommand_is_either_exercised_or_exempted_with_a_reason() {
    let out = Command::new(bin())
        .arg("--help")
        .output()
        .expect("the binary should run");
    let help =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);

    // The `Commands:` block, one name per line at two spaces of indent.
    let mut listed: Vec<String> = Vec::new();
    let mut in_commands = false;
    for line in help.lines() {
        if line.starts_with("Commands:") {
            in_commands = true;
            continue;
        }
        if in_commands {
            if line.starts_with("Options:") || line.trim().is_empty() && !listed.is_empty() {
                if line.starts_with("Options:") {
                    break;
                }
                continue;
            }
            let t = line.trim_start();
            if line.starts_with("  ") && !line.starts_with("      ") {
                if let Some(name) = t.split_whitespace().next() {
                    if name.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
                        listed.push(name.to_string());
                    }
                }
            }
        }
    }

    // A floor: if this parse stops matching, everything below passes over an empty list.
    assert!(
        listed.len() >= 40,
        "parsed only {} subcommand(s) out of `--help`; this scan has stopped matching it:\n{}",
        listed.len(),
        help
    );

    let exercised: Vec<&str> = CASES.iter().map(|c| c.argv[0]).collect();
    let exempt: Vec<&str> = EXEMPT.iter().map(|(n, _)| *n).collect();

    let unaccounted: Vec<&String> = listed
        .iter()
        .filter(|n| !exercised.contains(&n.as_str()) && !exempt.contains(&n.as_str()))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "these subcommands are neither driven under --dry-run nor exempted with a reason: \
         {:?}\n\nAdd a Case, or an EXEMPT entry saying why it cannot be driven here. A verb \
         added without either is a verb nobody has asked whether --dry-run performs.",
        unaccounted
    );

    // The other direction, which is the one E29 was about: an exemption for a command that
    // does not exist. `undo` sat in two harness exemption lists after being renamed away.
    let stale: Vec<&str> = exempt
        .iter()
        .copied()
        .filter(|n| !listed.contains(&n.to_string()))
        .collect();
    assert!(
        stale.is_empty(),
        "EXEMPT names subcommands the binary does not have: {:?}",
        stale
    );

    // And no reason may be empty, or the list degenerates into the exemption list it replaced.
    for (name, reason) in EXEMPT {
        assert!(
            reason.len() > 10,
            "the exemption for `{}` does not say why: {:?}",
            name,
            reason
        );
    }
}
