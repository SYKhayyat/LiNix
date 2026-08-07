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
/// The command implementations.
///
/// In the library rather than the binary so the suite can reach them. `main.rs` declared
/// `mod verbs;`, which put ~8,500 lines of real logic — the lock/unlock ledger, `check_health`,
/// the failure-attribution classifier, `reconcile` itself — where no test binary could link to
/// it, so it could only be exercised by spawning the program.
pub mod verbs;

// Primary entry points for library consumers
pub use app::App;
pub use config::Config;
pub use core::error::{Error, Result};
pub use core::manager::BackendCore;
pub use core::transaction::TransactionConfig;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
