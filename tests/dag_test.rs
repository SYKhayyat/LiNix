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

/// The lock is per backend: two managers run at once, one manager runs one at a time.
///
/// **This test used to prove neither half.** It fired one `brew` call and one `cargo` call
/// through `tokio::join!` and asserted both returned `Ok` — which a single global mutex also
/// satisfies, because a serialised pair still both succeed. Mock commands are instantaneous,
/// so there was nothing to contend over either. Nothing else in the suite covers granularity,
/// so the property was untested in both directions.
///
/// Now each call takes measurable time, and the two halves are asserted against each other:
/// different keys must finish in about one command's time, the same key in about two.
#[tokio::test]
async fn test_parallel_task_isolation_wiring() {
    use std::time::{Duration, Instant};

    let kernel = TestKernel::new().await;
    let executor = kernel.app.executor.clone();

    const STEP: Duration = Duration::from_millis(300);
    kernel.mock_executor.set_delay("brew list", STEP);
    kernel.mock_executor.set_delay("cargo install --list", STEP);

    // Two backends, two locks: they overlap, so the pair costs about one step rather than two.
    let started = Instant::now();
    let (a, b) = tokio::join!(
        executor.run_exclusive("brew", "brew", &["list"], false),
        executor.run_exclusive("cargo", "cargo", &["install", "--list"], false),
    );
    let across_backends = started.elapsed();
    a.expect("brew");
    b.expect("cargo");

    // One backend, one lock: the second call waits, so the pair costs about two steps. This is
    // the half a global mutex would also pass — it is here so the half above cannot be read as
    // "locking is simply absent".
    let started = Instant::now();
    let (a, b) = tokio::join!(
        executor.run_exclusive("brew", "brew", &["list"], false),
        executor.run_exclusive("brew", "brew", &["list"], false),
    );
    let same_backend = started.elapsed();
    a.expect("brew, first");
    b.expect("brew, second");

    assert!(
        across_backends < STEP * 2,
        "two different backends took {across_backends:?} for two {STEP:?} commands — they \
         serialised, so the lock is not per backend and every sync runs one manager at a time"
    );
    assert!(
        same_backend >= STEP * 2,
        "two calls to the SAME backend took {same_backend:?} — they overlapped, so nothing is \
         holding a manager's database against a concurrent write"
    );
    assert!(
        same_backend > across_backends,
        "the same backend ({same_backend:?}) was not slower than two different ones \
         ({across_backends:?}), so the lock key is being ignored"
    );
}
