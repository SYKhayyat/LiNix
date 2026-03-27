//! LiNix - Universal Package Manager
//!
//! A unified interface for managing packages across multiple package managers
//! and platforms with support for parallel operations, Lua hooks, and comprehensive
//! backend support.

pub mod app;
pub mod backends;
pub mod cli;
pub mod config;
pub mod core;
pub mod parsers;
pub mod utils;

pub use app::App;
pub use config::Config;
pub use core::error::{Error, Result};
