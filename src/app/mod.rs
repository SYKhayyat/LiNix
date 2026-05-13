pub mod context;
pub mod hooks;
pub mod metrics;
pub mod search;
pub mod sync;
pub mod run;
pub mod migrate;
pub mod teleport;
pub mod shim_manager;
pub mod undo;
pub mod bridge;
pub mod shell;
pub mod profile;
pub mod ui;

// Re-export the primary application kernel (The Dependency Injector)
pub use self::context::App;

// Re-export the multi-engine scripting engine (Lua + Rhai) - Point 4
pub use self::hooks::LuaHooks;

// Re-export the thread-safe telemetry collector - Point 2.3
pub use self::metrics::MetricsCollector;

// Re-export the parallel cross-backend search engine - Point 2.3
pub use self::search::UniversalSearch;

// Re-export the DAG-based system synchronization engine - Point 2.2 / 8 / 9
pub use self::sync::SyncEngine;

// Re-export the environment runner (Sandboxing & Bridging) - Point 16 / 17 / 19
pub use self::run::Runner;

// Re-export the system ingestion engine - Point 3
pub use self::migrate::Migrator;

// Re-export the cross-backend migration logic - Point 5
pub use self::teleport::Teleporter;

// Re-export the high-performance Rust shim manager - Point 6
pub use self::shim_manager::ShimManager;

// Re-export the Snapshot Gallery / Time Travel manager - Point 12
pub use self::undo::UndoManager;

// Re-export the contextual identity switcher - Point 18
pub use self::profile::ProfileManager;

// Re-export the ephemeral shell orchestrator - Point 19 / 20
pub use self::shell::GhostShell;

/// Application layer constants
pub const APP_NAME: &str = "linix";
pub const DEFAULT_CONFIG_NAME: &str = "config.toml";