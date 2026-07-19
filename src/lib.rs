//! LiNix - a declarative package manager.
//!
//! You list the packages you want in a file; `sync` installs what is missing and
//! removes what is no longer listed, across every supported package manager.

pub mod app;
pub mod backends;
pub mod cli;
pub mod config;
pub mod core;
pub mod model;
pub mod parsers;
pub mod utils;

// Primary entry points for library consumers
pub use app::App;
pub use config::Config;
pub use core::error::{Error, Result};
pub use core::manager::BackendCore;
pub use core::transaction::TransactionConfig;

/// LiNix Version Identifier
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
