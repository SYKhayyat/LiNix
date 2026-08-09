//! **One audit for every exemption table.**
//!
//! A scanning gate in this suite has the same four parts everywhere: a predicate that walks the
//! tree, a `const` table of sites excused from it with a sentence each, and four assertions —
//!
//! 1. the walk read enough files to have found anything (**the floor**),
//! 2. the predicate still matches something (**not vacuous**),
//! 3. every site the walk found is either clean or in the table (**unexplained**),
//! 4. every entry in the table still names a site the walk finds, and still carries a sentence
//!    long enough to be one (**stale**).
//!
//! Nine files wrote those four out by hand, and the copies had drifted exactly where you would
//! expect: **assertion 1 was missing from three of them**, which is finding 4 — a scan whose
//! predicate had stopped matching passed by finding nothing, three times over, and read as a
//! green gate for months.
//!
//! The helper owns the four. What a site *cannot* delegate — "and the exemption must not
//! contradict the row three lines below it", "and the proof string must name a line that is
//! actually in the file" — stays written out beside the table it is about, because that
//! assertion is the site's own knowledge and there is nothing to share.
//!
//! **The floor is not optional.** [`Ledger::of`] starts at zero and [`audit`](Ledger::audit)
//! panics on zero rather than skipping the check, because "I could not think of a floor" is how
//! the three vacuous gates were written in the first place.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::fmt::Write as _;

/// One row of an exemption table: the site it excuses, and the sentence excusing it.
#[derive(Clone, Copy)]
pub struct Entry<'a> {
    pub site: &'a str,
    pub why: &'a str,
}

impl<'a> From<&'a (&'a str, &'a str)> for Entry<'a> {
    fn from((site, why): &'a (&'a str, &'a str)) -> Self {
        Entry { site, why }
    }
}

/// A scanning gate's exemption table, and the four assertions that keep it honest.
pub struct Ledger<'a> {
    subject: &'a str,
    table: &'a str,
    entries: Vec<Entry<'a>>,
    floor: usize,
    min_reason: usize,
    remedy: &'a str,
    detail: Option<Box<dyn Fn(&str) -> Option<String> + 'a>>,
}

impl<'a> Ledger<'a> {
    /// `subject` names what the predicate looks for, in a sentence fragment that reads after
    /// "these sites are": *"a rename into place"*, *"an upgrade-all verb"*. `table` is the
    /// `const`'s own identifier, so a failure tells the reader which list to edit.
    pub fn of(subject: &'a str, table: &'a str) -> Self {
        Ledger {
            subject,
            table,
            entries: Vec::new(),
            floor: 0,
            min_reason: 40,
            remedy: "",
            detail: None,
        }
    }

    pub fn exempting(mut self, rows: impl IntoIterator<Item = Entry<'a>>) -> Self {
        self.entries.extend(rows);
        self
    }

    /// The `&[(&str, &str)]` spelling, which is most of them.
    pub fn pairs(self, rows: &'a [(&'a str, &'a str)]) -> Self {
        self.exempting(rows.iter().map(Entry::from))
    }

    /// How many units the walk must have read before its answer means anything. Files, modules,
    /// registrars — whatever the walk counts.
    pub fn scanning_at_least(mut self, n: usize) -> Self {
        self.floor = n;
        self
    }

    /// Default 40. Raise it where the reason has to carry a mechanism rather than a label.
    pub fn reason_of_at_least(mut self, n: usize) -> Self {
        self.min_reason = n;
        self
    }

    /// What to do about an unexplained site, appended to that failure only.
    pub fn remedy(mut self, text: &'a str) -> Self {
        self.remedy = text;
        self
    }

    /// What the walk actually saw at a site, printed under its name when it turns up
    /// unexplained. A file path alone tells a reader where to start looking; the line that
    /// matched tells them what they are looking for.
    pub fn detailing(mut self, f: impl Fn(&str) -> Option<String> + 'a) -> Self {
        self.detail = Some(Box::new(f));
        self
    }

    /// The four assertions. `scanned` is what the walk read; `found` is what it matched.
    #[track_caller]
    pub fn audit(&self, scanned: usize, found: &BTreeSet<String>) {
        assert!(
            self.floor > 0,
            "{}'s ledger has no floor. Without one, a predicate that has stopped matching \
             passes by finding nothing — which is what three of the gates this helper replaced \
             were doing.",
            self.table
        );
        assert!(
            scanned >= self.floor,
            "the walk for {} read only {scanned} unit(s), under its floor of {}; it is looking \
             in the wrong place, so neither its findings nor its silence mean anything",
            self.subject,
            self.floor
        );
        assert!(
            !found.is_empty(),
            "the walk for {} matched nothing at all across {scanned} unit(s). The predicate has \
             stopped recognising its subject; {} is now excusing sites that no longer exist.",
            self.subject,
            self.table
        );

        let excused: BTreeSet<&str> = self.entries.iter().map(|e| e.site).collect();

        let unexplained: Vec<&str> = found
            .iter()
            .map(String::as_str)
            .filter(|s| !excused.contains(s))
            .collect();
        if !unexplained.is_empty() {
            let mut msg = String::new();
            let _ = write!(
                msg,
                "these sites are {} and are not in {}: {unexplained:?}",
                self.subject, self.table
            );
            if let Some(detail) = &self.detail {
                for site in &unexplained {
                    if let Some(text) = detail(site) {
                        let _ = write!(msg, "\n    {site}\n        {text}");
                    }
                }
            }
            if !self.remedy.is_empty() {
                let _ = write!(msg, "\n{}", self.remedy);
            }
            let _ = write!(
                msg,
                "\nOr add each to {} with the sentence explaining why it must be that way.",
                self.table
            );
            panic!("{msg}");
        }

        let stale: Vec<&str> = excused
            .iter()
            .copied()
            .filter(|s| !found.contains(*s))
            .collect();
        assert!(
            stale.is_empty(),
            "{} excuses {stale:?}, which the walk no longer finds to be {}. Delete the \
             entries: a permission granted to nothing still reads as one guarding something.",
            self.table,
            self.subject
        );

        self.reasons_say_something();
    }

    /// Assertion 4's second half, on its own — for a site whose staleness check is domain
    /// knowledge the helper cannot have.
    #[track_caller]
    pub fn reasons_say_something(&self) {
        for e in &self.entries {
            assert!(
                e.why.chars().count() >= self.min_reason,
                "{}'s entry for `{}` has no reason worth the name ({} chars, floor {}): {:?}\n\
                 The reason is the exemption. Without one the row is a site nobody looked at, \
                 wearing the costume of one somebody did.",
                self.table,
                e.site,
                e.why.chars().count(),
                self.min_reason,
                e.why
            );
        }
    }

    /// The sites the walk found that nothing excuses.
    pub fn unexplained_in(&self, found: &BTreeSet<String>) -> Vec<String> {
        let excused: BTreeSet<&str> = self.entries.iter().map(|e| e.site).collect();
        found
            .iter()
            .filter(|s| !excused.contains(s.as_str()))
            .cloned()
            .collect()
    }
}
