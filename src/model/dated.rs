use crate::config::grammar::{Options, PackageDecl};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};

/// Whether a dated line is counting right now (SPEC II.7 rule 6).
///
/// Two keys, mirror images:
/// - `@expires` on a present line: present now, absent after.
/// - `@until` on an `absent:` line: absent now, present after.
///
/// Both are absolute (V.38). A duration cannot work in a file, because the machine reading
/// it next week has no idea when you wrote it — which is exactly why `@lease=2h` was inert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dating {
    /// No date. Counts forever.
    Undated,
    /// Dated and still in force.
    Counting,
    /// Dated and its date has passed. **Stops counting** — it does not flip to the
    /// opposite meaning, it stops having an opinion.
    Lapsed,
}

impl Dating {
    pub fn counts(self) -> bool {
        !matches!(self, Dating::Lapsed)
    }

    pub fn is_dated(self) -> bool {
        !matches!(self, Dating::Undated)
    }
}

/// Classify a declaration's dating at `now`.
///
/// An unparseable date is `Undated` here rather than an error: the grammar already refused
/// it at parse time (`@expires=2h`), so anything reaching this is well-formed.
pub fn dating_of(options: &Options, now: DateTime<Utc>) -> Dating {
    for key in ["expires", "until"] {
        if let Some(raw) = options.one(key) {
            let Some(when) = parse_absolute(raw) else {
                continue;
            };
            return if now >= when {
                Dating::Lapsed
            } else {
                Dating::Counting
            };
        }
    }
    Dating::Undated
}

/// Parse the absolute forms II.2 accepts. Kept beside `dating_of` so the grammar's
/// validation and this reader cannot drift apart into accepting different sets.
pub fn parse_absolute(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    if let Ok(d) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0)?));
    }
    None
}

/// Whether `decl` still has an opinion at `now`.
pub fn still_counts(decl: &PackageDecl, now: DateTime<Utc>) -> bool {
    dating_of(&decl.options, now).counts()
}

/// Turn a duration someone typed into the absolute time a file can hold (V.38).
///
/// `--temp 2h` is a fine thing to type and an impossible thing to store: a file saying "2
/// hours" cannot be read, because the machine reading it does not know when you wrote it,
/// and it would mean something different every time it was read. The command line knows
/// `now`, so this is where the conversion belongs — and the line it writes says exactly
/// when, forever.
///
/// Accepts `s`, `m`, `h`, `d`, `w`.
pub fn absolute_after(now: DateTime<Utc>, duration: &str) -> Option<String> {
    let raw = duration.trim();
    let unit = raw.chars().last()?;
    let value: i64 = raw[..raw.len() - unit.len_utf8()].parse().ok()?;
    if value < 0 {
        return None;
    }
    let delta = match unit {
        's' => chrono::Duration::try_seconds(value),
        'm' => chrono::Duration::try_minutes(value),
        'h' => chrono::Duration::try_hours(value),
        'd' => chrono::Duration::try_days(value),
        'w' => chrono::Duration::try_weeks(value),
        _ => return None,
    }?;
    let at = now.checked_add_signed(delta)?;
    // The format `parse_absolute` reads back, to the minute: a lease is not a stopwatch,
    // and seconds in a file you read next month are noise.
    Some(at.format("%Y-%m-%dT%H:%M").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        parse_absolute(s).unwrap()
    }

    fn opts(pairs: &[(&str, &str)]) -> Options {
        let mut o = Options::default();
        for (k, v) in pairs {
            o.insert(*k, *v);
        }
        o
    }

    #[test]
    fn an_undated_line_counts_forever() {
        assert_eq!(dating_of(&opts(&[]), at("2030-01-01")), Dating::Undated);
        assert!(dating_of(&opts(&[]), at("2030-01-01")).counts());
    }

    #[test]
    fn a_dated_line_counts_until_its_date() {
        let o = opts(&[("expires", "2026-07-17T14:00")]);
        assert_eq!(dating_of(&o, at("2026-07-17T13:59")), Dating::Counting);
        assert!(dating_of(&o, at("2026-07-17T13:59")).counts());
    }

    #[test]
    fn a_dated_line_stops_counting_once_its_date_passes() {
        // II.7 rule 6. It stops having an opinion; it does not acquire the opposite one.
        let o = opts(&[("expires", "2026-07-17T14:00")]);
        assert_eq!(dating_of(&o, at("2026-07-17T14:01")), Dating::Lapsed);
        assert!(!dating_of(&o, at("2026-07-17T14:01")).counts());
    }

    #[test]
    fn the_moment_itself_has_passed() {
        let o = opts(&[("expires", "2026-07-17T14:00")]);
        assert_eq!(dating_of(&o, at("2026-07-17T14:00")), Dating::Lapsed);
    }

    #[test]
    fn until_is_the_mirror_of_expires() {
        // `absent:apt:steam@until=...` — absent now, present after.
        let o = opts(&[("until", "2026-07-20T00:00")]);
        assert_eq!(dating_of(&o, at("2026-07-19")), Dating::Counting);
        assert_eq!(dating_of(&o, at("2026-07-21")), Dating::Lapsed);
    }

    #[test]
    fn every_absolute_form_the_grammar_accepts_is_readable_here() {
        // The grammar's validator and this reader must accept the same set, or a line the
        // grammar allows silently becomes undated and counts forever.
        assert!(parse_absolute("2026-07-17T14:00:00Z").is_some());
        assert!(parse_absolute("2026-07-17T14:00:00").is_some());
        assert!(parse_absolute("2026-07-17T14:00").is_some());
        assert!(parse_absolute("2026-07-17").is_some());
        assert!(parse_absolute("2h").is_none());
    }
}

#[cfg(test)]
mod duration_tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        parse_absolute("2026-07-17T12:00").unwrap()
    }

    #[test]
    fn a_duration_becomes_the_moment_it_runs_out() {
        // V.38: the file cannot hold "2 hours" — it would mean something different every
        // time it was read.
        assert_eq!(absolute_after(now(), "2h").unwrap(), "2026-07-17T14:00");
        assert_eq!(absolute_after(now(), "30m").unwrap(), "2026-07-17T12:30");
        assert_eq!(absolute_after(now(), "7d").unwrap(), "2026-07-24T12:00");
        assert_eq!(absolute_after(now(), "1w").unwrap(), "2026-07-24T12:00");
    }

    #[test]
    fn what_it_writes_is_what_the_resolver_reads_back() {
        // The two halves of the same fact: if these ever disagree, a `--temp` install is
        // either permanent or already expired.
        let written = absolute_after(now(), "2h").unwrap();
        let read = parse_absolute(&written).expect("must parse back");
        assert_eq!(read, parse_absolute("2026-07-17T14:00").unwrap());
    }

    #[test]
    fn nonsense_is_refused_rather_than_guessed() {
        for bad in ["", "2", "h", "2x", "-1h", "two hours", "2 h"] {
            assert!(absolute_after(now(), bad).is_none(), "{} must be refused", bad);
        }
    }
}
