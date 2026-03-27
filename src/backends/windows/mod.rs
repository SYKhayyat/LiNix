#[cfg(target_os = "windows")]
pub mod choco;
#[cfg(target_os = "windows")]
pub mod scoop;
#[cfg(target_os = "windows")]
pub mod winget;

#[cfg(target_os = "windows")]
pub use choco::ChocoManager;
#[cfg(target_os = "windows")]
pub use scoop::ScoopManager;
#[cfg(target_os = "windows")]
pub use winget::WingetManager;
