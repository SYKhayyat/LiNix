// tests/backend_tests.rs

use linix::core::executor::DryRunOutput;
use linix::core::{BackendCapabilities, PackageSpec};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

// Import our exhaustive A+ Test Infrastructure
mod mock_providers;
use mock_providers::TestKernel;

// ============================================================================
// BACKEND TEST HARNESS
// ============================================================================

async fn run_capability_test(backend: Arc<BackendCapabilities>, package_name: &str) {
    let installer = backend
        .as_installable()
        .expect("Test Failure: Backend must implement Installable trait for lifecycle testing.");

    let spec = PackageSpec {
        name: package_name.to_string(),
        backend: backend.name().to_string(),
        options: HashMap::new(),
        requires: vec![],
        present: true,
    };

    let inst_res = installer.install(&[spec], backend.needs_root()).await;
    assert!(
        inst_res.is_ok(),
        "Install failed for {}: {:?}",
        backend.name(),
        inst_res.err()
    );

    let rem_res = installer
        .remove(&[package_name.to_string()], backend.needs_root())
        .await;
    assert!(
        rem_res.is_ok(),
        "Remove failed for {}: {:?}",
        backend.name(),
        rem_res.err()
    );
}

// ============================================================================
// OS-SPECIFIC BACKEND TESTS
// ============================================================================

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_apt_backend_hermetic_logic() {
    let kernel = TestKernel::new().await;
    let backend = kernel.app.registry.get("apt").expect("Missing apt backend");

    // Set expected sudo-prefixed responses for Linux
    kernel.mock_executor.set_response(
        "sudo apt install -y curl",
        Ok(DryRunOutput::default().into()),
    );
    kernel
        .mock_executor
        .set_response("sudo apt purge -y curl", Ok(DryRunOutput::default().into()));

    timeout(Duration::from_secs(5), run_capability_test(backend, "curl"))
        .await
        .expect("APT Logic test timed out");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_pacman_backend_hermetic_logic() {
    let kernel = TestKernel::new().await;
    let backend = kernel
        .app
        .registry
        .get("pacman")
        .expect("Missing pacman backend");

    kernel.mock_executor.set_response(
        "sudo pacman -S --noconfirm --needed git",
        Ok(DryRunOutput::default().into()),
    );
    kernel.mock_executor.set_response(
        "sudo pacman -Rs --noconfirm git",
        Ok(DryRunOutput::default().into()),
    );

    timeout(Duration::from_secs(5), run_capability_test(backend, "git"))
        .await
        .expect("Pacman Logic test timed out");
}

#[cfg(target_os = "windows")]
#[tokio::test]
async fn test_winget_backend_hermetic_logic() {
    let kernel = TestKernel::new().await;
    let backend = kernel
        .app
        .registry
        .get("winget")
        .expect("Missing winget backend");

    kernel.mock_executor.set_response(
        "winget install --silent --accept-source-agreements --accept-package-agreements vim",
        Ok(DryRunOutput::default().into()),
    );
    kernel.mock_executor.set_response(
        "winget uninstall --silent vim",
        Ok(DryRunOutput::default().into()),
    );

    timeout(Duration::from_secs(5), run_capability_test(backend, "vim"))
        .await
        .expect("Winget Logic test timed out");
}

// ============================================================================
// CROSS-PLATFORM & SPECIALIZED TESTS
// ============================================================================

#[tokio::test]
async fn test_brew_backend_hermetic_logic() {
    let kernel = TestKernel::new().await;
    let backend = kernel
        .app
        .registry
        .get("brew")
        .expect("Missing brew backend");

    kernel
        .mock_executor
        .set_response("brew install htop", Ok(DryRunOutput::default().into()));
    kernel
        .mock_executor
        .set_response("brew uninstall htop", Ok(DryRunOutput::default().into()));

    timeout(Duration::from_secs(5), run_capability_test(backend, "htop"))
        .await
        .expect("Homebrew Logic test timed out");
}

#[tokio::test]
async fn test_cargo_backend_hermetic_logic() {
    let kernel = TestKernel::new().await;
    let backend = kernel
        .app
        .registry
        .get("cargo")
        .expect("Missing cargo backend");

    kernel
        .mock_executor
        .set_response("cargo install ripgrep", Ok(DryRunOutput::default().into()));
    kernel.mock_executor.set_response(
        "cargo uninstall ripgrep",
        Ok(DryRunOutput::default().into()),
    );

    timeout(
        Duration::from_secs(5),
        run_capability_test(backend, "ripgrep"),
    )
    .await
    .expect("Cargo Logic test timed out");
}

#[tokio::test]
async fn test_link_backend_vfs_integrity() {
    let kernel = TestKernel::new().await;
    let backend = kernel
        .app
        .registry
        .get("link")
        .expect("Missing link backend");

    let source_path = kernel.tmp.path().join("source_file.conf");
    tokio::fs::write(&source_path, "theme: solarized")
        .await
        .unwrap();
    let target_path = kernel.tmp.path().join("target_link.conf");

    let mut options = HashMap::new();
    options.insert(
        "target".to_string(),
        target_path.to_string_lossy().to_string(),
    );

    let spec = PackageSpec {
        name: source_path.to_string_lossy().to_string(),
        backend: "link".to_string(),
        options,
        requires: vec![],
        present: true,
    };

    let installer = backend.as_installable().unwrap();

    installer
        .install(&[spec], false)
        .await
        .expect("Link creation failed");

    let vfs_diff = kernel.app.executor.get_vfs_diff();
    let link_created = vfs_diff
        .iter()
        .any(|(path, val)| path == &target_path && val.contains("LINK:"));
    assert!(
        link_created,
        "Link record was not found in the Virtual Filesystem closure."
    );

    installer
        .remove(&[target_path.to_string_lossy().to_string()], false)
        .await
        .expect("Link purge failed");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_metadata_provider_resolution() {
    let kernel = TestKernel::new().await;
    let backend = kernel.app.registry.get("apt").expect("Missing apt backend");

    let provider = backend
        .as_metadata_provider()
        .expect("Apt must implement MetadataProvider trait.");

    // apt DELIBERATELY disables transitive-dependency expansion (`depends_args: None` in the
    // registry): apt resolves its own dependency closure at `apt-get install` time, and LiNix
    // re-deriving it caused a recursive `apt depends` fan-out (jq -> libc6 -> libgcc-s1 -> …)
    // that hung `status`/`sync`. So the provider must return an EMPTY set for apt. Asserting
    // that here guards against the expansion being silently re-enabled and re-introducing the
    // hang. (The generic depends-parsing path itself is covered by the backends::generic test
    // `get_dependencies_parses_names_without_sudo`, which uses a backend that DOES set
    // `depends_args`.)
    let deps = provider
        .get_dependencies("curl")
        .await
        .expect("Dependency resolution failed");

    assert!(
        deps.is_empty(),
        "apt intentionally does not expand dependencies (anti-hang); got {:?}",
        deps
    );
}
