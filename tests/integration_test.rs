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

/// What the collector was handed is what the summary is built from.
///
/// **This test asserted nothing.** It recorded one operation, two counts, called
/// `print_summary` twice and ended — the comment on its last step read "verification:
/// print_summary must not panic", which is the whole of what it checked. A collector that
/// dropped every operation on the floor passed it, and so did one whose rollup summed the
/// wrong field.
#[tokio::test]
async fn test_telemetry_metrics_reporting_accuracy() {
    let metrics = MetricsCollector::new();
    let start = Utc::now();

    metrics.record_operation("task-hermetic-1", "apt", start, true, None, 1, 1024, 1);
    metrics.record_operation(
        "task-hermetic-2",
        "apt",
        start,
        false,
        Some("exit 100".into()),
        3,
        0,
        4,
    );
    metrics.record_operation("task-hermetic-3", "brew", start, true, None, 1, 2048, 1);
    metrics.record_install(5);
    metrics.record_remove(2);

    // Every field survives the round trip, including the ones only the summary reads.
    let ops = metrics.operations();
    assert_eq!(ops.len(), 3, "an operation was dropped: {ops:?}");
    let failed = ops
        .iter()
        .find(|o| o.name == "task-hermetic-2")
        .expect("the failed operation was not recorded");
    assert!(!failed.success);
    assert_eq!(failed.error.as_deref(), Some("exit 100"));
    assert_eq!(failed.retry_count, 3, "the retry count is what makes a flaky manager visible");
    assert_eq!(
        failed.batch_size, 4,
        "batch_size is how the summary tells one command covering four packages from four \
         commands, and it is the field with no other reader"
    );

    assert_eq!(
        metrics.totals(),
        (5, 2, 3072),
        "installs, removals and bytes are summed across operations, not taken from the last one"
    );

    // The rollup the summary prints, over the operations actually collected.
    let rollup = linix::app::metrics::backend_rollup(&ops);
    let apt = rollup
        .iter()
        .find(|(b, ..)| b == "apt")
        .expect("apt is missing from the rollup");
    assert_eq!(apt.1, 2, "apt's two operations must roll up as two, not one");
    assert!(
        rollup.iter().any(|(b, ..)| b == "brew"),
        "a second backend must appear in its own row: {rollup:?}"
    );

    // And it still renders, in both narrations — the original test's only real assertion.
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
    let mut options = linix::config::grammar::Options::default();
    options.set("target", target_path.to_string_lossy().to_string(),
    );
    options.set("template", "true".to_string());

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
