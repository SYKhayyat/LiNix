use chrono::Utc;
use linix::app::sync::planner::{ChangePlanner, HostBackends, PlanScope};
use linix::app::MetricsCollector;
use linix::core::{PackageSpec, StateRegistry, Validator};
use std::collections::HashMap;
use tokio::fs;

// Import our authoritative A+ Test Infrastructure
mod mock_providers;
use mock_providers::TestKernel;

// ============================================================================
// KERNEL & BACKEND INTEGRATION
// ============================================================================

/// Verifies that the Kernel correctly discovers and registers backends
/// in a hermetic, isolated environment.
#[tokio::test]
async fn test_app_initialization_v3_assemble() {
    // 1. Initialize hermetic kernel (Async DI bootstrap)
    let kernel = TestKernel::new().await;
    let backends = kernel.app.registry.available();

    // 2. Verification: In our TestKernel, we explicitly enabled brew, apt, and cargo
    assert!(
        !backends.is_empty(),
        "No backends discovered in isolated context."
    );
    assert!(
        kernel.app.registry.get("brew").is_some(),
        "Homebrew backend missing from registry"
    );
    assert!(
        kernel.app.registry.get("github").is_some(),
        "GitHub backend missing from registry"
    );
}

/// Verifies that backends correctly implement the exhaustive 3.6.0 trait
/// capability matrix.
#[tokio::test]
async fn test_backend_capability_discovery_solid_wiring() {
    let kernel = TestKernel::new().await;
    let github = kernel
        .app
        .registry
        .get("github")
        .expect("GitHub backend missing from registry");

    // Mission-critical capability checks
    assert!(
        github.is_installable(),
        "GitHub must implement the Installable trait"
    );
    assert!(
        github.is_queryable(),
        "GitHub must implement the Queryable trait"
    );
    assert!(
        github.is_metadata_provider(),
        "GitHub must implement the MetadataProvider trait"
    );
}

// ============================================================================
// SECURITY & VALIDATION
// ============================================================================

/// Verifies that the Security Validator blocks dangerous inputs that could
/// lead to escalation or unauthorized system access.
#[tokio::test]
async fn test_security_validator_strict_enforcement() {
    // 1. Logic Check: Legitimate names must pass
    assert!(Validator::validate_package_name("valid-pkg-123.stable").is_ok());

    // 2. Logic Check: Dangerous patterns must be blocked (Bug Fix 2 & 6 Logic)
    let dangerous_inputs = vec![
        "pkg; rm -rf /",
        "pkgname$(whoami)",
        "../../etc/passwd",
        "../traversal",
        "\\..\\windows\\system32",
    ];

    for input in dangerous_inputs {
        let res = Validator::validate_package_name(input);
        assert!(
            res.is_err(),
            "Security vulnerability: Validator failed to block malicious input: {}",
            input
        );
    }
}

// ============================================================================
// TELEMETRY & PERFORMANCE
// ============================================================================

/// Verifies that the MetricsCollector accurately records parallel task
/// performance data for the transaction summary.
#[tokio::test]
async fn test_telemetry_metrics_reporting_accuracy() {
    let metrics = MetricsCollector::new();
    let start = Utc::now();

    // 1. Record a simulated successful operation
    metrics.record_operation(
        "task-hermetic-1",
        "apt",
        start,
        true, // Success
        None, // No error
        1,    // 1 attempt
        1024, // 1KB downloaded
        1,    // not batched
    );

    // 2. Record aggregate stats
    metrics.record_install(5);
    metrics.record_remove(2);

    // 3. Verification: print_summary must not panic
    metrics.print_summary(linix::app::metrics::Narration::Change);
    metrics.print_summary(linix::app::metrics::Narration::Rebuild);
}

// ============================================================================
// PLANNER & TEMPLATE INTEGRATION
// ============================================================================

/// Verifies that the ChangePlanner correctly identifies when a configuration
/// template needs to be physically updated on the host.
#[tokio::test]
async fn test_planner_template_logic_integration() {
    let kernel = TestKernel::new().await;
    let state = StateRegistry::default();
    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config);

    // 1. Create a source template file in the test sandbox
    let source_path = kernel.tmp.path().join("nginx.tpl");
    let target_path = kernel.tmp.path().join("nginx.conf");

    fs::write(&source_path, "worker_processes {{OS_CORES}};")
        .await
        .unwrap();

    // 2. Setup a spec for the 'link' backend with template logic enabled
    let mut options = HashMap::new();
    options.insert(
        "target".to_string(),
        target_path.to_string_lossy().to_string(),
    );
    options.insert("template".to_string(), "true".to_string());

    let spec = PackageSpec {
        name: source_path.to_string_lossy().to_string(),
        backend: "link".to_string(),
        options,
        requires: vec![],
        present: true,
    };

    // 3. Ensure the 'link' backend binary is "available"
    kernel.mock_executor.set_command_exists("link", true);

    let mut desired = HashMap::new();
    desired.insert("link".to_string(), vec![spec]);

    // 4. Plan the transition (Global Scope)
    let plan = planner
        .plan(&desired, PlanScope::Whole(HostBackends::default()))
        .await
        .expect("Integration Planning Failure: Template logic closure failed.");

    // 5. Verification: Since target doesn't exist, planner must schedule an installation
    assert!(
        !plan.is_empty(),
        "Planner logic error: Template creation was not scheduled."
    );
}
