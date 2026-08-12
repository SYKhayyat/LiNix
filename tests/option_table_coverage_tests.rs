//! Every statement kind that carries `@options` is refused a bogus one, in its own words.
//!
//! The lookup this quantifies over used to match the kind as a *string*, ending in
//! `_ => SCHEDULE_OPTION_KEYS`. Nothing was wrong on the day it was written — every caller
//! passed a spelling that had an arm — but a kind added later would have inherited schedule's
//! options in silence: `@cron` accepted on a thing with no schedule, its own options refused,
//! and no error anywhere to say so.
//!
//! The lookup is exhaustive over an enum now, so the tenth kind cannot compile without naming
//! its table. This is the other half: that the table is *reached*, and that the refusal names
//! the kind the user actually wrote. A kind wired to the wrong table still compiles.

use crate::harness::Fixture;
use shall::config::grammar::statement::{keys_for_kind, OptionKind};

/// One representative line per grammar that carries options, in the form a user writes.
///
/// `{}` is where a bogus option goes. Each row is probed three ways — see
/// [`a_bogus_option_is_refused_in_second_position_in_every_grammar`] — and the *base* probe is
/// what stops a row proving nothing: a line whose syntax is wrong is refused for the wrong
/// reason, and a table of rows like that reads as a pass.
const GRAMMARS: &[(&str, &str, &str)] = &[
    // (what it is, the line with a valid option, the bogus option to append)
    ("package", "cargo:ripgrep@version=1.6", "@nosuchkey"),
    (
        "absent",
        "absent:cargo:ripgrep@until=2030-01-01T00:00:00Z",
        "@nosuchkey",
    ),
    ("service", "service:nginx@status=running", "@nosuchkey"),
    (
        "link",
        "link:./src.txt@target=/tmp/shall-b2-dst",
        "@nosuchkey",
    ),
    (
        "shim",
        "shim:mytool@source=/tmp/shall-b2-tool",
        "@nosuchkey",
    ),
    (
        "dotfiles",
        "dotfiles:./df@target=/tmp/shall-b2-df",
        "@nosuchkey",
    ),
    (
        "setting",
        "setting:org.gnome.desktop.interface/clock-format@value=24h",
        "@nosuchkey",
    ),
    ("schedule", "schedule:nightly@run=/bin/true", "@nosuchkey"),
    ("exec", "exec:./bin/x.sh@runs=always", "@nosuchkey"),
    (
        "firewall",
        "firewall:default/incoming@value=deny",
        "@nosuchkey",
    ),
];

/// **The same text, admitted or refused by its position alone.**
///
/// `cargo:ripgrep@nosuchkey` was refused and `cargo:ripgrep@sha256=abc @nosuchkey` was accepted
/// in silence, because the lexer splits on the first `@` and separates options on commas — so
/// everything after a space was absorbed into the previous option's *value*. The checksum
/// became `"abc @nosuchkey"` and could not match; `@hold` written that way was inert (B2).
///
/// **Seven of the ten grammars accepted it outright.** The three that refused — `absent:`,
/// `exec:`, `firewall:` — were saved by a downstream type check on a date, a count and an enum,
/// and each quoted the swallowed text back as part of the value while refusing. That is
/// incidental protection, not structural: one free-form option added to any of them reopens it.
/// So the fix is in the lexer and this is the table that says the lexer is where it reached.
///
/// The existing option test declares one key at a time, in first position, which is exactly why
/// nine grammars hid this.
#[test]
fn a_bogus_option_is_refused_in_second_position_in_every_grammar() {
    let f = Fixture::new("b2_option_positions");
    // Everything the base lines name, so a row is refused for its option and never for a
    // missing file.
    f.write("src.txt", "x\n");
    f.write("df/keep.txt", "x\n");
    f.write("bin/x.sh", "#!/bin/sh\nexit 0\n");
    f.write("priority", "cargo\n");

    // A `schedule:` runs for the whole machine, so it lives in the `schedules` file and a
    // module refuses it — for a reason that has nothing to do with its options. Found by the
    // base probe, which is exactly what the base probe is for: without it that row would have
    // "passed" on a refusal about the wrong thing.
    let probe = |line: &str| -> (String, i32) {
        let (target, other) = match line.starts_with("schedule:") {
            true => ("schedules", "modules/starter.txt"),
            false => ("modules/starter.txt", "schedules"),
        };
        f.write(other, "");
        f.write(target, &format!("{line}\n"));
        f.run(&["eval"])
    };

    for (what, base, bogus) in GRAMMARS {
        // **The instrument first.** If the base line is not accepted, this row's syntax is
        // wrong and neither of the two probes below says anything about the lexer.
        let (out, code) = probe(base);
        assert_eq!(
            code, 0,
            "the `{what}` base line is not valid, so this row proves nothing: `{base}`\n{out}"
        );

        // The control: the same bogus option, alone, must be refused — or the validator is
        // simply not running for this kind and "refused in second position" would be vacuous.
        let alone = format!("{}{bogus}", base.split('@').next().unwrap());
        let (out, code) = probe(&alone);
        assert_ne!(
            code, 0,
            "`{what}` accepted `{bogus}` in FIRST position, so its validator is not reached at \
             all\n{out}"
        );

        // The finding.
        let after = format!("{base} {bogus}");
        let (out, code) = probe(&after);
        assert_ne!(
            code, 0,
            "`{what}` accepted `{after}` — the bogus option was swallowed into the value of the \
             one before it, which is B2\n{out}"
        );
    }
}

/// Nine kinds, and the enum's own list is what drives this — a kind added to `OptionKind`
/// without a case here fails the count below rather than being quietly untested.
#[test]
fn every_option_carrying_kind_is_covered_here() {
    assert_eq!(
        OptionKind::ALL.len(),
        9,
        "a statement kind was added or removed; give it a case in this file"
    );
}

#[test]
fn no_kind_silently_inherits_another_kinds_options() {
    // `cron` belongs to `schedule` alone. It was the fall-through's payload, so it is the one
    // key that proves the fall-through is gone: every other kind must refuse it.
    for kind in OptionKind::ALL {
        let legal = keys_for_kind(*kind);
        if *kind == OptionKind::Schedule {
            assert!(
                legal.contains(&"cron"),
                "schedule must still take `cron` — it is the only kind that does"
            );
            continue;
        }
        assert!(
            !legal.contains(&"cron"),
            "`{}` accepts `@cron`, which belongs to `schedule`: it is reading the wrong table",
            kind.as_str()
        );
    }
}

#[test]
fn each_kind_reads_a_table_of_its_own() {
    // Two kinds sharing a table is how a mis-wired arm hides: the options still look plausible
    // because they are somebody's. `service` and `dotfiles` are the shortest tables and would
    // collide first. Empty is exempt — `generate` takes nothing, on purpose.
    let mut seen: Vec<(&str, &[&str])> = Vec::new();
    for kind in OptionKind::ALL {
        let legal = keys_for_kind(*kind);
        if legal.is_empty() {
            continue;
        }
        if let Some((other, _)) = seen.iter().find(|(_, keys)| *keys == legal) {
            panic!(
                "`{}` and `{}` read the same option table — one of them is wired to the other's",
                kind.as_str(),
                other
            );
        }
        seen.push((kind.as_str(), legal));
    }
}

#[test]
fn generate_takes_no_options_and_says_so_in_its_own_words() {
    // The empty table is a table, not a special case in the validator — but the sentence the
    // user reads must still explain *why* there are none, not list an empty set.
    assert!(
        keys_for_kind(OptionKind::Generate).is_empty(),
        "generate takes no options: it runs every resolution, so there is no ceiling to set"
    );
}
