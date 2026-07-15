pub mod cache;
pub mod error;
pub mod executor;
pub mod git;
pub mod journal;
pub mod locksig;
pub mod manager;
pub mod package;
pub mod ratelimiter;
pub mod retention;
pub mod security;
pub mod snapshot;
pub mod state;
pub mod transaction;
pub mod validator;

// Re-export performance and caching logic
pub use cache::{PackageCache, SmartCache};

// Re-export error and result types
pub use error::{Error, Result};

// Re-export the git (manifest version-control) wrapper
pub use git::{GitCommit, GitManager};

// Re-export the high-performance execution pipeline
pub use executor::{CommandExecutor, ExecutionLayer, RawExecutor};
pub use transaction::{GraphAction, Transaction, TransactionConfig};

// Re-export system safety layers
pub use journal::{ActionStatus, Journal, JournalEntry};
pub use snapshot::{Snapshot, SnapshotManager, SnapshotProvider};

// Re-export the capability-based trait system
pub use manager::{
    BackendCapabilities, BackendCapabilitiesBuilder, BackendCore, HealthReport, HealthStatus,
    Installable, MetadataProvider, Queryable, RepoManager, Searchable, Upgradable,
};

// Re-export the core data models
pub use package::{Package, PackageSpec};

// Re-export security utilities
pub use security::verify_checksum;

// Re-export state management
pub use state::{GhostMetadata, ManagedPackage, StateRegistry};

// Re-export input and command validation
pub use validator::Validator;

// Re-export rate limiter
pub use ratelimiter::RateLimiter;

// Re-export retention policy engine
pub use retention::{RetentionConfig, RetentionItem, RetentionPolicy};
