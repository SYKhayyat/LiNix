pub mod cockpit;
pub mod preview;

/// Re-export the TUI Preview engine for high-performance pre-flight checks.
pub use self::preview::TuiPreview;

/// Re-export the generation cockpit (time-travel dashboard).
pub use self::cockpit::{Cockpit, CockpitAction, GenView};
