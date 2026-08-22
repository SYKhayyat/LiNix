use thiserror::Error;

/// Whether running the same operation again could produce a different answer.
///
/// The retry loop must not decide this by reading the message text: a held `dpkg` lock and a
/// package name that does not exist arrive as the same `CommandFailed` string, and three
/// backoff rounds against the second only delay the report. `Unknown` retries, because that
/// is what every failure did before this distinction existed and a wrong guess in that
/// direction costs time rather than correctness.
///
/// **The variants are declared least-optimistic-last, and that order is the semantics.** `Ord`
/// is derived from it, so "the worse of two verdicts" is `a.max(b)` rather than a hand-written
/// comparison over a rank table. The rank table was the first version; its `>` had an
/// equivalent mutant that no test could kill, because two ranks are equal exactly when the two
/// variants are the same one and both branches then return the same value. A property carried
/// by the type has nothing to compare and nothing to get wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Retryability {
    /// A lock someone else holds, a dropped connection, a mirror that timed out.
    Transient,
    /// It was called transient, retried, and came back the same. The claim was tested and
    /// failed — so a further retry is not worth suggesting, but "this can never work" is more
    /// than was measured: the cause may be a broken `wget` on the PATH, fixable tomorrow.
    ///
    /// Kept apart from `Unknown` because the two lead to different sentences. `Unknown` means
    /// nobody looked; this means somebody did.
    Exhausted,
    /// Nothing classified it.
    Unknown,
    /// The same command will fail the same way. Retrying cannot help.
    Permanent,
}

impl Retryability {
    /// The verdict for a run that carried on past several failures.
    ///
    /// One question is being answered — *will running this same command again succeed?* — so
    /// the least optimistic answer wins: `Permanent` > `Unknown` > `Exhausted` > `Transient`,
    /// which is the order the variants are declared in. `Unknown` outranks `Exhausted` and
    /// `Transient` because a failure nobody classified may yet be a permanent one, and calling
    /// the run retryable on the strength of the classified half is a promise about the half
    /// nobody looked at.
    pub fn and_also(self, other: Self) -> Self {
        self.max(other)
    }
}

/// Every variant carries a `String` rather than the source error so the enum stays
/// `Clone + Send`: a parallel transaction fans one failure out to several waiting tasks.
#[derive(Debug, Error, Clone)]
pub enum Error {
    /// The guard said no. Its own variant, not `Other`, because U21 gives a refusal exit code
    /// 3: a script that retries on failure must not retry a refusal, and it cannot tell them
    /// apart if both arrive as the same error.
    #[error("{0}")]
    Refused(String),

    /// A read-only command looked and found work to do (U21, exit code 2). Not an error in the
    /// ordinary sense — it is the answer — but it travels the error channel so that every
    /// command's result stays one type.
    #[error("{0}")]
    Differences(String),

    #[error("Backend '{0}' not found or unsupported on this platform")]
    BackendNotFound(String),

    /// A command ran and did not succeed. `retry` is filled in by whoever classified the
    /// exit — the backend's [`ExitPolicy`](crate::core::ExitPolicy) for a spawned manager,
    /// [`Retryability::Unknown`] for everyone else — so the retry loop never has to read the
    /// message back to decide what to do.
    /// The message says which command and how it failed, so it is printed as itself. The old
    /// "Command execution failed: " prefix produced "Command execution failed: `apt` failed
    /// (exit 100): …", which says it twice and pushes the manager's own words off the line.
    #[error("{message}")]
    CommandFailed {
        message: String,
        retry: Retryability,
        /// The manager's own words say the name it was handed is not there.
        ///
        /// Deliberately not derived from `retry`. `Permanent` answers *would another attempt
        /// differ?* and this answers *does the name exist?*, and reading the first as the
        /// second is N-1: `install` withdrew a wedged line for the two managers whose
        /// failure happened to be classified `Permanent` and left it in for every manager
        /// that had no policy — while helm's `plugin already exists` is `Permanent` about a
        /// name that plainly exists and must never withdraw anything.
        absent_name: bool,
    },

    #[error("I/O error: {0}")]
    Io(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("HTTP request failed: {0}")]
    Http(String),

    #[error("JSON processing error: {0}")]
    Json(String),

    #[error("TOML processing error: {0}")]
    Toml(String),

    /// A backend that resolves names itself — a git host, an index, an API — looked this one
    /// up and it is not there. It carries the name rather than describing it, because the
    /// caller that has to withdraw the declaration must not go looking for a package name in
    /// a sentence: `pixi` wraps its output mid-name, and the two managers whose prose did
    /// name the package are exactly the two where E1 looked fixed.
    #[error("Package '{name}' was not found: {message}")]
    NoSuchPackage { name: String, message: String },

    /// A manager ran, succeeded, and printed something its parser does not recognise.
    ///
    /// Its own variant because the alternative reading is the dangerous one and has to be
    /// unavailable: a listing nobody could parse must never arrive as `Ok(vec![])`, which the
    /// planner reads as an empty machine and answers by planning every declaration as a fresh
    /// install and dropping every drift removal. Arriving as an error instead puts the backend
    /// *outside* `installed_sets`, where `is_installed` answers true and removals stay
    /// scheduled — the branch that fails safe.
    #[error("{0}")]
    Unreadable(String),

    #[error("Operation was cancelled by the user")]
    Cancelled,

    #[error("Insufficient permissions for this operation: {0}")]
    Permission(String),

    #[error("API rate limit: {0}")]
    RateLimit(String),

    #[error("Scripting engine error (Lua/Rhai): {0}")]
    LuaScript(String),

    #[error("Parallel Transaction failed: {0}")]
    Transaction(String),

    #[error("Platform or architecture not supported: {0}")]
    UnsupportedPlatform(String),

    #[error("Atomic file persistence error: {0}")]
    Persist(String),

    #[error("Snapshot provider failure: {0}")]
    Snapshot(String),

    #[error("Journal/WAL failure: {0}")]
    Journal(String),

    #[error("Cron scheduling failure: {0}")]
    Cron(String),

    #[error("Interactive UI error (Dialoguer): {0}")]
    Dialoguer(String),

    /// The requested operation is not supported by this backend (e.g. a backend
    /// with no notion of orphan cleanup). This is a benign, honest "skip" — callers
    /// should treat it differently from a real failure rather than pretending success.
    #[error("Operation not supported by backend '{0}'")]
    Unsupported(String),

    /// No backend in `priority` claims this name, so the line naming it can never be
    /// satisfied by retrying. Kept apart from `Config` so `install` can tell "this name is
    /// wrong" from "the sync failed" — it withdraws the line it just wrote only for the
    /// first, and a dropped network must never be read as a reason to delete intent.
    /// The payload is the rendered grammar error; `name` is the name that resolved to
    /// nothing.
    #[error("Configuration error: {message}")]
    Unresolvable { name: String, message: String },

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// A command failure nobody has classified. The honest default: the retry loop treats it
    /// exactly as it treated every failure before classification existed.
    pub fn command_failed(message: impl Into<String>) -> Self {
        Error::CommandFailed {
            message: message.into(),
            retry: Retryability::Unknown,
            absent_name: false,
        }
    }

    /// The same failure with something added to what it says.
    ///
    /// **Appending to an error must not re-classify it.** Written as
    /// `Error::Transaction(format!("{e}{note}"))`, adding a sentence turns whatever the error
    /// was into an `Unknown` one — so the pin advice, which exists to explain a version nothing
    /// satisfies, was destroying the `Permanent` verdict of exactly the failures it fired on:
    /// three backoff rounds against a pin that cannot be met, `shall-failure-class: unknown`,
    /// and a user told nothing classified it.
    ///
    /// The variant is preserved, not just the class, because callers match on it — `Refused`
    /// carries exit code 3 and `Differences` carries 2, and neither survives being rebuilt as
    /// something else. Exhaustive on purpose: a variant added later cannot silently take the
    /// wrong arm.
    #[must_use]
    pub fn with_note(self, note: impl AsRef<str>) -> Self {
        let note = note.as_ref();
        // Appended to the payload rather than to the rendered error, so a variant whose
        // `#[error(…)]` adds a prefix keeps the note inside its own sentence.
        macro_rules! plus {
            ($v:expr, $s:expr) => {
                $v(format!("{}{}", $s, note))
            };
        }
        match self {
            Error::Refused(s) => plus!(Error::Refused, s),
            Error::Differences(s) => plus!(Error::Differences, s),
            Error::BackendNotFound(s) => plus!(Error::BackendNotFound, s),
            Error::Io(s) => plus!(Error::Io, s),
            Error::Config(s) => plus!(Error::Config, s),
            Error::Validation(s) => plus!(Error::Validation, s),
            Error::Http(s) => plus!(Error::Http, s),
            Error::Json(s) => plus!(Error::Json, s),
            Error::Toml(s) => plus!(Error::Toml, s),
            Error::Unreadable(s) => plus!(Error::Unreadable, s),
            Error::Permission(s) => plus!(Error::Permission, s),
            Error::RateLimit(s) => plus!(Error::RateLimit, s),
            Error::LuaScript(s) => plus!(Error::LuaScript, s),
            Error::Transaction(s) => plus!(Error::Transaction, s),
            Error::UnsupportedPlatform(s) => plus!(Error::UnsupportedPlatform, s),
            Error::Persist(s) => plus!(Error::Persist, s),
            Error::Snapshot(s) => plus!(Error::Snapshot, s),
            Error::Journal(s) => plus!(Error::Journal, s),
            Error::Cron(s) => plus!(Error::Cron, s),
            Error::Dialoguer(s) => plus!(Error::Dialoguer, s),
            Error::Unsupported(s) => plus!(Error::Unsupported, s),
            Error::Other(s) => plus!(Error::Other, s),
            Error::CommandFailed {
                message,
                retry,
                absent_name,
            } => Error::CommandFailed {
                message: format!("{message}{note}"),
                retry,
                absent_name,
            },
            Error::NoSuchPackage { name, message } => Error::NoSuchPackage {
                name,
                message: format!("{message}{note}"),
            },
            Error::Unresolvable { name, message } => Error::Unresolvable {
                name,
                message: format!("{message}{note}"),
            },
            // Nothing to append to, and nothing worth saying: advice about how to fix an
            // operation nobody performed would be the only sentence a cancelled run printed.
            Error::Cancelled => Error::Cancelled,
        }
    }

    /// A command failure carrying a verdict somebody already reached about it.
    ///
    /// For the summary a run raises *after* carrying on past failures it classified one by one.
    /// Built with [`Error::command_failed`] instead, that summary answers `unknown` to a run
    /// whose every failure was named — and `unknown` is what both readers of the class act on:
    /// the harness retries it as a possible defect, and the user is told nothing classified a
    /// failure Shall classified twice.
    pub fn command_failed_classified(message: impl Into<String>, retry: Retryability) -> Self {
        Error::CommandFailed {
            message: message.into(),
            retry,
            absent_name: false,
        }
    }

    /// A command failure that a fresh attempt cannot fix — a binary that is not there, a
    /// version pin nothing satisfies, a source that cannot be verified. Permanent says only
    /// that; use [`Error::command_failed_absent`] when the manager said the *name* is absent.
    pub fn command_failed_permanently(message: impl Into<String>) -> Self {
        Error::CommandFailed {
            message: message.into(),
            retry: Retryability::Permanent,
            absent_name: false,
        }
    }

    /// Say which declaration a failure happened for, without changing what the failure *is*.
    ///
    /// The message gains a line; `retry` and `absent_name` are untouched, and that is the whole
    /// point of doing it this way. Those two are what every caller downstream reads to decide
    /// whether to retry and whether to withdraw the line, so a wrapper — `Error::Other(format!(
    /// "while doing X: {e}"))` — turns a withdrawable line into a permanent wedge. This edits
    /// the sentence and nothing else.
    ///
    /// `install X` converges the whole configuration, which is the model working: Shall is
    /// declarative and your file is the truth. But it means a line you have never looked at can
    /// stop the install you just typed, and before this the error named the *command* — `sc`
    /// failed (exit 1056) — with nothing to say which declaration ran it or where that line
    /// lives (`Q34`).
    pub fn about_declaration(self, key: &str, origin: Option<&str>) -> Self {
        let where_from = match origin {
            Some(o) => format!(" ({})", o),
            None => String::new(),
        };
        let note = format!(
            "
    while applying `{}`{}.",
            key, where_from
        );
        match self {
            Error::CommandFailed {
                message,
                retry,
                absent_name,
            } => Error::CommandFailed {
                message: format!("{}{}", message, note),
                retry,
                absent_name,
            },
            Error::Refused(m) => Error::Refused(format!("{}{}", m, note)),
            Error::Validation(m) => Error::Validation(format!("{}{}", m, note)),
            // Anything else already says what it is in its own words, and the variants above
            // are the ones a backend produces. Left alone rather than stringified into `Other`,
            // which would cost the caller the variant it reads.
            other => other,
        }
    }

    /// A command failure whose output says the name is not there. The one command failure
    /// that withdraws a declaration.
    pub fn command_failed_absent(message: impl Into<String>) -> Self {
        Error::CommandFailed {
            message: message.into(),
            retry: Retryability::Permanent,
            absent_name: true,
        }
    }

    /// Whether this failure says the name it was given does not exist anywhere it looked.
    ///
    /// The single predicate every caller reads. Two roads reach it — a manager's declared
    /// phrasings via [`ExitPolicy`](crate::core::ExitPolicy), and a name-resolving backend
    /// saying so directly — because only the second kind knows the name it looked up. What
    /// matters is that neither road is *prose the caller parses*, which is what N-1 was.
    pub fn says_a_name_is_absent(&self) -> bool {
        match self {
            Error::CommandFailed { absent_name, .. } => *absent_name,
            Error::NoSuchPackage { .. } | Error::Unresolvable { .. } => true,
            _ => false,
        }
    }

    /// The name this failure says is absent, when the failure knows which one it was.
    pub fn absent_name(&self) -> Option<&str> {
        match self {
            Error::NoSuchPackage { name, .. } | Error::Unresolvable { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Whether the operation that produced this error is worth attempting again.
    pub fn retryability(&self) -> Retryability {
        match self {
            Error::CommandFailed { retry, .. } => *retry,

            // Nothing about the machine changes between attempts for any of these: the name
            // is wrong, the file is wrong, the platform cannot do it, or Shall itself said no.
            Error::BackendNotFound(_)
            | Error::NoSuchPackage { .. }
            | Error::Unresolvable { .. }
            | Error::Config(_)
            | Error::Validation(_)
            | Error::Toml(_)
            | Error::Json(_)
            | Error::Unsupported(_)
            | Error::UnsupportedPlatform(_)
            | Error::LuaScript(_)
            | Error::Refused(_)
            | Error::Differences(_)
            // A manager will print the same bytes on the next attempt, and the parser will
            // fail to recognise them the same way. Retrying an output-format change is time
            // spent proving the format did not change back.
            | Error::Unreadable(_)
            | Error::Cancelled => Retryability::Permanent,

            // A sudo timestamp does not warm up on its own inside a backoff, and a second
            // password prompt from a background retry is the H3 fault again.
            Error::Permission(_) => Retryability::Permanent,

            // The whole point of a rate limit is that the window moves.
            Error::RateLimit(_) | Error::Http(_) => Retryability::Transient,

            Error::Io(_)
            | Error::Persist(_)
            | Error::Snapshot(_)
            | Error::Journal(_)
            | Error::Cron(_)
            | Error::Dialoguer(_)
            | Error::Transaction(_)
            | Error::Other(_) => Retryability::Unknown,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err.to_string())
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Error::Http(err.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Json(err.to_string())
    }
}

impl From<toml::de::Error> for Error {
    fn from(err: toml::de::Error) -> Self {
        Error::Toml(err.to_string())
    }
}

impl From<mlua::Error> for Error {
    fn from(err: mlua::Error) -> Self {
        Error::LuaScript(err.to_string())
    }
}

impl From<tempfile::PersistError> for Error {
    fn from(err: tempfile::PersistError) -> Self {
        Error::Persist(err.to_string())
    }
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Error::Other(err.to_string())
    }
}

impl From<dialoguer::Error> for Error {
    fn from(err: dialoguer::Error) -> Self {
        Error::Dialoguer(err.to_string())
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Other(s)
    }
}

impl From<crate::parsers::Unrecognised> for Error {
    fn from(u: crate::parsers::Unrecognised) -> Self {
        Error::Unreadable(u.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The rule is a total order, and every pair must obey it.** A summary that took the
    /// *first* verdict, or the *last*, reads the same on a one-failure run and disagrees with
    /// itself the moment two failures arrive in a different order.
    #[test]
    fn the_least_optimistic_verdict_wins_whichever_order_they_arrive_in() {
        use Retryability::*;
        // Least to most dominant. Every later entry must beat every earlier one, both ways
        // round, and each must be a fixed point against itself.
        let ranked = [Transient, Exhausted, Unknown, Permanent];
        for (i, &lo) in ranked.iter().enumerate() {
            assert_eq!(lo.and_also(lo), lo, "{lo:?} disagreed with itself");
            for &hi in &ranked[i + 1..] {
                assert_eq!(lo.and_also(hi), hi, "{lo:?}.and_also({hi:?})");
                assert_eq!(hi.and_also(lo), hi, "{hi:?}.and_also({lo:?})");
            }
        }
    }

    /// A run of failures that were *all* passing ones is itself a passing failure — the whole
    /// reason the class is carried at all. If folding could only ever make things worse, the
    /// summary would answer `permanent` to every partial run and the class would carry no
    /// information.
    #[test]
    fn a_run_of_passing_failures_stays_passing() {
        let all_transient = [Retryability::Transient; 3]
            .into_iter()
            .fold(Retryability::Transient, Retryability::and_also);
        assert_eq!(all_transient, Retryability::Transient);
    }

    /// The constructor exists to stop `unknown` being the only answer a summary can give.
    #[test]
    fn a_classified_command_failure_reports_the_class_it_was_given() {
        for class in [
            Retryability::Transient,
            Retryability::Permanent,
            Retryability::Exhausted,
            Retryability::Unknown,
        ] {
            let e = Error::command_failed_classified("2 operation(s) failed", class);
            assert_eq!(e.retryability(), class);
            // A summary is about operations, not about a name. Reporting it as an absent name
            // would withdraw a declaration over a rate limit.
            assert!(!e.says_a_name_is_absent(), "{class:?}");
        }
    }
}
