//! The declarative model (SPEC II.1–II.7).
//!
//! **Profiles choose. Modules hold.** It is the one sentence that explains the whole
//! system, and it stops being true the moment profiles hold things or modules make choices
//! (V.2).

pub mod bootstrap;
pub mod cache;
pub mod conflict;
pub mod cycle;
pub mod dated;
pub mod dotfiles;
pub mod edit;
pub mod event;
pub mod exec;
pub mod firewall;
pub mod groups;
pub mod health;
pub mod introduced;
pub mod kernel;
pub mod layout;
pub mod modules;
pub mod priority;
pub mod profiles;
pub mod rehearsal;
pub mod resolve;
pub mod schedule;
pub mod scope;
pub mod script;
pub mod secret;
pub mod vars;
pub mod vars_embedded;
pub mod vars_provider;
pub mod vendor;

pub use conflict::Declared;
pub use edit::{active_module_files, inactive_declarations, Edit, Editor, Landing, Target, Writes};
pub use layout::{Layout, ModuleName};
pub use priority::Priority;
pub use resolve::{DesiredState, Reached, Resolver};
