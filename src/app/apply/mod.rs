//! One facet of `DesiredState` per module.
//!
//! These were methods on `App`, which meant every one of them could reach all thirteen of its
//! collaborators — while none of them used more than four. Each is a struct holding exactly
//! what it uses, so it can be built and exercised without an `App` at all.

pub mod bootstrap;
pub mod dependents;
pub mod dotfiles;
pub mod execs;
pub mod extras;
pub mod firewall;
pub mod repositories;
pub mod schedules;

pub use bootstrap::Bootstrap;
pub use dependents::Dependents;
pub use dotfiles::Dotfiles;
pub use execs::Execs;
pub use extras::Extras;
pub use firewall::Firewall;
pub use repositories::Repositories;
pub use schedules::Schedules;
