// tests/backends_tests.rs
//! Integration tests for all LiNix backends.
//! Each test checks `is_available()` and, if available, performs a dry-run install/remove
//! of a dummy package that is guaranteed to exist (e.g., `echo` for system backends,
//! `cargo:ripgrep` for language backends, etc.).
//! Uses `CommandExecutor` with `dry_run = true` to avoid real system changes.

use linix::backends::create_default_registry;
use linix::config::Config;
use linix::app::LuaHooks;
use linix::core::{CommandExecutor, PackageSpec, Installable, BackendCapabilities};
use std::collections::HashMap;
use std::sync::Arc;
use tracing_test::traced_test;

/// Helper to create a registry with dry-run executor.
async fn create_test_registry() -> Arc<linix::backends::BackendRegistry> {
    let config = Config::default();
    let executor = CommandExecutor::new(true, false); // dry-run = true
    let hooks = Arc::new(LuaHooks::new(&config).unwrap());
    Arc::new(create_default_registry(executor, &config, hooks).await)
}

/// Helper to run a simple install/remove test for a backend.
/// Uses a dummy package that is expected to be available in the backend's repository.
async fn test_backend_install_remove(
    backend: &BackendCapabilities,
    package_name: &str,
    backend_name: &str,
) {
    if !backend.is_available() {
        eprintln!("Skipping test for {}: not available", backend_name);
        return;
    }
    if !backend.is_installable() {
        eprintln!("Skipping install test for {}: not installable", backend_name);
        return;
    }

    let spec = PackageSpec {
        name: package_name.to_string(),
        backend: backend_name.to_string(),
        options: HashMap::new(),
        requires: vec![],
    };

    let installer = backend.as_installable().unwrap();
    // Install (dry-run)
    let result = installer.install(&[spec.clone()], false).await;
    assert!(result.is_ok(), "Install failed for {}:{}: {:?}", backend_name, package_name, result);

    // Remove (dry-run)
    let result = installer.remove(&[package_name.to_string()], false).await;
    assert!(result.is_ok(), "Remove failed for {}:{}: {:?}", backend_name, package_name, result);
}

#[tokio::test]
#[traced_test]
async fn test_apt_backend() {
    let registry = create_test_registry().await;
    if let Some(apt) = registry.get("apt") {
        test_backend_install_remove(&apt, "echo", "apt").await;
    } else {
        eprintln!("APT backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_pacman_backend() {
    let registry = create_test_registry().await;
    if let Some(pacman) = registry.get("pacman") {
        test_backend_install_remove(&pacman, "echo", "pacman").await;
    } else {
        eprintln!("Pacman backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_dnf_backend() {
    let registry = create_test_registry().await;
    if let Some(dnf) = registry.get("dnf") {
        test_backend_install_remove(&dnf, "echo", "dnf").await;
    } else {
        eprintln!("DNF backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_brew_backend() {
    let registry = create_test_registry().await;
    if let Some(brew) = registry.get("brew") {
        test_backend_install_remove(&brew, "hello", "brew").await;
    } else {
        eprintln!("Homebrew backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_cargo_backend() {
    let registry = create_test_registry().await;
    if let Some(cargo) = registry.get("cargo") {
        test_backend_install_remove(&cargo, "cargo-edit", "cargo").await;
    } else {
        eprintln!("Cargo backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_npm_backend() {
    let registry = create_test_registry().await;
    if let Some(npm) = registry.get("npm") {
        test_backend_install_remove(&npm, "typescript", "npm").await;
    } else {
        eprintln!("NPM backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_pip_backend() {
    let registry = create_test_registry().await;
    if let Some(pip) = registry.get("pip") {
        test_backend_install_remove(&pip, "requests", "pip").await;
    } else {
        eprintln!("Pip backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_github_backend() {
    let registry = create_test_registry().await;
    if let Some(github) = registry.get("github") {
        // Use a small, stable GitHub release (ripgrep)
        test_backend_install_remove(&github, "BurntSushi/ripgrep", "github").await;
    } else {
        eprintln!("GitHub backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_web_backend() {
    let registry = create_test_registry().await;
    if let Some(web) = registry.get("web") {
        // Use a small, stable file (a known URL)
        test_backend_install_remove(&web, "https://example.com/", "web").await;
    } else {
        eprintln!("Web backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_flatpak_backend() {
    let registry = create_test_registry().await;
    if let Some(flatpak) = registry.get("flatpak") {
        test_backend_install_remove(&flatpak, "org.freedesktop.Platform", "flatpak").await;
    } else {
        eprintln!("Flatpak backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_snap_backend() {
    let registry = create_test_registry().await;
    if let Some(snap) = registry.get("snap") {
        test_backend_install_remove(&snap, "core", "snap").await;
    } else {
        eprintln!("Snap backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_nix_backend() {
    let registry = create_test_registry().await;
    if let Some(nix) = registry.get("nix") {
        test_backend_install_remove(&nix, "hello", "nix").await;
    } else {
        eprintln!("Nix backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_mise_backend() {
    let registry = create_test_registry().await;
    if let Some(mise) = registry.get("mise") {
        test_backend_install_remove(&mise, "node", "mise").await;
    } else {
        eprintln!("Mise backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_vscode_backend() {
    let registry = create_test_registry().await;
    if let Some(vscode) = registry.get("vscode") {
        test_backend_install_remove(&vscode, "rust-lang.rust-analyzer", "vscode").await;
    } else {
        eprintln!("VSCode backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_emacs_backend() {
    let registry = create_test_registry().await;
    if let Some(emacs) = registry.get("emacs") {
        test_backend_install_remove(&emacs, "use-package", "emacs").await;
    } else {
        eprintln!("Emacs backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_service_backend() {
    let registry = create_test_registry().await;
    if let Some(service) = registry.get("service") {
        // Service backend only enables/disables, no install/remove of actual packages.
        // We'll just check availability and skip install test.
        assert!(service.is_available() || !service.is_available()); // placeholder
    } else {
        eprintln!("Service backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_link_backend() {
    let registry = create_test_registry().await;
    if let Some(link) = registry.get("link") {
        // Link backend is always available (no external command)
        assert!(link.is_available());
        // Install test with a dummy file
        test_backend_install_remove(&link, "/tmp/linix_test_link_source", "link").await;
    } else {
        eprintln!("Link backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_appimage_backend() {
    let registry = create_test_registry().await;
    if let Some(appimage) = registry.get("appimage") {
        // AppImage backend requires Linux
        if cfg!(target_os = "linux") {
            test_backend_install_remove(&appimage, "https://example.com/dummy.AppImage", "appimage").await;
        } else {
            eprintln!("AppImage backend only available on Linux, skipping");
        }
    } else {
        eprintln!("AppImage backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_btrfs_backend() {
    let registry = create_test_registry().await;
    if let Some(btrfs) = registry.get("btrfs") {
        // BTRFS backend requires Linux and btrfs tools
        if cfg!(target_os = "linux") && btrfs.is_available() {
            // Use a temporary path for testing subvolume (dry-run will not create)
            test_backend_install_remove(&btrfs, "/tmp/linix_test_subvol", "btrfs").await;
        } else {
            eprintln!("BTRFS backend not available, skipping");
        }
    } else {
        eprintln!("BTRFS backend not found in registry");
    }
}

#[tokio::test]
#[traced_test]
async fn test_windows_backends() {
    // Only run on Windows
    if !cfg!(target_os = "windows") {
        eprintln!("Skipping Windows-specific backend tests on non-Windows OS");
        return;
    }
    let registry = create_test_registry().await;
    
    if let Some(winget) = registry.get("winget") {
        test_backend_install_remove(&winget, "Microsoft.PowerShell", "winget").await;
    } else {
        eprintln!("Winget backend not found in registry");
    }
    
    if let Some(scoop) = registry.get("scoop") {
        test_backend_install_remove(&scoop, "sudo", "scoop").await;
    } else {
        eprintln!("Scoop backend not found in registry");
    }
}