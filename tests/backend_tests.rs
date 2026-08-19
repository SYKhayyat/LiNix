// tests/backend_tests.rs

use shall::core::executor::DryRunOutput;
// Only the two `cfg(target_os = "linux")` tests below escalate, so the import lives with them
// rather than at the top, where every other platform would carry an unused one.
#[cfg(target_os = "linux")]
use shall::core::executor::CommandExecutor;
use shall::core::{BackendCapabilities, PackageSpec};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

// Import our exhaustive A+ Test Infrastructure
use crate::mock_providers::TestKernel;

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
        options: Default::default(),
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
        .remove(
            &[package_name.to_string()],
            backend.needs_root(),
            shall::app::sync::guard::Reaped::for_reason(
                shall::app::sync::guard::GuardScope::Remove,
                "a unit test of the effector itself",
            ),
        )
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

    // **The argv the product actually emits**, measured rather than remembered. This test is
    // `cfg(target_os = "linux")` and the build matrix produced one target out of four, so it had
    // never once executed — and in that time the product moved twice underneath it: `--` now
    // terminates the options before a package name (E29), and removal is `remove` rather than
    // `purge`, which became opt-in. A stub nobody matches proves nothing, which is the whole
    // reason the mock refuses to stay quiet about one.
    kernel.mock_executor.set_response(
        &CommandExecutor::as_launched("apt", &["install", "-y", "--", "curl"], true),
        Ok(DryRunOutput::default().into()),
    );
    kernel.mock_executor.set_response(
        &CommandExecutor::as_launched("apt", &["remove", "-y", "--", "curl"], true),
        Ok(DryRunOutput::default().into()),
    );

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
        &CommandExecutor::as_launched(
            "pacman",
            &["-S", "--noconfirm", "--needed", "--", "git"],
            true,
        ),
        Ok(DryRunOutput::default().into()),
    );
    kernel.mock_executor.set_response(
        &CommandExecutor::as_launched("pacman", &["-Rs", "--noconfirm", "--", "git"], true),
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

    // The `--` is there because the probe measured winget honouring it on windows-latest, both
    // ways identical in exit code, in output, and in how the operand is echoed. It was listed
    // as not terminating on the shape of the parser and that was an inference, not a fact.
    kernel.mock_executor.set_response(
        "winget install --silent --accept-source-agreements --accept-package-agreements -- vim",
        Ok(DryRunOutput::default().into()),
    );
    kernel.mock_executor.set_response(
        "winget uninstall --silent -- vim",
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
        .set_response("brew install -- htop", Ok(DryRunOutput::default().into()));
    kernel
        .mock_executor
        .set_response("brew uninstall -- htop", Ok(DryRunOutput::default().into()));

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

    kernel.mock_executor.set_response(
        "cargo install -- ripgrep",
        Ok(DryRunOutput::default().into()),
    );
    kernel.mock_executor.set_response(
        // With the terminator, because that is what runs. Without it this registration matched
        // nothing and the assertion below passed on the mock's default answer.
        "cargo uninstall -- ripgrep",
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

    let mut options = shall::config::grammar::Options::default();
    options.set("target", target_path.to_string_lossy().to_string());

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
        .remove(
            &[target_path.to_string_lossy().to_string()],
            false,
            shall::app::sync::guard::Reaped::for_reason(
                shall::app::sync::guard::GuardScope::Remove,
                "a unit test of the effector itself",
            ),
        )
        .await
        .expect("Link purge failed");
}

// `test_metadata_provider_resolution` stood here: it asserted that *apt* answers a dependency
// query with an empty set, "to guard against the expansion being silently re-enabled". It
// guarded one backend against a defect that lived in the planner, and the six other managers
// with a real dependency query were never in its reach. The property it wanted —
// nothing that plans asks a backend what a package depends on — is
// `tests/a_plan_installs_only_declarations_tests.rs`, which holds for every backend including
// the ones not written yet. Two gates for one rule is how the weaker one keeps passing.

// ============================================================================
// REBUILD ORDERING (K1)
// ============================================================================

/// `rebuild` batches per backend, and the order is load-bearing: a user-space package can
/// need a system compiler, and no system package has ever needed a crate. Rebuilding
/// user-space software first would rebuild it against the system state the rebuild is about
/// to replace.
///
/// The rule is expressed as `needs_root()` rather than a hand-kept list of system backends.
/// This asserts it against the registry this machine actually built, so a backend that
/// changes its answer is caught here rather than by an out-of-order rebuild.
#[tokio::test]
async fn rebuild_puts_every_root_backend_before_every_user_backend() {
    let kernel = TestKernel::new().await;
    let registry = kernel.app.registry;

    let names: Vec<String> = registry
        .present_on_this_machine()
        .iter()
        .map(|b| b.name().to_string())
        .collect();
    if names.len() < 2 {
        return; // nothing to order on a host with one backend
    }

    // Deliberately hostile priority: user-space backends first, so a passing result cannot
    // be the input order surviving.
    let mut priority = names.clone();
    priority.sort_by_key(|n| registry.get(n).map(|b| b.needs_root()).unwrap_or(false));

    let is_foundation = |b: &str| registry.get(b).map(|x| x.needs_root()).unwrap_or(false);
    let ordered = shall::app::rebuild::order_backends(&names, &priority, &is_foundation);

    let mut seen_user_backend: Option<String> = None;
    for name in &ordered {
        if is_foundation(name) {
            assert!(
                seen_user_backend.is_none(),
                "{} needs root but was ordered after {}, which does not",
                name,
                seen_user_backend.as_deref().unwrap_or("?")
            );
        } else {
            seen_user_backend = Some(name.clone());
        }
    }
    assert_eq!(ordered.len(), names.len(), "ordering dropped a backend");
}
