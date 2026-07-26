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

    /// For input that never came from a file (a `linix install` argument).
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.origin, self.what)?;
        if let Some(h) = &self.hint {
            write!(f, "\n  {}", h)?;
        }
        Ok(())
    }
}

/// The whole message survives the crossing — origin and hint included. A grammar error that
/// reaches the user as "invalid config" has thrown away the two things it was built to say.
impl From<GrammarError> for crate::core::Error {
    fn from(e: GrammarError) -> Self {
        crate::core::Error::Config(e.to_string())
    }
}

impl std::error::Error for GrammarError {}

pub type Result<T> = std::result::Result<T, GrammarError>;
