use linix::app::sync::planner::{ChangePlanner, Scope};
use linix::app::sync::resolver::StateResolver;
use linix::core::{Error, PackageSpec};
use std::collections::HashMap;
use tokio::fs;

// Import our exhaustive A+ Test Infrastructure
mod mock_providers;
use mock_providers::TestKernel;

// ============================================================================
// FEATURE 3: NAMED MODULES (@module: recursive expansion)
// ============================================================================

/// Verifies that the Resolver correctly unrolls nested @module references
/// into a flat list of PackageSpecs.
///
/// This test confirms that:
/// 1. Modules are correctly identified by the @module: prefix.
/// 2. Deep recursive nesting is handled without cycles.
/// 3. Source metadata is correctly attached for Feature 4 scoping.
#[tokio::test]
async fn test_recursive_module_expansion_logic() {
    let kernel = TestKernel::new().await;

    // 1. Setup Module A (The Leaf module)
    let mod_a_path = kernel.app.config.modules_dir.join("network.module.txt");
    fs::create_dir_all(&kernel.app.config.modules_dir)
        .await
        .unwrap();
    fs::write(&mod_a_path, "brew:curl\nbrew:wget")
        .await
        .unwrap();

    // 2. Setup Module B (The Recursive module)
    let mod_b_path = kernel.app.config.modules_dir.join("bundle.module.txt");
    fs::write(&mod_b_path, "@module:network\nbrew:git")
        .await
        .unwrap();

    // 3. Setup a primary manifest utilizing Module B
    let manifest_path = kernel.app.config.groups_dir.join("main.txt");
    fs::create_dir_all(&kernel.app.config.groups_dir)
        .await
        .unwrap();
    fs::write(&manifest_path, "@module:bundle").await.unwrap();

    // 4. Resolve System State
    // Modernized: Await async constructor and provide explicit locked=false
    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let desired = resolver
        .resolve_desired_state()
        .await
        .expect("State resolution failed");

    // 5. Verification of the expanded closure
    let brew_specs = desired
        .get("brew")
        .expect("Missing brew backend specs in resolution map");
    let names: Vec<&str> = brew_specs.iter().map(|s| s.name.as_str()).collect();

    // Assert that the deep recursion reached the leaf packages
    assert!(
        names.contains(&"curl"),
        "Resolver failed to expand nested leaf 'curl' from Module A"
    );
    assert!(
        names.contains(&"wget"),
        "Resolver failed to expand nested leaf 'wget' from Module A"
    );
    assert!(
        names.contains(&"git"),
        "Resolver failed to expand direct member 'git' from Module B"
    );
    assert_eq!(
        names.len(),
        3,
        "Expanded closure count mismatch. Expected 3 packages."
    );

    // Assert that the source metadata is correctly tagged (Feature 4 requirement)
    let curl_spec = brew_specs.iter().find(|s| s.name == "curl").unwrap();
    assert_eq!(curl_spec.options.get("__source").unwrap(), "module:network");
}

/// Verifies that references to non-existent modules result in a descriptive
/// Config error rather than a silent failure or panic.
#[tokio::test]
async fn test_module_resolution_failure_handling() {
    let kernel = TestKernel::new().await;

    let manifest_path = kernel.app.config.groups_dir.join("fail.txt");
    fs::create_dir_all(&kernel.app.config.groups_dir)
        .await
        .unwrap();
    fs::write(&manifest_path, "@module:ghost-module-123")
        .await
        .unwrap();

    // Modernized: Resolve E0061/E0599
    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let result = resolver.resolve_desired_state().await;

    assert!(
        result.is_err(),
        "Resolution should have failed for a missing module reference"
    );
    if let Err(Error::Config(msg)) = result {
        assert!(
            msg.contains("ghost-module-123"),
            "Error message should identify the specific missing module"
        );
    } else {
        panic!("Incorrect error type returned: {:?}", result.err());
    }
}

// ============================================================================
// FEATURE 1: STRUCTURED JSON DRY-RUN REPORTING
// ============================================================================

/// Verifies that the ChangePlanner generates a SyncReport with accurate fields
/// suitable for high-fidelity JSON serialization.
#[tokio::test]
async fn test_sync_report_generation_schema_fidelity() {
    let kernel = TestKernel::new().await;
    let state_guard = kernel.state.lock().await;

    let planner = ChangePlanner::new(
        kernel.app.registry.clone(),
        &state_guard,
        &kernel.app.config,
    );

    // Seed a desired state containing source metadata
    let mut desired = HashMap::new();
    desired.insert(
        "brew".to_string(),
        vec![PackageSpec {
            name: "ripgrep".into(),
            backend: "brew".into(),
            options: HashMap::from([("__source".into(), "module:dev-tools".into())]),
            requires: vec![],
        }],
    );

    // Plan with None (Global Sync)
    let plan = planner.plan(&desired, None).await.unwrap();
    let report = plan.generate_report();

    // 1. Verify business logic mapping to the Report structure
    assert_eq!(report.change_count, 1);
    assert_eq!(report.install.len(), 1);
    assert_eq!(report.install[0].name, "ripgrep");
    assert_eq!(
        report.install[0].source,
        Some("module:dev-tools".to_string())
    );

    // 2. Verify JSON Serialization (Schema Integrity)
    let json_output = serde_json::to_string(&report).expect("SyncReport failed JSON serialization");
    assert!(
        json_output.contains("\"change_count\":1"),
        "JSON missing change_count field"
    );
    assert!(
        json_output.contains("\"name\":\"ripgrep\""),
        "JSON missing package name"
    );
    assert!(
        json_output.contains("\"source\":\"module:dev-tools\""),
        "JSON missing source metadata"
    );
}

// ============================================================================
// FEATURE 4: SCOPED UPGRADE LOGIC
// ============================================================================

/// Verifies that a Scope correctly prunes the DAG to only include
/// nodes matching the requested source origin.
#[tokio::test]
async fn test_scoped_planner_filtering_accuracy() {
    let kernel = TestKernel::new().await;
    let state_guard = kernel.state.lock().await;
    let planner = ChangePlanner::new(
        kernel.app.registry.clone(),
        &state_guard,
        &kernel.app.config,
    );

    // Setup a mixed desired state
    let mut desired = HashMap::new();
    desired.insert(
        "brew".to_string(),
        vec![
            PackageSpec {
                name: "pkg-work".into(),
                backend: "brew".into(),
                options: HashMap::from([("__source".into(), "manifest:work.txt".into())]),
                requires: vec![],
            },
            PackageSpec {
                name: "pkg-home".into(),
                backend: "brew".into(),
                options: HashMap::from([("__source".into(), "manifest:home.txt".into())]),
                requires: vec![],
            },
        ],
    );

    // Execute Plan for Scope: "work.txt"
    let plan = planner
        .plan(&desired, Some(Scope::Profile("work.txt".into())))
        .await
        .unwrap();

    // Verification
    assert_eq!(
        plan.total_install(),
        1,
        "Planner failed to prune the graph based on scope"
    );
    let report = plan.generate_report();
    assert_eq!(report.install[0].name, "pkg-work");
    assert!(
        !report.install.iter().any(|r| r.name == "pkg-home"),
        "Package from outside the scope (home.txt) was incorrectly included in the plan"
    );
}

// ============================================================================
// BUG FIX 3: --LOCKED MODE INTEGRITY
// ============================================================================

/// Verifies that Locked Mode prevents resolution if manifest versions deviate
/// from the cryptographically tracked locks.json.
#[tokio::test]
async fn test_locked_mode_version_conflict_enforcement() {
    let kernel = TestKernel::new().await;

    // 1. Setup a lock file with version 1.2.3
    let lock_content = r#"{ "locks": { "brew:vim": "1.2.3" } }"#;
    fs::create_dir_all(&kernel.app.config.groups_dir)
        .await
        .unwrap();
    fs::write(
        kernel.app.config.groups_dir.join("locks.json"),
        lock_content,
    )
    .await
    .unwrap();

    // 2. Setup a manifest requesting a conflicting version 2.0.0
    let manifest_path = kernel.app.config.groups_dir.join("main.txt");
    fs::write(&manifest_path, "brew:vim@version=2.0.0")
        .await
        .unwrap();

    // 3. Resolve in Locked Mode (locked = true)
    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), true).await;
    let result = resolver.resolve_desired_state().await;

    // 4. Assert Failure
    assert!(
        result.is_err(),
        "Resolver should have rejected the version mismatch in locked mode"
    );
    if let Err(Error::Validation(msg)) = result {
        assert!(
            msg.contains("version mismatch"),
            "Incorrect validation error message: {}",
            msg
        );
        assert!(
            msg.contains("brew:vim"),
            "Error should identify the offending package"
        );
    }
}
