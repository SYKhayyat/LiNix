// src/app/mod.rs
pub mod context;
pub mod hooks;
pub mod metrics;
pub mod search;
pub mod sync;

pub use self::context::App;
pub use self::hooks::LuaHooks;
pub use self::metrics::MetricsCollector;
pub use self::search::UniversalSearch;
pub use self::sync::SyncEngine;