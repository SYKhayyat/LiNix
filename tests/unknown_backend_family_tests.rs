//! Q9, on the form its own enumeration missed.
//!
//! Q9 ruled: *every verb taking a backend name refuses an unknown one*, with `install`'s
//! message, and named the four that take it as a `--backend` flag — "checked from the code
//! rather than from the one that was reported". The `backend:name` **spec** form was never in
//! that list, so the ruling covered half its surface. Measured on 2026-07-29, after Q9 shipped:
//!
//! | command                        | answer                                          | exit |
//! |--------------------------------|-------------------------------------------------|------|
//! | `hold nosuchbackend:foo`       | `Held 1 package(s).`                            | 0    |
//! | `unhold nosuchbackend:foo`     | `Released 1 hold(s).`                           | 0    |
//! | `unmanage nosuchbackend:foo`   | `not managed and not declared — nothing to forget` | 0 |
//! | `why nosuchbackend:foo`        | `not under Shall management`                     | 0    |
//! | `upgrade nosuchbackend:foo`    | `not a managed package — skipping`               | 0    |
//! | `rebuild nosuchbackend:foo`    | `skipping — not declared in any active module`    | 0    |
//! | `unlock backends nosuchbackend:foo` | `was not frozen on this host`               | 0    |
//! | `uninstall nosuchbackend:foo`  | `is not declared in any active file`             | 1    |
//! | `info nosuchbackend:foo`       | `is not installed on this machine`               | 0    |
//!
//! `hold` is the sharp one: it *recorded* a hold against a manager that does not exist and
//! reported success. Every other line is a true sentence about the wrong thing — each is also
//! exactly what a correctly-spelled name in a real manager gets when there is nothing to do, so
//! a typo is indistinguishable from a no-op. That is the silence Principle I forbids and the
//! same reasoning Q9 already accepted for `list -b`.
//!
//! **The check is derived, not listed.** The verbs are read from `--help`, and the ones exempt
//! from the rule are exempt for a stated reason and asserted to exist — because an unvalidated
//! exemption list is E29, where both harnesses exempted `undo`, a subcommand that had been
//! renamed away.

use std::collections::BTreeSet;

/// A prefix no build of Shall has a backend for.
const UNKNOWN: &str = "nosuchbackend:foo";

/// Subcommands whose positional argument is **not** a package spec, each with the vocabulary it
/// does take. They answer in that vocabulary instead, which is right — `activate` telling you
/// `nosuchbackend:foo` is not a profile name is a better answer than a backend refusal.
///
/// Asserted to exist below. A name that stops being a subcommand has to leave this list, or the
/// list starts excusing verbs that are no longer here (E29).
const NOT_A_PACKAGE_SPEC: &[(&str, &str)] = &[
    ("activate", "a profile name"),
    ("deactivate", "a profile name"),
    ("add", "an adapter source — a path or a URL"),
    (
        "adapters",
        "an extension surface — one of the eight `adapters/*.toml` files",
    ),
    ("apply", "a saved plan file"),
    ("check", "a section of `check`"),
    ("completions", "a shell name"),
    ("diff", "a git commit"),
    ("rollback", "a git commit"),
    ("edit", "a file in the config repo"),
    ("restore", "a bundle file"),
    ("run", "a program to execute"),
    ("shell", "a shell to open"),
    ("fleet", "a fleet subcommand"),
    // The first positional is what to freeze — a group, a kind, or `kind:qualifier` — and the
    // package names come after it. The `backend:name` question is still asked of *those*:
    // `handle_lock` and `handle_unlock` both run `require_known_spec_backends` when the
    // selection is the backends kind, which is the namespace where a prefix is a manager.
    (
        "lock",
        "what to freeze — a group, a kind, or `kind:qualifier`",
    ),
    (
        "unlock",
        "what to release — a group, a kind, or `kind:qualifier`",
    ),
    // Reads rather than resolves: `search` is the command for "which manager has this?", so a
    // prefix is not part of its question.
    ("search", "a search term"),
];

/// Verbs that take a `backend:name` and must therefore refuse an unknown prefix.
///
/// Derived: every subcommand `--help` lists, minus the ones above, minus the ones that reject
/// the argument as a usage error (a verb taking no positional cannot be asked this question).
fn spec_taking_verbs(f: &Fixture) -> BTreeSet<String> {
    let exempt: BTreeSet<&str> = NOT_A_PACKAGE_SPEC.iter().map(|(n, _)| *n).collect();
    f.subcommands()
        .into_iter()
        .filter(|c| !exempt.contains(c.as_str()))
        .filter(|c| {
            // A usage error means this verb has no positional to give — not a gap.
            let (out, _) = f.run(&[c, UNKNOWN, "-y"]);
            !out.contains("unrecognized subcommand")
                && !out.contains("required arguments were not provided")
                && !out.contains("unexpected argument")
                && !out.contains("invalid value")
        })
        .collect()
}

use crate::harness::Fixture;

impl Fixture {
    /// Every subcommand, from the program rather than from a list in this file.
    fn subcommands(&self) -> Vec<String> {
        let (help, _) = self.run(&["--help"]);
        let mut out = Vec::new();
        let mut in_commands = false;
        for line in help.lines() {
            if line.starts_with("Commands:") {
                in_commands = true;
                continue;
            }
            if in_commands {
                if line.starts_with("Options:") {
                    break;
                }
                // `  <name>  <description>` — two leading spaces, then the name.
                let trimmed = line.trim_start();
                if line.len() - trimmed.len() == 2 {
                    if let Some(name) = trimmed.split_whitespace().next() {
                        if name.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
                            out.push(name.to_string());
                        }
                    }
                }
            }
        }
        assert!(
            out.len() > 20,
            "only {} subcommands parsed out of `--help`, so this test would examine almost \
             nothing — the help format changed:\n{help}",
            out.len()
        );
        out
    }
}

#[test]
fn every_verb_taking_a_package_spec_refuses_an_unknown_backend() {
    let f = Fixture::new("unknown-backend-family");

    // Control: the verb Q9 was written from still refuses, so a green run below cannot mean the
    // refusal was removed everywhere.
    let (control, code) = f.run(&["list", "-b", "nosuchbackend"]);
    assert_eq!(code, 1, "the control failed — `list -b` no longer refuses an unknown backend, so Q9 has regressed at its own reported instance:\n{control}");
    assert!(control.contains("is not a backend Shall uses"), "{control}");

    let verbs = spec_taking_verbs(&f);
    assert!(
        verbs.contains("install"),
        "`install` is the verb Q9's message comes from; if it is not in the derived set, the \
         derivation is wrong: {verbs:?}"
    );

    let mut silent: Vec<String> = Vec::new();
    for verb in &verbs {
        let (out, code) = f.run(&[verb, UNKNOWN, "-y"]);
        if !out.contains("is not a backend Shall uses") {
            silent.push(format!(
                "`shall {verb} {UNKNOWN}` → rc={code}, {}",
                out.lines().next().unwrap_or("(no output)").trim()
            ));
        }
    }

    assert!(
        silent.is_empty(),
        "these verbs take a `backend:name` and do not refuse a prefix that is not a backend:\n  \
         {}\n\nQ9 ruled that every verb taking a backend name refuses an unknown one, with \
         `install`'s message naming the `priority` file and the spelling. Each answer above is a \
         true sentence about the wrong thing, and each is byte-identical to what a \
         correctly-spelled name gets when there is nothing to do — so a typo cannot be told \
         from a no-op. If a verb belongs here for a reason, add it to `NOT_A_PACKAGE_SPEC` with \
         the vocabulary it does take.",
        silent.join("\n  ")
    );
    eprintln!(
        "{} verbs checked: {}",
        verbs.len(),
        verbs.iter().cloned().collect::<Vec<_>>().join(" ")
    );
}

/// The exemption list, validated. Both integration harnesses once exempted `undo`, a subcommand
/// that had been renamed to `snapshot`/`rollback` — so the exemption protected nothing and
/// nobody noticed for months (E29). An exemption naming something that does not exist is worse
/// than no exemption: it reads as coverage.
#[test]
fn every_exempted_verb_still_exists() {
    let f = Fixture::new("unknown-backend-exemptions");
    let subcommands: BTreeSet<String> = f.subcommands().into_iter().collect();
    let missing: Vec<&str> = NOT_A_PACKAGE_SPEC
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !subcommands.contains(*n))
        .collect();
    assert!(
        missing.is_empty(),
        "`NOT_A_PACKAGE_SPEC` exempts subcommands that do not exist: {missing:?}. Delete them — \
         an exemption for a verb that is gone is an exemption that reads as coverage."
    );
}

/// And the other direction, which is what stops the fix from becoming a new defect: a real
/// backend must still be accepted, qualified and bare alike. A refusal that fires on
/// `cargo:ripgrep` would be worse than the silence it replaced.
#[test]
fn a_real_backend_prefix_is_never_refused() {
    let f = Fixture::new("unknown-backend-no-over-refusal");

    // A backend this build registers, taken from the program rather than assumed. `list` names
    // only what is ready here, so this also skips cleanly on a bare machine.
    let (health, _) = f.run(&["check", "health"]);
    let ready: Vec<String> = health
        .lines()
        .filter(|l| l.starts_with("[READY]"))
        .filter_map(|l| l.split_whitespace().nth(1).map(String::from))
        .collect();
    let Some(backend) = ready.first() else {
        panic!("no backend is READY on this machine, so over-refusal cannot be tested here");
    };

    // `unlock` names its axis since Z2 — the names it takes are still `backend:name`, which is
    // what the Q9 check reads.
    let verbs: [&[&str]; 5] = [
        &["info"],
        &["why"],
        &["unmanage"],
        &["unhold"],
        &["unlock", "backends"],
    ];
    for verb in verbs {
        for arg in [
            format!("{backend}:shall-probe-zzz"),
            "shall-probe-zzz".into(),
        ] {
            let mut argv: Vec<&str> = verb.to_vec();
            argv.push(&arg);
            argv.push("-y");
            let (out, _) = f.run(&argv);
            assert!(
                !out.contains("is not a backend Shall uses"),
                "`shall {} {arg}` was refused, and `{backend}` is a backend this build \
                 registers. The Q9 check has started rejecting real names:\n{out}",
                verb.join(" ")
            );
        }
    }
}
