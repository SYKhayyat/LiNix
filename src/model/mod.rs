//! The declarative model (SPEC II.1–II.7).
//!
//! **Profiles choose. Modules hold.** It is the one sentence that explains the whole
//! system, and it stops being true the moment profiles hold things or modules make choices
//! (V.2).

pub mod conflict;
pub mod cycle;
pub mod dated;
pub mod edit;
pub mod layout;
pub mod modules;
pub mod priority;
pub mod profiles;
pub mod resolve;
pub mod bootstrap;
pub mod dotfiles;
pub mod exec;
pub mod schedule;
pub mod scope;
pub mod vars;
pub mod vars_embedded;
pub mod vars_provider;

pub use conflict::Declared;
pub use edit::{active_module_files, inactive_declarations, Edit, Editor, Landing, Target};
pub use layout::{Layout, ModuleName};
pub use priority::Priority;
pub use resolve::{DesiredState, Reached, Resolver};
