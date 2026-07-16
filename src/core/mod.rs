pub mod cache;
pub mod error;
pub mod executor;
pub mod git;
pub mod journal;
pub mod manager;
pub mod package;
pub mod ratelimiter;
pub mod retention;
pub mod security;
pub mod snapshot;
pub mod state;
pub mod transaction;
pub mod validator;

pub use cache::{PackageCache, SmartCache};

pub use error::{Error, Result};

pub use git::{GitCommit, GitManager};

pub use executor::{CommandExecutor, ExecutionLayer, RawExecutor};
pub use transaction::{GraphAction, Transaction, TransactionConfig};

pub use journal::{ActionStatus, Journal, JournalEntry};
pub use snapshot::{Snapshot, SnapshotManager, SnapshotProvider};

pub use manager::{
    BackendCapabilities, BackendCapabilitiesBuilder, BackendCore, HealthReport, HealthStatus,
    Installable, MetadataProvider, Queryable, RepoManager, Searchable, Upgradable,
};

pub use package::{Package, PackageSpec};

pub use security::verify_checksum;

pub use state::{GhostMetadata, ManagedPackage, StateRegistry};

pub use validator::Validator;

pub use ratelimiter::RateLimiter;

pub use retention::{RetentionConfig, RetentionItem, RetentionPolicy};
