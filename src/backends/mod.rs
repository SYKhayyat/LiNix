pub mod registry;
pub mod generic;

// Specialized backends
pub mod github;
pub mod web;
pub mod link;
pub mod nix;
pub mod vscode;
pub mod mise;
pub mod emacs;
pub mod service;
pub mod appimage;
pub mod snap;
pub mod flatpak;
pub mod btrfs;
pub mod pacman;
pub mod dnf;

// New specialized backends with install path support
pub mod brew;
pub mod cargo;
pub mod pipx;
pub mod npm;
pub mod pnpm;
pub mod yarn;

pub use registry::{create_default_registry, BackendRegistry};
pub use generic::{GenericBackendCore, ManagerConfig};