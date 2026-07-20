//! Which file of a release gets installed.
//!
//! A declaration that resolves to a different artifact on two machines is not declarative, so
//! the choice is made from an ordered preference over a closed vocabulary, filtered first to
//! what this machine can run, and recorded in the lock once made.

pub mod capability;
pub mod discover;
pub mod format;
pub mod options;
pub mod pattern;
pub mod platform;
pub mod select;

pub use discover::{find_executable, DiscoveryError, Entry};
pub use format::{Format, FormatOrder, UnknownFormat};
pub use options::{default_formats, ArtifactOptions};
pub use pattern::{AssetPattern, BadPattern};
pub use platform::Platform;
pub use select::{select, Asset, NoMatch, PassedOver, Pick, Request, Selection};
