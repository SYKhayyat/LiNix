use thiserror::Error;

/// Centralized error handling for the LiNix project.
/// Hardened for Version 3.5.0 to support parallel execution and telemetry
/// by ensuring the Error type is fully Cloneable and Debuggable.
#[derive(Debug, Error, Clone)]
pub enum Error {
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

impl From<String> for Error {
    fn from(s: String) -> Self { 
        Error::Other(s) 
    }
}