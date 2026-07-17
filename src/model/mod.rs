//! The declarative model (SPEC II.1–II.7).
//!
//! **Profiles choose. Modules hold.** It is the one sentence that explains the whole
//! system, and it stops being true the moment profiles hold things or modules make choices
//! (V.2).

pub mod conflict;
pub mod dated;
pub mod layout;
pub mod modules;
pub mod priority;
pub mod profiles;
pub mod resolve;

pub use conflict::Declared;
pub use layout::Layout;
pub use priority::Priority;
pub use resolve::{DesiredState, Reached, Resolver};
