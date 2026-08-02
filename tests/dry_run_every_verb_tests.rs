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
//!
//! **The blind spot this file had, and what it cost.** Both snapshots walked the *config*
//! directory, and three exemptions said so out loud: `hold`, `unhold` and `heal` were excused
//! because "holds live in the data dir, not the config dir". The data directory holds
//! `registry.json` — the managed set, the file that decides whether the next `sync` *removes* a
//! package. So `--dry-run adopt` recorded 112 packages as managed while correctly not writing
//! the manifest that declares them, which is the one state the model reads as *the user deleted
//! every line*; `linix check` then said `112 to remove … run linix sync`, and it did. The gate
//! that says "every subcommand" could not have seen it: an exemption naming the instrument's
//! blind spot is not a reason, it is the finding. Both snapshots now walk the whole fixture —
//! config, data and the working directory — and the three excused verbs are driven.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One command to check, with the setup that makes it able to do something.
struct Case {
    /// What to run, after `--dry-run`.
    argv: &'static [&'static str],
    /// Files to place before the run, **relative to the fixture root** — so a case can plant
    /// `data/journal.jsonl` as readily as `config/profiles/Work`.
    setup: &'static [(&'static str, &'static str)],
    /// Commands run *for real* first, so the measured one has something to change. `unhold`
    /// releases nothing on a machine holding nothing.
    pre: &'static [&'static [&'static str]],
    /// Output that means "this host gave me nothing to do". A case printing it is **skipped and
    /// named**, never passed: `adopt` needs installed packages, and a green result on a host
    /// with none is the failure mode this file exists to end.
    nothing_to_do: Option<&'static str>,
}

/// The commands that mutate the config directory and can be driven with no network, no package
/// manager and no terminal.
const CASES: &[Case] = &[
    Case {
        argv: &["activate", "Work"],
        setup: &[("config/profiles/Work", "use starter\n")],
        pre: &[],
        nothing_to_do: None,
    },
    Case {
        argv: &["activate", "-a", "Work"],
        setup: &[("config/profiles/Work", "use starter\n")],
        pre: &[],
        nothing_to_do: None,
    },
    Case {
        argv: &["deactivate", "Main"],
        setup: &[],
        pre: &[],
        nothing_to_do: None,
    },
    Case {
        argv: &["profile", "create", "Newone"],
        setup: &[],
        pre: &[],
        nothing_to_do: None,
    },
    Case {
        argv: &["profile", "save", "Snapshotted"],
        setup: &[],
        pre: &[],
        nothing_to_do: None,
    },
    Case {
        argv: &["module", "create", "extra"],
        setup: &[],
        pre: &[],
        nothing_to_do: None,
    },
    Case {
        argv: &["lock"],
        setup: &[],
        pre: &[],
        nothing_to_do: None,
    },
    Case {
        argv: &["config", "init", "--force"],
        setup: &[],
        pre: &[],
        nothing_to_do: None,
    },
    Case {
        argv: &["git", "init"],
        setup: &[],
        pre: &[],
        nothing_to_do: None,
    },
    // The three the config-only snapshot excused, and the one that made it matter. Every one
    // writes `data/registry.json` or `data/journal.jsonl` and nothing else.
    Case {
        argv: &["hold", "github:sharkdp/hexyl"],
        setup: &[("config/modules/starter.txt", "github:sharkdp/hexyl\n")],
        pre: &[],
        nothing_to_do: None,
    },
    Case {
        argv: &["unhold", "github:sharkdp/hexyl"],
        setup: &[("config/modules/starter.txt", "github:sharkdp/hexyl\n")],
        pre: &[&["hold", "github:sharkdp/hexyl"]],
        nothing_to_do: None,
    },
    // An interrupted removal in the WAL: what `heal` exists for, and the only fixture that
    // gives it work. The action is a *removal* of a package this host does not have, so the
    // recovery it attempts cannot install anything.
    Case {
        argv: &["heal", "-y"],
        setup: &[(
            // One JSON value per line — the WAL is a log, not a document.
            "data/journal.jsonl",
            r#"{"id":"github:linix-probe-zzz:wal","action":{"Remove":{"name":"linix-probe-zzz","backend":"github"}},"status":"InProgress","started_at_unix":1000000,"finished_at_unix":null,"error":null}"#,
        )],
        pre: &[],
        nothing_to_do: None,
    },
    Case {
        argv: &["adopt", "-y"],
        setup: &[],
        pre: &[],
        nothing_to_do: Some("Nothing to adopt"),
    },
    // Q15, ruled 2026-07-30: `bundle`'s product outlives the run and can be carried to another
    // machine, so a preview must not manufacture one. It was exempted here as "writes to a path
    // the user names" — which was the unruled guess, and which `--dry-run bundle` then used to
    // write all nine files and report "Bundle written to X" in the past tense.
    //
    // It needs no `pre`: `bundle` copies the config root, which `init` has already filled, so
    // the control changes something on every host.
    Case {
        argv: &["bundle", "--out", "bundle-out"],
        setup: &[],
        pre: &[],
        nothing_to_do: None,
    },
];

/// Subcommands not driven here, each with the reason. Checked against `--help`, so a name that
/// stops existing fails this file rather than sitting in the list forever (E29).
///
/// **A reason says what this file cannot supply — never what this file cannot see.** Three
/// entries here used to read "lives in the data dir, not the config dir", which described the
/// snapshot's blind spot and excused the verbs writing into it; that is how B-1 stayed invisible
/// through two rounds of a gate named "every verb". `hooks` was excused as "read-only listing"
/// while `hooks install` writes into a manager's system hook directory, and `path` as
/// "read-only" while `path --set` writes LiNix's own settings file.
const EXEMPT: &[(&str, &str)] = &[
    ("sync", "covered by grader_extras_guard_tests and dry_run_tests; needs backends"),
    ("rebuild", "removes and reinstalls through a real manager"),
    ("watch", "runs until interrupted"),
    ("run", "runs a declared command; not a config mutation"),
    ("remove-orphans", "needs a manager that reports orphans"),
    (
        "clean-cache",
        "needs a backend holding a real download cache; a fixture has none to clear",
    ),
    ("reset", "refuses without a terminal — asserted in grader_refusal_exit_code_tests"),
    ("check", "read-only — reports, never writes"),
    ("vars", "read-only — prints this host's resolved variables"),
    ("purge-unmanaged", "needs an adopted machine; refuses on the ratio check first"),
    ("protected", "read-only — explains why a name is guarded"),
    // Q15: `plan` is the whole exemption, and the reason is not that the user named the path —
    // they name it for `bundle` too. Its file IS the preview, so a `--dry-run plan` that wrote
    // nothing would be a command with no output.
    (
        "plan",
        "its file IS the preview — a `--dry-run plan` that wrote nothing would produce no          output at all (Q15, ruled 2026-07-30)",
    ),
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
    (
        "path",
        "`--set` writes LiNix's own settings file, which lives outside any directory a fixture \
         controls — driven by a_preview_does_not_store_a_new_config_root below",
    ),
    ("init", "creates the config dir itself, so there is no before-state to compare"),
    ("sbom", "read-only — measured: it takes no output flag and prints to stdout"),
    (
        "export",
        "driven by a_preview_does_not_write_native_manifests below, which can skip a host with          nothing to export; the table's control would read that as a broken fixture",
    ),
    ("restore", "reads a bundle and installs through real managers"),
    ("why", "read-only — explains a package's provenance"),
    ("service", "needs an init system"),
    ("bisect", "runs many syncs through real managers"),
    ("fleet", "talks to other hosts"),
    (
        "hooks",
        "install/uninstall write into a manager's system hook dir and need root; status and \
         shell-init are the read-only halves",
    ),
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
        // In the fixture, so a verb writing relative to the working directory writes where the
        // snapshot looks rather than into the repo.
        .current_dir(dir)
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

/// Every file under the fixture — `config/`, `data/`, and anything a verb drops in the working
/// directory — by relative path, with its bytes.
///
/// Content and not just names: `activate` rewrites `active` in place, so a listing would call
/// the switched profile no change at all. **`data/` and not just `config/`**: `registry.json` is
/// the managed set, and a preview that records a package there has armed the next `sync` to
/// remove it (B-1). Nothing is filtered out — a lock file this walk reports as changed is a
/// finding to read, not noise to exclude, and the diff names the file either way.
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

fn fixture(name: &str, case: &Case) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (out, code) = run(&dir, &["init"]);
    assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
    for (rel, body) in case.setup {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
    }
    for argv in case.pre {
        let (out, code) = run(&dir, argv);
        assert_eq!(
            code,
            0,
            "the fixture's own `linix {}` failed:\n{out}",
            argv.join(" ")
        );
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
    let mut skipped = Vec::new();

    for case in CASES {
        // The preview.
        let dir = fixture(&format!("dryrun-{}", slug(case.argv)), case);
        let before = snapshot(&dir);
        let mut args = vec!["--dry-run"];
        args.extend_from_slice(case.argv);
        let (out, _) = run(&dir, &args);
        let after = snapshot(&dir);
        let changed = diff(&before, &after);

        if let Some(marker) = case.nothing_to_do {
            if out.contains(marker) {
                skipped.push(format!(
                    "`linix {}` — this host gave it nothing to do (\"{marker}\")",
                    case.argv.join(" ")
                ));
                continue;
            }
        }

        // The control, on its own fresh fixture: the same command without the flag has to
        // change something, or the case above proved nothing.
        let ctl_dir = fixture(&format!("control-{}", slug(case.argv)), case);
        let ctl_before = snapshot(&ctl_dir);
        let (ctl_out, _) = run(&ctl_dir, case.argv);
        let ctl_changed = diff(&ctl_before, &snapshot(&ctl_dir));

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
                "`linix --dry-run {}` changed the fixture:\n    {}\n  \
                 (the same command without --dry-run changes: {})\n  output:\n{}",
                case.argv.join(" "),
                changed.join("\n    "),
                ctl_changed.join(", "),
                out.trim()
            ));
        }
    }

    // Named, not silent: a skip is the one honest outcome for a host that cannot supply the
    // work, and a count nobody prints is how coverage collapses unnoticed (G-11).
    if !skipped.is_empty() {
        eprintln!(
            "skipped {} of {} case(s):\n  {}",
            skipped.len(),
            CASES.len(),
            skipped.join("\n  ")
        );
    }

    assert!(
        failures.is_empty(),
        "--dry-run performed {} of {} commands:\n\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n\n")
    );
}

/// `path --set` writes `linix.settings.toml`, which lives beside the user's other application
/// settings rather than in any directory `LINIX_CONFIG_DIR` moves — so the only honest way to
/// assert a preview leaves it alone is to look at the real one.
///
/// It restores whatever it finds, in both directions, because a test that damages the machine
/// when it fails is a worse instrument than no test.
#[test]
fn a_preview_does_not_store_a_new_config_root() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("dryrun-path-set");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let (located, code) = run(&dir, &["path", "--explain"]);
    assert_eq!(code, 0, "{located}");
    let settings = located
        .lines()
        .find_map(|l| l.strip_prefix("settings file: "))
        .map(|p| PathBuf::from(p.trim()))
        .unwrap_or_else(|| panic!("`path --explain` names no settings file:\n{located}"));

    let before = std::fs::read(&settings).ok();
    let (out, _) = run(
        &dir,
        &["--dry-run", "path", "--set", &dir.to_string_lossy()],
    );
    let after = std::fs::read(&settings).ok();

    if after != before {
        match &before {
            Some(bytes) => std::fs::write(&settings, bytes).unwrap(),
            None => std::fs::remove_file(&settings).unwrap(),
        }
        panic!(
            "`linix --dry-run path --set` wrote {} (restored). It said:\n{}",
            settings.display(),
            out.trim()
        );
    }
    assert!(
        out.contains("[DRY-RUN]"),
        "the preview neither wrote nor said it would not:\n{out}"
    );
}

/// `export` writes native manifests (Brewfile, requirements.txt, package.json, Aptfile) into a
/// directory the user names, and Q15 rules it goes with `bundle`: its product outlives the run.
///
/// **Measured before it was built, because the ruling says so.** The round-5 grader could not
/// measure `export` — the fixture had nothing to export, so neither run wrote anything and there
/// was no control — and ruled it with `bundle` on the reasoning rather than on a measurement.
/// The measurement says `export` **already honours the flag**: it prints `[DRY-RUN] would write
/// <path>` per manifest and writes none of them. So this test is a regression guard, not a fix,
/// and W43's code change is `bundle` alone.
///
/// It is here rather than in `CASES` because a host with no packages to export makes the
/// control write nothing, which the table correctly reads as a broken fixture. Such a host is
/// **skipped and named**, the same way the table skips `adopt`.
#[test]
fn a_preview_does_not_write_native_manifests() {
    let case = Case {
        argv: &["export", "--out", "export-out"],
        setup: &[],
        pre: &[&["adopt", "-y"]],
        nothing_to_do: None,
    };

    // The control first: it decides whether this host can measure anything at all.
    let ctl = fixture("control-export", &case);
    let ctl_before = snapshot(&ctl);
    let (ctl_out, _) = run(&ctl, case.argv);
    let ctl_changed = diff(&ctl_before, &snapshot(&ctl));
    if ctl_changed.is_empty() {
        eprintln!(
            "skipped: this host has nothing `export` can emit, so a preview writing nothing              would prove nothing. `export` said:
{}",
            ctl_out.trim()
        );
        return;
    }

    let dir = fixture("dryrun-export", &case);
    let before = snapshot(&dir);
    let mut args = vec!["--dry-run"];
    args.extend_from_slice(case.argv);
    let (out, _) = run(&dir, &args);
    let changed = diff(&before, &snapshot(&dir));

    assert!(
        changed.is_empty(),
        "`linix --dry-run export` wrote:
    {}
  (the control writes: {})
  output:
{}",
        changed.join(
            "
    "
        ),
        ctl_changed.join(", "),
        out.trim()
    );
    assert!(
        out.contains("[DRY-RUN]"),
        "the preview wrote nothing and did not say so, which is the half of B-1 that was worse          than the writing:
{out}"
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
