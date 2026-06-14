use thiserror::Error;

/// Centralized error handling for the LiNix mission-critical engine.
/// 
/// Modernized for v3.6.0: This enum provides high-fidelity error variants 
/// for every LiNix subsystem. It is designed to be thread-safe (Clone + Send) 
/// to support parallel transaction auditing and autonomous diagnostics.
#[derive(Debug, Error, Clone)]
pub enum Error {
    /// Failure when a backend (e.g., 'apt', 'brew') is missing or unsupported.
    #[error("Backend '{0}' not found or unsupported on this platform")]
    BackendNotFound(String),

    /// Failure during the execution of an external system command.
    #[error("Command execution failed: {0}")]
    CommandFailed(String),

    /// Standard filesystem I/O failures.
    #[error("I/O error: {0}")]
    Io(String),

    /// Corruption or logical errors within the LiNix TOML configuration.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Failure to pass security or naming constraints.
    #[error("Validation error: {0}")]
    Validation(String),

    /// Network-level failures during package downloads or API queries.
    #[error("HTTP request failed: {0}")]
    Http(String),

    /// Failures during JSON serialization/deserialization of metadata.
    #[error("JSON processing error: {0}")]
    Json(String),

    /// Failures during TOML parsing.
    #[error("TOML processing error: {0}")]
    Toml(String),

    /// Failure when a package identifier is not found in the remote registry.
    #[error("Package '{0}' was not found in the target repository")]
    PackageNotFound(String),

    /// Error indicating the user manually aborted a transaction.
    #[error("Operation was cancelled by the user")]
    Cancelled,

    /// Insufficient system privileges (e.g., missing sudo).
    #[error("Insufficient permissions for this operation: {0}")]
    Permission(String),

    /// API Rate limit reached (primarily for GitHub and VS Code Marketplace).
    #[error("API Rate limit exceeded")]
    RateLimit,

    /// Failures within the Lua or Rhai lifecycle scripts.
    #[error("Scripting engine error (Lua/Rhai): {0}")]
    LuaScript(String),

    /// Logical failures during Directed Acyclic Graph (DAG) construction or execution.
    #[error("Parallel Transaction failed: {0}")]
    Transaction(String),

    /// Failure when running on an OS/Arch variant that LiNix does not yet support.
    #[error("Platform or architecture not supported: {0}")]
    UnsupportedPlatform(String),

    /// Failure to atomically persist state to disk.
    #[error("Atomic file persistence error: {0}")]
    Persist(String),

    /// Feature 2: Failures in filesystem snapshot providers (Btrfs, ZFS, etc.).
    #[error("Snapshot provider failure: {0}")]
    Snapshot(String),

    /// Bug Fix 10: Failures in the mission-critical transaction journal (WAL).
    #[error("Journal/WAL failure: {0}")]
    Journal(String),

    /// Feature 5: Failures in the native system task scheduler.
    #[error("Cron scheduling failure: {0}")]
    Cron(String),

    /// Modernized Fix for E0277: Native mapping for interactive UI failures.
    #[error("Interactive UI error (Dialoguer): {0}")]
    Dialoguer(String),

    /// Generic catch-all for miscellaneous failures.
    #[error("{0}")]
    Other(String),
}

/// Specialized Result type for LiNix kernel operations.
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

/// A+ Grade Fix: Resolves E0277 by providing a From implementation for Dialoguer.
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