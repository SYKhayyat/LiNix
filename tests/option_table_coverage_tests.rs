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

use linix::config::grammar::statement::{keys_for_kind, OptionKind};

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
