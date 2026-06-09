use linix::app::{App, MetricsCollector};
use linix::config::Config;
use linix::core::{PackageSpec, Validator, StateRegistry};
use linix::core::executor::{MockExecutor, DryRunOutput};
use linix::app::sync::planner::ChangePlanner;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::tempdir;
use chrono::Utc;

/// Phase 5.1 Hardening: Helper to create a fully isolated App for integration tests.
/// This prevents noisy system calls during testing.
async fn create_isolated_test_app() -> (App, Arc<MockExecutor>) {
    let tmp = tempdir().unwrap();
    let registry_path = tmp.path().join("registry.json");
    StateRegistry::set_test_path(registry_path);

    let mut config = Config::default();
    config.dry_run = true;
    config.groups_dir = tmp.path().join("groups");

    let mock_layer = Arc::new(MockExecutor::new());
    mock_layer.set_command_exists("code", true);
    mock_layer.set_command_exists("brew", true);
    mock_layer.set_command_exists("sc", true);
    mock_layer.set_command_exists("cargo", true);

    let app = App::new(config).await.expect("Failed to init isolated app");
    
    (app, mock_layer)
}

#[tokio::test]
async fn test_app_initialization_v3_assemble() {
    let (app, _) = create_isolated_test_app().await;
    let backends = app.registry.available();
    assert!(!backends.is_empty(), "No backends discovered in isolated context.");
}

#[tokio::test]
async fn test_backend_capability_discovery_solid_wiring() {
    let (app, _) = create_isolated_test_app().await;
    let github = app.registry.get("github").expect("GitHub backend missing from registry");
    
    assert!(github.is_installable(), "GitHub must implement Installable");
    assert!(github.is_queryable(), "GitHub must implement Queryable");
    assert!(github.is_metadata_provider(), "GitHub must implement MetadataProvider");
}

#[tokio::test]
async fn test_security_validator_strict() {
    assert!(Validator::validate_package_name("valid-pkg-123.stable").is_ok());
    
    let dangerous_inputs = vec![
        "pkg; rm -rf /",
        "pkgname$(whoami)",
        "../../etc/passwd",
        "../traversal",
    ];
    
    for input in dangerous_inputs {
        let res = Validator::validate_package_name(input);
        assert!(res.is_err(), "Security vulnerability: Validator failed to block: {}", input);
    }
}

#[tokio::test]
async fn test_telemetry_metrics_updated_signature() {
    let metrics = MetricsCollector::new();
    let start = Utc::now();

    metrics.record_operation("task1", "apt", start, true, None, 1, 2048);
    metrics.print_summary();
}

#[tokio::test]
async fn test_planner_template_logic_integration() {
    let (app, mock_layer) = create_isolated_test_app().await;
    let state = StateRegistry::default();
    let planner = ChangePlanner::new(app.registry.clone(), &state, &app.config);
    
    let tmp = tempdir().unwrap();
    let source_path = tmp.path().join("source.tpl");
    let target_path = tmp.path().join("target.txt");
    
    tokio::fs::write(&source_path, "hello world").await.unwrap();
    
    let mut options = HashMap::new();
    options.insert("target".to_string(), target_path.to_string_lossy().to_string());
    options.insert("template".to_string(), "true".to_string());
    
    let spec = PackageSpec {
        name: source_path.to_string_lossy().to_string(),
        backend: "link".to_string(),
        options,
        requires: vec![],
    };
    
    mock_layer.set_command_exists("link", true);
    let mut desired = HashMap::new();
    desired.insert("link".to_string(), vec![spec]);
    
    let plan = planner.plan(&desired).await.expect("Planning failed");
    assert!(!plan.is_empty(), "Planner should identify that template needs creation");
}

#[tokio::test]
async fn test_teleport_api_consistency() {
    let (app, mock_layer) = create_isolated_test_app().await;
    let teleporter = app.teleporter();
    
    // Mocking for Teleport logic path: Ensure Source backends return empty
    mock_layer.set_response("brew list --versions", Ok(DryRunOutput::default().into()));
    mock_layer.set_response("cargo list", Ok(DryRunOutput::default().into()));
    mock_layer.set_response("sc query type= service state= active", Ok(DryRunOutput::default().into()));

    // Teleport should now correctly return Err(PackageNotFound) instead of Ok(())
    let result = teleporter.teleport("nonexistent-package-xyz", "cargo").await;
    assert!(result.is_err(), "Teleport should have failed for a package that does not exist in any backend");
}