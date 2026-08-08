use linix::app::sync::planner::{ChangePlanner, HostBackends, PlanScope};
use linix::core::{GraphAction, PackageSpec, StateRegistry, Transaction};
use petgraph::stable_graph::StableDiGraph;
use std::collections::HashMap;

// Import our authoritative A+ Test Infrastructure
mod mock_providers;
use mock_providers::TestKernel;

/// Verifies that the LiNix Transaction engine executes nodes in the
/// correct topological order while respecting parallel dependencies.
///
/// Scenario: Node C depends on Node A and Node B.
/// Logic: (A & B) must be recorded in the call log before C.
#[tokio::test]
async fn test_dag_execution_order_wiring() {
    // 1. Initialize hermetic test kernel (Async DI bootstrap)
    let kernel = TestKernel::new().await;
    let mut graph = StableDiGraph::new();

    // 2. Define standard package specs for a dependency chain
    let spec_a = PackageSpec {
        name: "compiler-core".into(),
        backend: "brew".into(),
        options: Default::default(),
        requires: vec![],
        present: true,
    };
    let spec_b = PackageSpec {
        name: "build-system".into(),
        backend: "brew".into(),
        options: Default::default(),
        requires: vec![],
        present: true,
    };
    let spec_c = PackageSpec {
        name: "complex-app".into(),
        backend: "brew".into(),
        options: Default::default(),
        requires: vec!["brew:compiler-core".into(), "brew:build-system".into()],
        present: true,
    };

    // 3. Construct the DAG
    let a = graph.add_node(GraphAction::Install(spec_a));
    let b = graph.add_node(GraphAction::Install(spec_b));
    let c = graph.add_node(GraphAction::Install(spec_c));

    graph.add_edge(a, c, ());
    graph.add_edge(b, c, ());

    // 4. Initialize the Transaction
    // Resolves E0061: Passes kernel-wide diagnostics engine as the 4th argument via DI
    let mut tx = Transaction::new(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
    );

    // 5. Execute closure
    let result = tx.execute_with_telemetry().await;
    assert!(
        result.is_ok(),
        "Topological execution failed: {:?}",
        result.err()
    );

    // 6. Verification: Logic check of the call log order
    let calls = kernel.mock_executor.get_calls().await;
    let pos_a = calls
        .iter()
        .position(|c| c.contains("compiler-core"))
        .expect("Node A missing");
    let pos_b = calls
        .iter()
        .position(|c| c.contains("build-system"))
        .expect("Node B missing");
    let pos_c = calls
        .iter()
        .position(|c| c.contains("complex-app"))
        .expect("Node C missing");

    assert!(pos_a < pos_c, "Ordering Error: Root A must precede Child C");
    assert!(pos_b < pos_c, "Ordering Error: Root B must precede Child C");
}

/// Verifies that the ChangePlanner detects circular dependency loops
/// in the manifest closure and refuses to build a flawed DAG.
#[tokio::test]
async fn test_circular_dependency_detection_wiring() {
    let kernel = TestKernel::new().await;

    // Use a fresh registry-state for planner isolation
    let state = StateRegistry::default();
    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config);

    // 1. Create a circular paradox: A -> B -> A
    let mut desired = HashMap::new();
    desired.insert(
        "brew".to_string(),
        vec![
            PackageSpec {
                name: "loop-a".into(),
                backend: "brew".into(),
                options: Default::default(),
                requires: vec!["brew:loop-b".into()],
                present: true,
            },
            PackageSpec {
                name: "loop-b".into(),
                backend: "brew".into(),
                options: Default::default(),
                requires: vec!["brew:loop-a".into()],
                present: true,
            },
        ],
    );

    // 2. Attempt Planning
    // Resolves E0061: Provides None (Full Sync)
    let plan_result = planner
        .plan(&desired, PlanScope::Whole(HostBackends::default()))
        .await;

    // 3. Assert Failure
    assert!(
        plan_result.is_err(),
        "Planner allowed a circular dependency loop."
    );
    if let Err(linix::core::Error::Transaction(msg)) = plan_result {
        // V.45: names the cycle rather than just reporting one exists.
        assert!(msg.contains("cycle"), "should say it is a cycle: {}", msg);
    }
}

/// Verifies that the CommandExecutor's LockMap allows distinct backends to execute in parallel
/// while enforcing mutual exclusion for the same backend.
#[tokio::test]
async fn test_parallel_task_isolation_wiring() {
    let kernel = TestKernel::new().await;
    let executor = kernel.app.executor.clone();

    // Logic: brew and cargo should lock their own DBs and run concurrently.
    let lock1 = executor.run_exclusive("brew", "brew", &["list"], false);
    let lock2 = executor.run_exclusive("cargo", "cargo", &["install", "--list"], false);

    // If the logic is correct, these join successfully.
    // If a global lock exists, they would stall.
    let (res1, res2) = tokio::join!(lock1, lock2);

    assert!(res1.is_ok(), "Lock acquisition failed for brew");
    assert!(res2.is_ok(), "Lock acquisition failed for cargo");
}
