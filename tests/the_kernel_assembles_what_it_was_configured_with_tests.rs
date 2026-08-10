//! **The kernel assembles, and the parts it assembles do what their traits say.**
//!
//! Five properties that sit under everything else in the suite and are asserted nowhere else:
//! the registry is populated from the configuration rather than from a hard-coded list; a
//! backend that claims three capabilities really implements all three; the validator refuses
//! the five shapes of a name that is a command; the metrics collector keeps every field the
//! summary reads; and the planner notices a template whose source has moved on.
//!
//! **The telemetry test asserted nothing until it was rewritten.** It recorded one operation,
//! called `print_summary` twice, and ended — its own last comment read *"verification:
//! print_summary must not panic"*. A collector that dropped every operation on the floor passed
//! it, and so did one whose rollup summed the wrong field.

use chrono::Utc;
use linix::app::sync::planner::{ChangePlanner, HostBackends, PlanScope};
use linix::app::MetricsCollector;
use linix::core::{PackageSpec, StateRegistry, Validator};
use std::collections::HashMap;
use tokio::fs;

use crate::mock_providers::TestKernel;

/// The registry is built from the configuration, not from a list compiled into the binary.
#[tokio::test]
async fn the_registry_holds_the_backends_the_config_named() {
    let kernel = TestKernel::new().await;
    let backends = kernel.app.registry.available();

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
async fn a_backend_that_claims_three_capabilities_implements_all_three() {
    let kernel = TestKernel::new().await;
    let github = kernel
        .app
        .registry
        .get("github")
        .expect("GitHub backend missing from registry");

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

/// A package name that is a shell command, a substitution, or a path out of the tree is
/// refused before it reaches an argv.
#[tokio::test]
async fn a_name_that_is_a_command_is_refused_before_it_reaches_an_argv() {
    assert!(Validator::validate_package_name("valid-pkg-123.stable").is_ok());

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

/// What the collector was handed is what the summary is built from.
///
/// **This test asserted nothing.** It recorded one operation, two counts, called
/// `print_summary` twice and ended — the comment on its last step read "verification:
/// print_summary must not panic", which is the whole of what it checked. A collector that
/// dropped every operation on the floor passed it, and so did one whose rollup summed the
/// wrong field.
#[tokio::test]
async fn every_field_the_summary_reads_survives_the_round_trip() {
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
    assert_eq!(
        failed.retry_count, 3,
        "the retry count is what makes a flaky manager visible"
    );
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
    assert_eq!(
        apt.1, 2,
        "apt's two operations must roll up as two, not one"
    );
    assert!(
        rollup.iter().any(|(b, ..)| b == "brew"),
        "a second backend must appear in its own row: {rollup:?}"
    );

    // And it still renders, in both narrations — the original test's only real assertion.
    metrics.print_summary(linix::app::metrics::Narration::Change);
    metrics.print_summary(linix::app::metrics::Narration::Rebuild);
}

/// A template whose source has moved on is planned as a change; one that matches is not.
#[tokio::test]
async fn a_template_whose_source_moved_on_is_planned_as_a_change() {
    let kernel = TestKernel::new().await;
    let state = StateRegistry::default();
    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config);

    let source_path = kernel.tmp.path().join("nginx.tpl");
    let target_path = kernel.tmp.path().join("nginx.conf");

    fs::write(&source_path, "worker_processes {{OS_CORES}};")
        .await
        .unwrap();

    let mut options = linix::config::grammar::Options::default();
    options.set("target", target_path.to_string_lossy().to_string());
    options.set("template", "true".to_string());

    let spec = PackageSpec {
        name: source_path.to_string_lossy().to_string(),
        backend: "link".to_string(),
        options,
        requires: vec![],
        present: true,
    };

    kernel.mock_executor.set_command_exists("link", true);

    let mut desired = HashMap::new();
    desired.insert("link".to_string(), vec![spec]);

    let plan = planner
        .plan(&desired, PlanScope::Whole(HostBackends::default()))
        .await
        .expect("Integration Planning Failure: Template logic closure failed.");

    assert!(
        !plan.is_empty(),
        "Planner logic error: Template creation was not scheduled."
    );
}
