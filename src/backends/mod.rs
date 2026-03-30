pub mod registry;

// System package managers (Linux only)
#[cfg(target_os = "linux")]
pub mod apk;
#[cfg(target_os = "linux")]
pub mod apt;
#[cfg(target_os = "linux")]
pub mod dnf;
#[cfg(target_os = "linux")]
pub mod pacman;
#[cfg(target_os = "linux")]
pub mod zypper;

// Universal package managers
pub mod brew;
pub mod flatpak;
pub mod snap;

// Language-specific package managers
pub mod bun;
pub mod cargo;
pub mod composer;
pub mod gem;
pub mod go;
pub mod npm;
pub mod pip;
pub mod pipx;
pub mod poetry;
pub mod yarn;

// GitHub releases
pub mod github;

// New backends
pub mod uv;
pub mod pnpm;
pub mod vscode;
pub mod mise;

// Windows backends
#[cfg(target_os = "windows")]
pub mod windows;

pub use registry::{create_default_registry, BackendRegistry};