pub mod bisect;
pub mod context;
pub mod diagnostics;
pub mod fleet;
pub mod generation;
pub mod hooks;
pub mod insight;
pub mod metrics;
pub mod migrate;
pub mod policy;
pub mod profile;
pub mod run;
pub mod sandbox;
pub mod search;
pub mod services;
pub mod shim_manager;
pub mod sync;
pub mod teleport;
pub mod ui;
pub mod undo;

// Feature 5: Native System Automation & Alerts
pub mod scheduler;

// Feature 6: Ephemeral Environments (Directory-based mod)
pub mod shell;

// Re-export the primary application kernel (The Service Provider)
pub use self::context::App;

// Re-export the multi-engine scripting engine (Lua + Rhai)
pub use self::hooks::LuaHooks;

// Re-export the thread-safe telemetry collector
pub use self::metrics::MetricsCollector;

// Re-export the parallel cross-backend search engine
pub use self::search::UniversalSearch;

// Re-export the DAG-based system synchronization engine
pub use self::sync::SyncEngine;

// Re-export the environment runner (Sandboxing & Bridging)
pub use self::run::Runner;

// Re-export the system ingestion engine
pub use self::migrate::Migrator;

// Re-export the cross-backend transition logic
pub use self::teleport::Teleporter;

// Re-export the high-performance Rust shim manager
pub use self::shim_manager::ShimManager;

// Re-export the Snapshot Gallery / Time Travel manager
pub use self::undo::UndoManager;

// Re-export the contextual identity switcher (Profiles)
pub use self::profile::ProfileManager;

// Re-export the ephemeral shell orchestrator (Feature 6)
pub use self::shell::GhostShell;

/// Application layer constants
pub const APP_NAME: &str = "linix";
pub const DEFAULT_CONFIG_NAME: &str = "config.toml";
