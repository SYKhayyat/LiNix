pub mod artifact;
pub mod generic;
pub mod node_registry;
pub mod onboarder;
pub mod pip_search;
pub mod registry;

pub mod appimage;
pub mod btrfs;
pub mod conda;
pub mod dnf;
pub mod emacs;
pub mod flatpak;
pub mod github;
pub mod link;
pub mod mise;
pub mod nix;
pub mod pacman;
pub mod service;
pub mod snap;
pub mod vscode;
pub mod web;
pub mod xbps;

#[cfg(target_os = "windows")]
pub mod psresource;

pub mod brew;
pub mod cargo;
pub mod npm;
pub mod pipx;
pub mod pnpm;
pub mod uv;
pub mod yarn;

// Dedicated backends whose CLI doesn't fit the generic config model (no uninstall verb,
// filesystem enumeration, or a subcommand-of-another-binary invocation).
pub mod go;
pub mod krew;
pub mod pubdart;

pub use generic::{GenericBackendCore, ManagerConfig};
pub use registry::{create_default_registry, BackendRegistry};

/// True when a version string is a concrete pin (not "latest"/"*"/empty). Shared by
/// backends that honor `PackageSpec.options["version"]` for reproducible installs.
pub fn concrete_version(v: &str) -> bool {
    !v.is_empty() && v != "latest" && v != "*"
}
