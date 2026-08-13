//! What Shall's exit codes mean (U21, 7m) — decided once, in one place.
//!
//! Ruled 2026-07-24. An exit code decided per command is a convention no script can rely on,
//! so this table is the whole vocabulary and every command draws from it:
//!
//! | code | meaning |
//! |---|---|
//! | 0 | converged — the machine matches what you declared |
//! | 1 | Shall failed — it could not carry the command out |
//! | 2 | differences found — something needs you |
//! | 3 | refused by the guard |
//!
//! **The separation that matters is 3.** A guard refusal is neither a crash nor a difference:
//! Shall worked correctly and declined on purpose. A script that retries on failure must not
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
    /// Shall could not carry the command out.
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

    // `Exit::table()` used to live here: a generator whose doc said it existed "so the
    // documentation cannot drift from what the binary returns", called by nothing but its own
    // unit test. Nothing was generated from it, so the hand-written table at the top of this
    // file drifted anyway — row 2 said something the binary does not return, eight lines away.
    // It is deleted rather than wired into `--help`, which nobody asked for. What binds the two
    // copies now is a test that reads *this file*: `the_exit_table_is_generated_not_retyped`.

    pub const ALL: [Exit; 4] = [
        Exit::Converged,
        Exit::Failed,
        Exit::Differences,
        Exit::Refused,
    ];

    pub fn meaning(self) -> &'static str {
        match self {
            Exit::Converged => "converged — the machine matches what you declared",
            Exit::Failed => "Shall failed — it could not carry the command out",
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
    fn every_code_is_distinct() {
        let mut seen = Vec::new();
        for e in Exit::ALL {
            assert!(!seen.contains(&e.code()), "{:?} reuses a code", e);
            seen.push(e.code());
        }
    }

    /// **The meanings, stated independently of the code that produces them.**
    ///
    /// This assertion used to be `Exit::table().contains(e.meaning())` — and `table()` was
    /// *built by calling* `meaning()`, so both sides came from one source and the comparison
    /// could not fail. Measured rather than argued: `cargo mutants` replaced every meaning with
    /// `"xyzzy"`, and with `""`, and the suite stayed green through both.
    ///
    /// An assertion about a string has to name the string. That is not the transcription
    /// problem F7 was — a gate reading a copy of a *list* cannot see the list grow — because
    /// there is nothing here to grow: `ALL` is exhaustive and `the_codes_are_fixed` pins its
    /// length. What a reader sees is bound to these strings separately, by a test that reads
    /// this file's own doc-comment.
    #[test]
    fn the_meanings_are_what_a_script_was_promised() {
        assert_eq!(
            Exit::Converged.meaning(),
            "converged — the machine matches what you declared"
        );
        assert_eq!(
            Exit::Failed.meaning(),
            "Shall failed — it could not carry the command out"
        );
        assert_eq!(
            Exit::Differences.meaning(),
            "differences found — something needs you"
        );
        assert_eq!(Exit::Refused.meaning(), "refused by the guard");
    }
}
