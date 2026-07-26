use super::dated::{dating_of, Dating};
use crate::config::grammar::{GrammarError, Options, Origin};
use chrono::{DateTime, Utc};

/// One resolved declaration and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub options: Options,
    pub origin: Origin,
    /// `false` for an `absent:` line. The only genuinely new thing the desired-state map
    /// could not already carry (SPEC Phase 2).
    pub present: bool,
}

/// Decide between two declarations of the same package (SPEC II.7 rules 5 and 6).
///
/// **Two active declarations that contradict = ERROR.** Not first-wins, not file order.
/// Files used to be read in filesystem order and the first declaration won, so
/// `a.txt: jq@1.6` versus `b.txt: jq@1.7` was decided by the disk — and sorting the read
/// order only makes the wrong answer deterministic (V.5).
///
/// The one exception is a dated line: while it is counting it beats an undated one, and
/// once its date passes it stops counting entirely (II.7 rule 6). That is what makes
/// "take the game away until the weekend" expressible without it reading as a
/// contradiction with a timer (V.37).
pub fn reconcile(
    key: &str,
    a: Declared,
    b: Declared,
    now: DateTime<Utc>,
) -> Result<Declared, GrammarError> {
    let (da, db) = (dating_of(&a.options, now), dating_of(&b.options, now));

    // A lapsed line has no opinion. It lingers in your file — LiNix must not rewrite what
    // you wrote (II.16) — but it does not participate.
    match (da, db) {
        (Dating::Lapsed, Dating::Lapsed) => return Ok(a),
        (Dating::Lapsed, _) => return Ok(b),
        (_, Dating::Lapsed) => return Ok(a),
        _ => {}
    }

    // Rule 6's exception to rule 5: while it is counting, a dated line beats an undated
    // one. This is the ONLY case where two contradicting declarations do not error.
    match (da.is_dated(), db.is_dated()) {
        (true, false) => return Ok(a),
        (false, true) => return Ok(b),
        _ => {}
    }

    if !contradicts(&a, &b) {
        // Identical, or one merely adds options the other did not set. Merge rather than
        // complain: declaring the same thing twice is not a disagreement.
        return Ok(merge(a, b));
    }

    Err(conflict_error(key, &a, &b))
}

/// Whether two declarations actually disagree, as opposed to merely both existing.
fn contradicts(a: &Declared, b: &Declared) -> bool {
    if a.present != b.present {
        return true;
    }
    // Same key set with different values is a disagreement; a key only one of them sets is
    // not.
    for (k, av) in a.options.iter() {
        if let Some(bv) = b.options.all(k).first() {
            if av.first() != Some(bv) {
                return true;
            }
        }
    }
    false
}

fn merge(mut a: Declared, b: Declared) -> Declared {
    for (k, vs) in b.options.iter() {
        if !a.options.contains(k) {
            for v in vs {
                a.options.insert(k, v.clone());
            }
        }
    }
    a
}

/// The error names BOTH files. An error that names one of them is an error that blames the
/// wrong file half the time.
fn conflict_error(key: &str, a: &Declared, b: &Declared) -> GrammarError {
    let describe = |d: &Declared| -> String {
        if !d.present {
            return format!("absent:{}", key);
        }
        let opts: Vec<String> = d
            .options
            .iter()
            .flat_map(|(k, vs)| vs.iter().map(move |v| format!("{}={}", k, v)))
            .collect();
        if opts.is_empty() {
            key.to_string()
        } else {
            format!("{}@{}", key, opts.join(","))
        }
    };

    GrammarError::new(
        a.origin.clone(),
        format!(
            "`{}` is declared two different ways, and both are active:\n  \
             {}  says  {}\n  {}  says  {}",
            key,
            a.origin,
            describe(a),
            b.origin,
            describe(b),
        ),
    )
    .with_hint(
        "LiNix will not pick one for you: whichever it chose would be right half the time \
         and silent about it. Change one of them, or put them in profiles that are not \
         active together.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dated::parse_absolute;

    fn now() -> DateTime<Utc> {
        parse_absolute("2026-07-16T12:00").unwrap()
    }

    fn decl(file: &str, line: usize, pairs: &[(&str, &str)]) -> Declared {
        let mut options = Options::default();
        for (k, v) in pairs {
            options.insert(*k, *v);
        }
        Declared {
            options,
            origin: Origin::new(file, line),
            present: true,
        }
    }

    fn absent(file: &str, line: usize) -> Declared {
        Declared {
            options: Options::default(),
            origin: Origin::new(file, line),
            present: false,
        }
    }

    #[test]
    fn two_contradicting_declarations_are_an_error() {
        // Two lines that disagree are an error, not a last-one-wins.
        let a = decl("modules/a.txt", 1, &[("version", "1.6")]);
        let b = decl("modules/b.txt", 3, &[("version", "1.7")]);
        let err = reconcile("apt:jq", a, b, now()).unwrap_err();
        assert!(err.what.contains("two different ways"), "{}", err);
    }

    #[test]
    fn the_error_names_both_files_and_lines() {
        // Part IV requires exactly this.
        let a = decl("modules/a.txt", 1, &[("version", "1.6")]);
        let b = decl("modules/b.txt", 3, &[("version", "1.7")]);
        let err = reconcile("apt:jq", a, b, now()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("modules/a.txt:1"), "{}", msg);
        assert!(msg.contains("modules/b.txt:3"), "{}", msg);
        assert!(msg.contains("1.6") && msg.contains("1.7"), "{}", msg);
    }

    #[test]
    fn present_and_absent_contradict() {
        let a = decl("modules/a.txt", 1, &[]);
        let b = absent("modules/bloat.txt", 2);
        assert!(reconcile("apt:libreoffice", a, b, now()).is_err());
    }

    #[test]
    fn declaring_the_same_thing_twice_is_not_a_disagreement() {
        let a = decl("modules/a.txt", 1, &[("version", "1.6")]);
        let b = decl("modules/b.txt", 3, &[("version", "1.6")]);
        assert!(reconcile("apt:jq", a, b, now()).is_ok());
    }

    #[test]
    fn options_only_one_side_sets_are_merged_not_fought_over() {
        let a = decl("modules/a.txt", 1, &[("version", "1.6")]);
        let b = decl("modules/b.txt", 3, &[("hold", "true")]);
        let out = reconcile("apt:jq", a, b, now()).unwrap();
        assert_eq!(out.options.one("version"), Some("1.6"));
        assert_eq!(out.options.one("hold"), Some("true"));
    }

    #[test]
    fn a_counting_dated_line_beats_an_undated_one() {
        // II.7 rule 6 — the ONLY exception to rule 5.
        let dated = decl("modules/temp.txt", 1, &[("expires", "2026-07-20T00:00")]);
        let plain = decl("modules/base.txt", 2, &[("version", "1.6")]);
        let out = reconcile("apt:jq", dated, plain, now()).unwrap();
        assert_eq!(out.origin.file.to_string_lossy(), "modules/temp.txt");
    }

    #[test]
    fn a_lapsed_line_stops_counting_and_the_undated_one_wins() {
        // The dated line lingers in the file; it just stops having an opinion.
        let lapsed = decl("modules/temp.txt", 1, &[("expires", "2026-01-01T00:00")]);
        let plain = decl("modules/base.txt", 2, &[("version", "1.6")]);
        let out = reconcile("apt:jq", lapsed, plain, now()).unwrap();
        assert_eq!(out.origin.file.to_string_lossy(), "modules/base.txt");
        assert_eq!(out.options.one("version"), Some("1.6"));
    }

    #[test]
    fn a_suspension_takes_the_game_away_until_the_weekend() {
        // V.37, end to end: `absent:` with a date beats the module that wants it, until
        // the date passes — then the module wins again and it comes back.
        let suspended = Declared {
            options: {
                let mut o = Options::default();
                o.insert("until", "2026-07-18T00:00");
                o
            },
            origin: Origin::new("modules/focus.txt", 1),
            present: false,
        };
        let wanted = decl("modules/gaming.txt", 5, &[]);

        let during = reconcile("apt:steam", suspended.clone(), wanted.clone(), now()).unwrap();
        assert!(!during.present, "the game is away during the week");

        let after = parse_absolute("2026-07-19T12:00").unwrap();
        let weekend = reconcile("apt:steam", suspended, wanted, after).unwrap();
        assert!(weekend.present, "and comes back at the weekend");
    }

    #[test]
    fn two_lapsed_lines_do_not_error_against_each_other() {
        let a = decl("modules/a.txt", 1, &[("expires", "2026-01-01")]);
        let b = decl("modules/b.txt", 2, &[("expires", "2026-02-01")]);
        assert!(reconcile("apt:jq", a, b, now()).is_ok());
    }
}
