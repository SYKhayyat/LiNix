//! `linix check` — the one command that looks at the machine (U9, 7i).
//!
//! There used to be ten commands answering "what is going on": `status`, `doctor`,
//! `unmanaged`, `absent`, `conflicts`, `audit` and the old `check`, each with its own
//! spelling, its own flags and its own idea of what a summary is. **Ruled 2026-07-24: they
//! collapse into one**, with a section per question, and the old names are deleted rather than
//! aliased (P2 — an alias is the second way to do one thing, kept alive).
//!
//! **`heal` survives, because it acts.** That is the whole dividing line: this command reads
//! and reports, and everything that repairs — including what `doctor --fix` used to do — is
//! `heal`'s. A command that both diagnoses and changes things is one you cannot run to find
//! out whether you want to change things.
//!
//! The default output is one line per section: a verdict, and the command that acts on it. A
//! section named on the command line prints its detail instead.

use std::fmt;

/// One question `check` can answer. Each is a section of the summary and a valid argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Does everything the active profiles reach parse and resolve?
    Config,
    /// What would `sync` change?
    Drift,
    /// What would `adopt` take?
    Unmanaged,
    /// Which `absent:` declarations are in force?
    Absent,
    /// Is the same package declared two different ways?
    Conflicts,
    /// Can each backend actually run?
    Health,
    /// Does anything managed have a known vulnerability?
    Security,
    /// Is any code the repo can run (an event hook) unapproved, and so silently dead? (II.12)
    Approvals,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Config,
        Section::Drift,
        Section::Unmanaged,
        Section::Absent,
        Section::Conflicts,
        Section::Health,
        Section::Security,
        Section::Approvals,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Section::Config => "config",
            Section::Drift => "drift",
            Section::Unmanaged => "unmanaged",
            Section::Absent => "absent",
            Section::Conflicts => "conflicts",
            Section::Health => "health",
            Section::Security => "security",
            Section::Approvals => "approvals",
        }
    }

    /// The one place the vocabulary is read from, so the parser and the error cannot disagree
    /// about what is legal.
    pub fn parse(name: &str) -> Option<Section> {
        let lower = name.trim().to_lowercase();
        Section::ALL.into_iter().find(|s| s.as_str() == lower)
    }

    pub fn vocabulary() -> String {
        Section::ALL
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for Section {
    /// `f.pad`, not `f.write_str`: the summary aligns sections in a column, and `write_str`
    /// silently ignores the width in `{:<11}` — the padding is requested and does not happen.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// What one section found: a verdict line and, when there is something to do, the command that
/// does it. Naming the next command is the point — a report you have to translate into an
/// action yourself has done the easy half (P8).
#[derive(Debug, Clone)]
pub struct Finding {
    pub section: Section,
    /// True when nothing needs attention. Decides the summary marker and the exit code.
    pub ok: bool,
    /// The one-line verdict, e.g. "3 to install, 1 to remove".
    pub summary: String,
    /// What to run about it. `None` when there is nothing to do.
    pub next: Option<String>,
}

impl Finding {
    pub fn ok(section: Section, summary: impl Into<String>) -> Finding {
        Finding {
            section,
            ok: true,
            summary: summary.into(),
            next: None,
        }
    }

    pub fn attention(
        section: Section,
        summary: impl Into<String>,
        next: impl Into<String>,
    ) -> Finding {
        Finding {
            section,
            ok: false,
            summary: summary.into(),
            next: Some(next.into()),
        }
    }

    /// The summary line: section, verdict, and the command to run about it.
    pub fn line(&self) -> String {
        let mark = if self.ok { "ok  " } else { "->  " };
        let head = format!("{}{:<11} {}", mark, self.section, self.summary);
        match &self.next {
            Some(next) => format!("{}\n{:19}run `{}`", head, "", next),
            None => head,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_parses_by_its_own_name() {
        for s in Section::ALL {
            assert_eq!(Section::parse(s.as_str()), Some(s));
        }
        // Case and surrounding space are a typing detail, not a different word.
        assert_eq!(Section::parse("  Health "), Some(Section::Health));
    }

    #[test]
    fn an_unknown_section_is_not_guessed() {
        for bad in ["", "status", "doctor", "everything", "vulns"] {
            assert_eq!(Section::parse(bad), None, "{} was accepted", bad);
        }
    }

    /// The vocabulary is what the error prints, so it must carry every section — otherwise a
    /// user is told to pick from a list that is missing the one they wanted.
    #[test]
    fn the_vocabulary_lists_every_section() {
        let v = Section::vocabulary();
        for s in Section::ALL {
            assert!(v.contains(s.as_str()), "{} missing from {}", s, v);
        }
    }

    /// A finding that needs attention names the command that acts on it. A report whose next
    /// step is the reader guessing has done the easy half (P8).
    #[test]
    fn a_finding_that_needs_attention_names_the_next_command() {
        let f = Finding::attention(Section::Drift, "3 to install", "linix sync");
        assert!(!f.ok);
        let line = f.line();
        assert!(line.contains("drift"), "{}", line);
        assert!(line.contains("3 to install"), "{}", line);
        assert!(line.contains("linix sync"), "{}", line);
    }

    /// The summary is a column, and `Display` must honour the width it is given — `write_str`
    /// ignores it, so the alignment silently does not happen.
    #[test]
    fn a_section_pads_to_the_width_it_is_given() {
        assert_eq!(format!("[{:<11}]", Section::Drift), "[drift      ]");
        assert_eq!(format!("[{:<11}]", Section::Unmanaged), "[unmanaged  ]");
    }

    #[test]
    fn a_clean_finding_offers_no_command() {
        let f = Finding::ok(Section::Conflicts, "none");
        assert!(f.ok);
        let line = f.line();
        assert!(line.contains("conflicts"), "{}", line);
        assert!(!line.contains("run `"), "a clean section suggested work: {}", line);
    }
}
