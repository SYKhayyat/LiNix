pub mod archive; // Add this
pub mod command;
pub mod file;
pub mod progress;
pub mod retry;

pub use command::*;
pub use file::*;
pub use progress::*;
pub use retry::*;
pub use archive::extract_archive; // Add this