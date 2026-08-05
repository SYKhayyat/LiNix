use thiserror::Error;

/// Whether running the same operation again could produce a different answer.
///
/// The retry loop must not decide this by reading the message text: a held `dpkg` lock and a
/// package name that does not exist arrive as the same `CommandFailed` string, and three
/// backoff rounds against the second only delay the report. `Unknown` retries, because that
/// is what every failure did before this distinction existed and a wrong guess in that
/// direction costs time rather than correctness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retryability {
    /// A lock someone else holds, a dropped connection, a mirror that timed out.
    Transient,
    /// The same command will fail the same way. Retrying cannot help.
    Permanent,
    /// Nothing classified it.
    Unknown,
    /// It was called transient, retried, and came back the same. The claim was tested and
    /// failed — so a further retry is not worth suggesting, but "this can never work" is more
    /// than was measured: the cause may be a broken `wget` on the PATH, fixable tomorrow.
    ///
    /// Kept apart from `Unknown` because the two lead to different sentences. `Unknown` means
    /// nobody looked; this means somebody did.
    Exhausted,
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
    /// `install X` converges the whole configuration, which is the model working: LiNix is
    /// declarative and your file is the truth. But it means a line you have never looked at can
    /// stop the install you just typed, and before this the error named the *command* — `sc`
    /// failed (exit 1056) — with nothing to say which declaration ran it or where that line
    /// lives (`Q34`).
    pub fn about_declaration(self, key: &str, origin: Option<&str>) -> Self {
        let where_from = match origin {
            Some(o) => format!(" ({})", o),
            None => String::new(),
        };
        let note = format!("
    while applying `{}`{}.", key, where_from);
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
            // is wrong, the file is wrong, the platform cannot do it, or LiNix itself said no.
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
