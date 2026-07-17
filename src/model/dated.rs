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
