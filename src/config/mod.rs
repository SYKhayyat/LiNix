#[allow(clippy::module_inception)]
pub mod config;
pub mod grammar;
pub mod parser;
pub mod settings;

pub use config::{Config, GuardSettings, VarsSettings, PREFERENCES_FILE_NAME};
pub use settings::{resolve_root, ResolvedRoot, RootSource, Settings};

/// Module-level constants for configuration defaults
pub const DEFAULT_BACKEND: &str = "apt";
pub const CONFIG_DIR: &str = ".config/linix";
