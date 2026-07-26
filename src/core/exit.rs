//! What LiNix's exit codes mean (U21, 7m) — decided once, in one place.
//!
//! Ruled 2026-07-24. An exit code decided per command is a convention no script can rely on,
//! so this table is the whole vocabulary and every command draws from it:
//!
//! | code | meaning |
//! |---|---|
//! | 0 | converged — the machine matches what you declared |
//! | 1 | LiNix failed — it could not carry the command out |
//! | 2 | differences found — a read-only command that looked and found work to do |
//! | 3 | refused by the guard |
//!
//! **The separation that matters is 3.** A guard refusal is neither a crash nor a difference:
//! LiNix worked correctly and declined on purpose. A script that retries on failure must not
//! retry a refusal, and a script that acts on differences must not act on one — collapsing
//! them into `1` makes both mistakes possible and neither visible.
//!
//! **2 is only ever produced by a command that looked.** `sync` converges, so it exits 0 when
//! there is nothing left to do; `check` reports, so it exits 2 when it found something. The
//! same state, two codes, because the question differs.

/// The exit codes, as a closed set. Nothing constructs a bare integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The machine matches what was declared.
    Converged = 0,
    /// LiNix could not carry the command out.
    Failed = 1,
    /// A read-only command found work to do.
    Differences = 2,
    /// The guard refused. Not a failure and not a difference.
    Refused = 3,
}

impl Exit {
    pub fn code(self) -> i32 {
        self as i32
    }

    /// One line for `--help` and the readme, generated from the enum so the documentation
    /// cannot drift from what the binary returns.
    pub fn table() -> String {
        Exit::ALL
            .iter()
            .map(|e| format!("{} {}", e.code(), e.meaning()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub const ALL: [Exit; 4] = [
        Exit::Converged,
        Exit::Failed,
        Exit::Differences,
        Exit::Refused,
    ];

    pub fn meaning(self) -> &'static str {
        match self {
            Exit::Converged => "converged — the machine matches what you declared",
            Exit::Failed => "LiNix failed — it could not carry the command out",
            Exit::Differences => "differences found — something needs you",
            Exit::Refused => "refused by the guard",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbers are the interface. A script written against them must not be broken by an
    /// edit here, so they are asserted rather than assumed.
    #[test]
    fn the_codes_are_fixed() {
        assert_eq!(Exit::Converged.code(), 0);
        assert_eq!(Exit::Failed.code(), 1);
        assert_eq!(Exit::Differences.code(), 2);
        assert_eq!(Exit::Refused.code(), 3);
    }

    /// A guard refusal must be distinguishable from a crash and from a difference — that is
    /// the separation the whole table exists for.
    #[test]
    fn a_refusal_is_neither_a_failure_nor_a_difference() {
        assert_ne!(Exit::Refused.code(), Exit::Failed.code());
        assert_ne!(Exit::Refused.code(), Exit::Differences.code());
    }

    #[test]
    fn every_code_is_distinct_and_documented() {
        let mut seen = Vec::new();
        for e in Exit::ALL {
            assert!(!seen.contains(&e.code()), "{:?} reuses a code", e);
            seen.push(e.code());
            assert!(!e.meaning().is_empty());
        }
        let table = Exit::table();
        for e in Exit::ALL {
            assert!(
                table.contains(e.meaning()),
                "{:?} missing from the table",
                e
            );
        }
    }
}
