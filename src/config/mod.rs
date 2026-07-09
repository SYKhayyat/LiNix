#[allow(clippy::module_inception)]
pub mod config;
pub mod manifest;
pub mod parser; // <-- added

pub use config::{Config, PruneScope};

/// Module-level constants for configuration defaults
pub const DEFAULT_BACKEND: &str = "apt";
pub const CONFIG_DIR: &str = ".config/linix";
