// tests/hardening_tests.rs
//
// Integration coverage for behavior added/changed during the v5 hardening pass:
//  - resolver merges multiple sources for one package (was first-write-wins)
//  - scoped `upgrade` is non-destructive end-to-end (resolver -> planner)
//  - RepoManager dispatch issues the right backend command
//
// All hermetic via the shared TestKernel (MockExecutor + temp sandbox); OS-independent,
// so these also exercise the relevant paths on Linux at CI time.

mod mock_providers;
use mock_providers::TestKernel;

use linix::app::sync::planner::{ChangePlanner, ScopedFilter};
use linix::app::sync::resolver::StateResolver;
use linix::core::executor::DryRunOutput;
use linix::core::PackageSpec;
use std::collections::HashMap;

/// Build a PackageSpec with a pinned version option.
fn pinned_spec(backend: &str, name: &str, version: &str) -> PackageSpec {
    let mut options = HashMap::new();
    options.insert("version".to_string(), version.to_string());
    PackageSpec { name: name.into(), backend: backend.into(), options, requires: vec![] }
}

/// A package listed in a manifest AND pulled in by a module must end up tagged with
/// BOTH origins, so it stays visible to every scope it belongs to.
#[tokio::test]
async fn resolver_merges_manifest_and_module_sources() {
    let kernel = TestKernel::new().await;
    let cfg = &kernel.app.config;
    tokio::fs::create_dir_all(&cfg.groups_dir).await.unwrap();
    tokio::fs::create_dir_all(&cfg.modules_dir).await.unwrap();
    // base manifest lists the package directly and also references the @dev module,
    // which lists the same package.
    tokio::fs::write(cfg.groups_dir.join("base.txt"), "cargo:ripgrep\n@module:dev\n").await.unwrap();
    tokio::fs::write(cfg.modules_dir.join("dev.module.txt"), "cargo:ripgrep\n").await.unwrap();

    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let desired = resolver.resolve_desired_state().await.unwrap();

    let spec = desired
        .get("cargo")
        .and_then(|specs| specs.iter().find(|s| s.name == "ripgrep"))
        .expect("ripgrep should be resolved under cargo");

    let source = spec.options.get("__source").expect("__source should be tagged");
    let segments: Vec<&str> = source.split(';').collect();
    assert!(segments.contains(&"manifest:base.txt"), "missing manifest origin in {:?}", source);
    assert!(segments.contains(&"module:dev"), "missing module origin in {:?}", source);
}

/// End-to-end: a targeted `upgrade --module dev` must never schedule removals for managed
/// packages outside that scope — while an UNSCOPED plan would remove the same drift.
#[tokio::test]
async fn scoped_upgrade_is_non_destructive_end_to_end() {
    let kernel = TestKernel::new().await;
    let cfg = &kernel.app.config;
    tokio::fs::create_dir_all(&cfg.groups_dir).await.unwrap();
    tokio::fs::create_dir_all(&cfg.modules_dir).await.unwrap();
    tokio::fs::write(cfg.groups_dir.join("base.txt"), "@module:dev\n").await.unwrap();
    tokio::fs::write(cfg.modules_dir.join("dev.module.txt"), "cargo:ripgrep\n").await.unwrap();

    // A managed package that is NOT in any manifest/module == drift.
    {
        let mut state = kernel.app.state.lock().await;
        state.add("cargo", "out-of-scope-pkg", None, HashMap::new(), Some("manifest:other".into()), false);
    }

    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let desired = resolver.resolve_desired_state().await.unwrap();

    // Scoped plan: zero removals.
    let scoped = {
        let state = kernel.app.state.lock().await;
        let planner = ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config);
        planner.plan(&desired, ScopedFilter::Module("dev".into())).await.unwrap()
    };
    assert_eq!(scoped.total_remove(), 0, "scoped upgrade must not remove out-of-scope packages");

    // Unscoped plan: the same drift IS scheduled for removal (proves the guard is what
    // prevents it, not e.g. protection).
    let unscoped = {
        let state = kernel.app.state.lock().await;
        let planner = ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config);
        planner.plan(&desired, ScopedFilter::None).await.unwrap()
    };
    assert!(unscoped.total_remove() >= 1, "unscoped sync should remove the drift package");
}

/// A backend's RepoManager must issue the backend's real "add source" command.
#[tokio::test]
async fn repo_manager_dispatches_add_command() {
    let kernel = TestKernel::new().await;
    kernel.mock_executor.set_command_exists("gem", true);

    let gem = kernel.app.registry.get("gem").expect("gem should be registered");
    let repo = gem.as_repo_manager().expect("gem should support RepoManager");
    repo.add_repo("myrepo", "https://gems.example.com/", false).await.unwrap();

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls.iter().any(|c| c.contains("sources -a https://gems.example.com/")),
        "expected a `gem sources -a <url>` call, got: {:?}",
        calls
    );
}

/// `unmanaged` reports installed packages not under LiNix management (and excludes
/// managed ones).
#[tokio::test]
async fn unmanaged_lists_installed_but_unmanaged() {
    let kernel = TestKernel::new().await;
    // cargo reports two installed crates...
    kernel.mock_executor.set_response(
        "cargo install --list",
        Ok(DryRunOutput {
            stdout: b"ripgrep v13.0.0:\n    rg\nexa v0.10.1:\n    exa\n".to_vec(),
            stderr: vec![],
        }
        .into()),
    );
    // ...but only ripgrep is under management.
    {
        let mut state = kernel.app.state.lock().await;
        state.add("cargo", "ripgrep", None, HashMap::new(), Some("manifest:base".into()), false);
    }

    let unmanaged = kernel.app.get_unmanaged_packages().await.unwrap();
    assert!(
        unmanaged.iter().any(|p| p.backend == "cargo" && p.name == "exa"),
        "exa should be reported as unmanaged, got: {:?}",
        unmanaged.iter().map(|p| (&p.backend, &p.name)).collect::<Vec<_>>()
    );
    assert!(
        !unmanaged.iter().any(|p| p.name == "ripgrep"),
        "ripgrep is managed and must not be listed as unmanaged"
    );
}

/// Reproducible installs: a pinned version reaches the backend command in its native
/// syntax — inline (`pip install requests==2.31.0`) for generic backends, and a flag
/// (`cargo install ripgrep --version 13.0.0`) for cargo.
#[tokio::test]
async fn pinned_version_reaches_install_command() {
    let kernel = TestKernel::new().await;

    // generic inline pin (pip == syntax)
    let pip = kernel.app.registry.get("pip").expect("pip registered");
    pip.as_installable().unwrap()
        .install(&[pinned_spec("pip", "requests", "2.31.0")], false)
        .await
        .unwrap();

    // bespoke flag pin (cargo --version)
    let cargo = kernel.app.registry.get("cargo").expect("cargo registered");
    cargo.as_installable().unwrap()
        .install(&[pinned_spec("cargo", "ripgrep", "13.0.0")], false)
        .await
        .unwrap();

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls.iter().any(|c| c.contains("install requests==2.31.0")),
        "pip should pin with ==, got: {:?}", calls
    );
    assert!(
        calls.iter().any(|c| c.contains("install ripgrep --version 13.0.0")),
        "cargo should pin with --version, got: {:?}", calls
    );
}

/// A floating version ("latest") must NOT be pinned — it installs the bare name.
#[tokio::test]
async fn floating_version_is_not_pinned() {
    let kernel = TestKernel::new().await;
    let pip = kernel.app.registry.get("pip").unwrap();
    pip.as_installable().unwrap()
        .install(&[pinned_spec("pip", "requests", "latest")], false)
        .await
        .unwrap();
    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls.iter().any(|c| c.contains("install requests") && !c.contains("==")),
        "latest should install bare name, got: {:?}", calls
    );
}
