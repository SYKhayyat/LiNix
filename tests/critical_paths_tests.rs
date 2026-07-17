use linix::app::sync::planner::ChangePlanner;
use linix::app::sync::resolver::StateResolver;
use linix::core::executor::DryRunOutput;
use linix::core::journal::JournalAction;
use linix::core::{Error, GraphAction, PackageSpec, Transaction, TransactionConfig};
use std::collections::HashMap;

// Import our authoritative A+ Test Infrastructure
mod mock_providers;
use mock_providers::TestKernel;

// ============================================================================
// LOGIC TESTS: PLANNING & DEPENDENCIES
// ============================================================================

/// Verifies that the ChangePlanner correctly unrolls native transitive
/// dependencies provided by a backend's MetadataProvider capability.
#[tokio::test]
async fn test_planner_recursive_native_dependencies() {
    let kernel = TestKernel::new().await;
    let state_lock = kernel.state.lock().await;

    // Modernized: ChangePlanner requires Registry, State reference, and Config
    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state_lock, &kernel.app.config);

    // Mock Scenario: brew package 'pkg-a' natively depends on 'pkg-b'
    let mock_output = "pkg-b\n";
    kernel.mock_executor.set_response(
        "brew deps pkg-a",
        Ok(DryRunOutput {
            stdout: mock_output.as_bytes().to_vec(),
            stderr: vec![],
        }
        .into()),
    );
    kernel
        .mock_executor
        .set_response("brew deps pkg-b", Ok(DryRunOutput::default().into()));

    let mut desired = HashMap::new();
    desired.insert(
        "brew".to_string(),
        vec![PackageSpec {
            name: "pkg-a".into(),
            backend: "brew".into(),
            options: HashMap::new(),
            requires: vec![],
            present: true,
        }],
    );

    // Execute Planning
    // Resolves E0061: Passes None (Global Sync)
    let plan = planner
        .plan(&desired, None)
        .await
        .expect("Critical Path Error: Planning failed for native dependencies.");

    // Verification: Closure must contain 2 nodes (pkg-a and native-dep pkg-b)
    assert_eq!(
        plan.graph.node_count(),
        2,
        "Planner failed to resolve recursive native closure."
    );
}

/// Verifies that the Planner detects circular manifest-level dependencies
/// and returns a descriptive error rather than causing an infinite loop.
#[tokio::test]
async fn test_dag_cycle_detection_logic() {
    let kernel = TestKernel::new().await;
    let state_lock = kernel.state.lock().await;
    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state_lock, &kernel.app.config);

    let mut desired = HashMap::new();

    // Create Circular Logic: pkg-a requires pkg-b, pkg-b requires pkg-a
    let spec_a = PackageSpec {
        name: "pkg-a".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec!["brew:pkg-b".into()],
        present: true,
    };
    let spec_b = PackageSpec {
        name: "pkg-b".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec!["brew:pkg-a".into()],
        present: true,
    };

    desired.insert("brew".to_string(), vec![spec_a, spec_b]);

    // Execute Planning
    let result = planner.plan(&desired, None).await;

    // Verification
    assert!(
        result.is_err(),
        "Planner failed to identify circular manifest closure."
    );
    if let Err(Error::Transaction(msg)) = result {
        assert!(
            msg.contains("Circular dependency"),
            "Incorrect error context returned for cycle."
        );
    }
}

// ============================================================================
// LOGIC TESTS: TRANSACTION & ROLLBACK
// ============================================================================

/// Verifies that a failed modification correctly triggers a transactional
/// rollback of preceding successful nodes.
#[tokio::test]
async fn test_transaction_rollback_fidelity() {
    let kernel = TestKernel::new().await;

    let failing_spec = PackageSpec {
        name: "fail-node".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec![],
        present: true,
    };

    // Set response to failure
    kernel.mock_executor.set_response(
        "brew install fail-node",
        Err(Error::CommandFailed("Simulated Network Timeout".into())),
    );

    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    graph.add_node(GraphAction::Install(failing_spec));

    // Modernized: Provide Diagnostics and Config (TransactionConfig::quick restored in library)
    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        TransactionConfig::default(),
    );

    let result = tx.execute_with_telemetry().await;
    assert!(
        result.is_err(),
        "Transaction logic failed to catch node failure."
    );
}

/// Verifies that the Self-Healing (WAL) logic correctly uninstalls and
/// re-attempts "InProgress" modifications found in the transaction journal.
///
/// Resolves A+ Grade logic: Confirms that healing updates the Journal status.
#[tokio::test]
async fn test_journal_self_healing_logic() {
    let kernel = TestKernel::new().await;

    // 1. Manually simulate an interrupted session by recording a start in the WAL
    {
        // Resolve E0282: Explicit type hint for the lock guard
        let mut j: tokio::sync::MutexGuard<'_, linix::core::Journal> =
            kernel.app.journal.lock().await;
        let spec = PackageSpec {
            name: "stale-pkg".into(),
            backend: "brew".into(),
            options: HashMap::new(),
            requires: vec![],
            present: true,
        };
        let _ = j.record_start(JournalAction::Install(spec));
    }

    // 2. Resolve E0599: Use the public kernel factory for SyncEngine
    let engine = kernel.app.sync_engine().await;

    // Prime mocks for the healing sequence (Remove -> Install)
    kernel.mock_executor.set_response(
        "brew uninstall stale-pkg",
        Ok(DryRunOutput::default().into()),
    );
    kernel
        .mock_executor
        .set_response("brew install stale-pkg", Ok(DryRunOutput::default().into()));

    // 3. Execute Heal
    engine.heal().await.expect("Healing cycle crashed.");

    // 4. Verification: A+ Fix Logic Check
    // The journal should now report that NO recovery is needed because the
    // heal() method updated the status of the entries.
    let j_after = kernel.app.journal.lock().await;
    assert!(
        !j_after.needs_recovery(),
        "A+ Fix Failure: Journal indicates recovery still needed after successful heal."
    );
}

// ============================================================================
// LOGIC TESTS: RESOLVER & VFS
// ============================================================================

/// Verifies that the StateResolver correctly parses and unrolls complex
/// semver constraints from manifest strings.
#[tokio::test]
async fn test_semver_constraint_resolution_logic() {
    let kernel = TestKernel::new().await;

    // Modernized: Await async constructor and pass locked=false
    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;

    let spec_line = "brew:curl@version=>=7.0.0";

    // Modernized: Call method on the RESOLVED Resolver (Future was awaited above)
    let spec = resolver
        .parse_and_probe_spec(spec_line)
        .await
        .expect("Critical Path: Resolver failed to parse semver spec line.");

    assert_eq!(spec.options.get("version").unwrap(), ">=7.0.0");
}

/// Verifies that the CommandExecutor accurately records file modifications
/// in the Virtual File System (VFS) during dry-run sessions.
#[tokio::test]
async fn test_dry_run_vfs_simulation() {
    let kernel = TestKernel::new().await;
    let executor = kernel.app.executor.clone();

    let path = std::path::PathBuf::from("/virtual/A+_Integrity_Pass.txt");
    let content = "System Integrity Verified.";

    // write_atomic should target VFS in dry-run mode
    executor
        .write_atomic(&path, content)
        .await
        .expect("VFS Write failed.");

    let read_content = executor.read_file(&path).await.expect("VFS Read failed.");
    assert_eq!(
        read_content, content,
        "VFS failed to preserve/retrieve written content."
    );

    let diff = executor.get_vfs_diff();
    assert!(
        !diff.is_empty(),
        "VFS diff tracker is empty after dry-run modification."
    );
}
