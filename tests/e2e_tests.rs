use linix::app::sync::planner::ChangePlanner;
use linix::app::sync::resolver::StateResolver;
use linix::core::executor::DryRunOutput;
use linix::core::{GraphAction, PackageSpec, Transaction, TransactionConfig};
use std::collections::HashMap;
use tokio::fs;

// Import our authoritative A+ Test Infrastructure
mod mock_providers;
use mock_providers::TestKernel;

// ============================================================================
// E2E LOGIC TESTS: DECLARATIVE SYNC FLOW
// ============================================================================

/// Verifies the full LiNix system lifecycle closure:
/// Manifest Creation -> Resolution -> Planning -> Parallel Execution -> State Update.
#[tokio::test]
async fn test_e2e_sync_flow_hermetic() {
    // 1. Initialize hermetic test environment (DI + Async Bootstrap)
    let kernel = TestKernel::new().await;

    // 2. Setup: a module holding the package, and a profile that reaches it. A module
    //    nothing activates is inert by design — profiles choose, modules hold — so the
    //    profile and the `active` line are not ceremony here, they are the thing under
    //    test. 'brew' is our universal mock identifier.
    let root = kernel.app.config.config_root();
    fs::write(root.join("modules/workstation.txt"), "brew:neovim\n")
        .await
        .unwrap();
    fs::write(root.join("profiles/Work"), "use workstation\n")
        .await
        .unwrap();
    fs::write(root.join("active"), "Work\n").await.unwrap();

    // 3. Resolution Phase: Transform manifest strings into PackageSpecs
    // Modernized v3.6.0: Await async constructor and provide explicit locked=false
    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let desired = resolver
        .resolve_desired_state()
        .await
        .expect("E2E Resolution Error: Manifest closure expansion failed.");

    // 4. Planning Phase: Calculate the system delta
    let changes = {
        let state_guard = kernel.state.lock().await;
        let planner = ChangePlanner::new(
            kernel.app.registry.clone(),
            &state_guard,
            &kernel.app.config,
        );
        // None handles global system reconciliation
        planner
            .plan(&desired, None)
            .await
            .expect("E2E Planning Error: Failed to generate SyncChanges DAG.")
    };

    // 5. Prime Mocks: Set expected CLI output for the executor
    kernel
        .mock_executor
        .set_response("brew install neovim", Ok(DryRunOutput::default().into()));

    // 6. Execution Phase: Apply the transaction closure
    // Modernized v3.6.0: Uses the Kernel's sync_engine factory to ensure 10-arg DI is correct
    let engine = kernel.app.sync_engine().await;
    let result = engine
        .sync(changes, linix::app::sync::guard::GuardScope::Sync)
        .await;

    assert!(
        result.is_ok(),
        "E2E Transaction Logic Failed: {:?}",
        result.err()
    );

    // 7. Verification: Consolidation check in the mission-critical registry
    let state = kernel.state.lock().await;
    assert!(
        state.is_managed("brew", "neovim"),
        "Integrity Failure: 'neovim' missing from registry post-transaction."
    );
}

// ============================================================================
// E2E LOGIC TESTS: CONCURRENCY & PARALLEL INTEGRITY
// ============================================================================

/// Verifies that high-breadth parallel transactions execute without deadlocks
/// and correctly share the kernel-wide Diagnostic Engine.
#[tokio::test]
async fn test_concurrent_transaction_safety_e2e() {
    let kernel = TestKernel::new().await;

    // 1. Build a high-throughput parallel DAG (5 independent nodes)
    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    for i in 0..5 {
        let pkg_name = format!("pkg-parallel-{}", i);
        let spec = PackageSpec {
            name: pkg_name.clone(),
            backend: "brew".into(),
            options: HashMap::new(),
            requires: vec![],
            present: true,
        };

        // Setup expected responses in mock layer
        kernel.mock_executor.set_response(
            &format!("brew install {}", pkg_name),
            Ok(DryRunOutput::default().into()),
        );

        graph.add_node(GraphAction::Install(spec));
    }

    // 2. Initialize Transaction
    // Modernized v3.6.0: Provides Diagnostics (4th arg) and Config (5th arg)
    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(), // DI
        TransactionConfig::default(),
    );

    // 3. Execute with telemetry
    let result = tx.execute_with_telemetry().await;

    // 4. Verification
    assert!(
        result.is_ok(),
        "Concurrent parallel transaction failed: {:?}",
        result.err()
    );
    let telemetry = result.expect("Telemetry record missing");
    assert_eq!(
        telemetry.len(),
        5,
        "Not all parallel nodes reached terminal success."
    );
}
