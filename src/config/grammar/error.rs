use std::fmt;
use std::path::{Path, PathBuf};

/// Where a line came from. Every grammar error carries one: an error that cannot name the
/// file and line is an error nobody can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub file: PathBuf,
    /// 1-based, as an editor counts.
    pub line: usize,
}

impl Origin {
    pub fn new(file: impl Into<PathBuf>, line: usize) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }

    /// For input that never came from a file (a `shall install` argument).
    pub fn argument() -> Self {
        Self {
            file: PathBuf::from("<argument>"),
            line: 0,
        }
    }
}

impl std::str::FromStr for Origin {
    type Err = ();

    /// The inverse of [`Display`], kept beside it because the round trip crosses seams where
    /// everything is a string — `__source` on a spec, `__gated_by` on a gate — and the two
    /// halves drift the moment they are apart. A Windows path's drive letter is why the split
    /// is from the right.
    fn from_str(s: &str) -> std::result::Result<Self, ()> {
        // No line number is the `argument()` shape, not a malformed one.
        match s
            .rsplit_once(':')
            .and_then(|(f, l)| Some((f, l.parse::<usize>().ok()?)))
        {
            Some((file, line)) => Ok(Origin::new(file, line)),
            None => Ok(Origin::new(s, 0)),
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.file.display())
        } else {
            write!(f, "{}:{}", self.file.display(), self.line)
        }
    }
}

/// A grammar violation. `what` states what was wrong; `hint` says what to do instead.
///
/// The split exists because the hint is the part that teaches the rule, and II.2 requires
/// specific hints for specific mistakes ("commas need the block form") rather than a
/// generic parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarError {
    pub origin: Origin,
    pub what: String,
    pub hint: Option<String>,
}

impl GrammarError {
    pub fn new(origin: Origin, what: impl Into<String>) -> Self {
        Self {
            origin,
            what: what.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn at(file: &Path, line: usize, what: impl Into<String>) -> Self {
        Self::new(Origin::new(file, line), what)
    }
}

impl fmt::Display for GrammarError {
    /// Both halves are drawn through [`printable`](crate::core::validator::printable), because a
    /// refusal quotes the line it refused and the line is untrusted text. W38 gave the character
    /// validator this rule and left the grammar's own refusals with the raw bytes: a module saved
    /// by Notepad begins with a byte-order mark, and the refusal then reads
    /// `` `cargo` is not a backend Shall uses — add `cargo` to your priority file ``, naming two
    /// strings that look identical and are not.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use crate::core::validator::printable;
        write!(f, "{}: {}", self.origin, printable(&self.what))?;
        if let Some(h) = &self.hint {
            write!(f, "\n  {}", printable(h))?;
        }
        Ok(())
    }
}

impl std::error::Error for GrammarError {}

pub type Result<T> = std::result::Result<T, GrammarError>;

/// The whole message survives the crossing — origin and hint included. A grammar error that
/// reaches the user as "invalid config" has thrown away the two things it was built to say.
impl From<GrammarError> for crate::core::Error {
    fn from(e: GrammarError) -> Self {
        crate::core::Error::Config(e.to_string())
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn an_invisible_character_in_a_refusal_is_named_not_drawn() {
        // U+FEFF is what Windows Notepad writes at the top of a UTF-8 file, and U+202E is the
        // trojan-source override: the first makes a refusal unreadable, the second makes it
        // reversible.
        for (raw, named) in [('\u{feff}', "<U+FEFF>"), ('\u{202e}', "<U+202E>")] {
            let e = GrammarError::new(Origin::new("modules/dev.txt", 1), format!("`{raw}cargo`"))
                .with_hint(format!("add `{raw}cargo` to your `priority` file"));
            let rendered = e.to_string();
            assert!(
                rendered.contains(named),
                "the codepoint was not named: {rendered:?}"
            );
            assert!(
                !rendered.contains(raw),
                "the character was drawn at the terminal: {rendered:?}"
            );
        }
    }
}
