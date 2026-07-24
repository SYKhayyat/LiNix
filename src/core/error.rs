use thiserror::Error;

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

    #[error("Command execution failed: {0}")]
    CommandFailed(String),

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

    #[error("Package '{0}' was not found in the target repository")]
    PackageNotFound(String),

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
