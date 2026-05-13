use thiserror::Error;

/// Centralized error handling for the LiNix project.
/// Utilizes 'thiserror' for ergonomic and descriptive error reporting across 33+ backends.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Backend '{0}' not found or unsupported on this platform")]
    BackendNotFound(String),

    #[error("Command execution failed: {0}")]
    CommandFailed(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON processing error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML processing error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Package '{0}' was not found in the target repository")]
    PackageNotFound(String),

    #[error("Operation was cancelled by the user")]
    Cancelled,

    #[error("Insufficient permissions for this operation: {0}")]
    Permission(String),

    #[error("API Rate limit exceeded (usually GitHub)")]
    RateLimit,

    #[error("Scripting engine error (Lua/Rhai): {0}")]
    LuaScript(String),

    #[error("Parallel Transaction failed: {0}")]
    Transaction(String),

    #[error("Platform or architecture not supported: {0}")]
    UnsupportedPlatform(String),

    #[error("Atomic file persistence error: {0}")]
    Persist(String),

    #[error("Snapshot provider failed: {0}")]
    Snapshot(String),

    #[error("{0}")]
    Other(String),
}

/// Specialized Result type for LiNix operations.
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