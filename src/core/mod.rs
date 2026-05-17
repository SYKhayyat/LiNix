pub mod cache;
pub mod error;
pub mod executor;
pub mod journal;
pub mod manager;
pub mod package;
pub mod security;
pub mod snapshot;
pub mod state;
pub mod transaction;
pub mod validator;

// Re-export performance and caching logic
pub use cache::{PackageCache, SmartCache};

// Re-export error and result types
pub use error::{Error, Result};

// Re-export the high-performance execution pipeline
pub use executor::{CommandExecutor, ExecutionLayer, RawExecutor};
pub use transaction::{Transaction, GraphAction, TransactionConfig};

// Re-export system safety layers
pub use snapshot::{Snapshot, SnapshotManager, SnapshotProvider};
pub use journal::{Journal, JournalEntry, ActionStatus};

// Re-export the capability-based trait system
pub use manager::{
    BackendCore, Installable, Queryable, RepoManager, Searchable,
    Upgradable, BackendCapabilities, BackendCapabilitiesBuilder,
};

// Re-export the core data models
pub use package::{Package, PackageSpec};

// Re-export security utilities
pub use security::verify_checksum;

// Re-export state management
pub use state::{ManagedPackage, StateRegistry, GhostMetadata};

// Re-export input and command validation
pub use validator::Validator;

// Note: ratelimiter module has been reintegrated - available in backends