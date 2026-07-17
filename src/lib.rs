//! LiNix - Universal Mission-Critical Package Manager
//!
//! A high-performance, Directed Acyclic Graph (DAG) based orchestration engine
//! for managing packages across 33+ backends and multiple platforms.

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
