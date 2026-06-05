pub mod config;
pub mod parser;
pub mod manifest; // <-- added

pub use config::Config;

/// Module-level constants for configuration defaults
pub const DEFAULT_BACKEND: &str = "apt";
pub const CONFIG_DIR: &str = ".config/linix";