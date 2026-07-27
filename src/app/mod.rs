pub mod adopt;
pub mod apply;
pub mod bisect;
pub mod bundle;
pub mod check;
pub mod conflicts;
pub mod context;
pub mod diagnostics;
pub mod eval;
pub mod events;
pub mod export;
pub mod fleet;
pub mod hooks;
pub mod insight;
pub mod leases;
pub mod locate;
pub mod metrics;
pub mod module_registry;
pub mod pm_hooks;
pub mod profile;
pub mod profile_expr;
pub mod rebuild;
pub mod repl;
pub mod run;
pub mod sandbox;
pub mod search;
pub mod services;
pub mod shim_manager;
pub mod snapshot_restore;
pub mod sync;
pub mod ui;
pub mod vocab;

pub mod scheduler;
pub mod shell;

pub use self::adopt::Adopter;
pub use self::apply::{
    Bootstrap, Dependents, Dotfiles, Execs, Extras, Firewall, Repositories, Schedules,
};
pub use self::context::App;
pub use self::hooks::LuaHooks;
pub use self::leases::Leases;
pub use self::metrics::MetricsCollector;
pub use self::profile::ProfileManager;
pub use self::run::Runner;
pub use self::search::UniversalSearch;
pub use self::shell::EphemeralShell;
pub use self::shim_manager::ShimManager;
pub use self::snapshot_restore::SnapshotRestore;
pub use self::sync::SyncEngine;

pub const APP_NAME: &str = "linix";
pub const DEFAULT_CONFIG_NAME: &str = crate::config::PREFERENCES_FILE_NAME;
