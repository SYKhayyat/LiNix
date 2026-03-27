use thiserror::Error;

/// Core error types for LiNix
#[derive(Debug, Error)]
pub enum Error {
    #[error("Backend '{0}' not available")]
    BackendUnavailable(String),

    #[error("Backend '{0}' not found")]
    BackendNotFound(String),

    #[error("Command execution failed: {0}")]
    CommandFailed(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Package '{0}' not found")]
    PackageNotFound(String),

    #[error("Operation cancelled by user")]
    Cancelled,

    #[error("Insufficient permissions: {0}")]
    Permission(String),

    #[error("Rate limit exceeded")]
    RateLimit,

    #[error("Lua script error: {0}")]
    LuaScript(String),

    #[error("Transaction failed: {0}")]
    Transaction(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Platform not supported: {0}")]
    UnsupportedPlatform(String),

    #[error("File persist error: {0}")]
    Persist(String),

    #[error("{0}")]
    Other(String),
}

/// Specialized Result type for LiNix operations
pub type Result<T> = std::result::Result<T, Error>;

impl From<mlua::Error> for Error {
    fn from(err: mlua::Error) -> Self {
        Error::LuaScript(err.to_string())
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Other(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Other(s.to_string())
    }
}

impl From<tempfile::PersistError> for Error {
    fn from(err: tempfile::PersistError) -> Self {
        Error::Persist(err.to_string())
    }
}
