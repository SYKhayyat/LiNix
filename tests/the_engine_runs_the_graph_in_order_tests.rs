//! **The order the graph is executed in, the loop it refuses, and the lock that lets two
//! managers run at once.**
//!
//! Three properties of `Transaction`, each of which the engine gets wrong in a way that looks
//! like success: a child installed before its parent still reports `Ok`; a cycle planned rather
//! than refused hangs or repeats; and a global lock where a per-backend one belongs makes every
//! sync run one manager at a time while every assertion still passes.

use shall::app::sync::planner::{ChangePlanner, HostBackends, PlanScope};
use shall::core::{GraphAction, PackageSpec, StateRegistry, Transaction};
use petgraph::stable_graph::StableDiGraph;
use std::collections::HashMap;

use crate::mock_providers::TestKernel;

/// The engine runs a node only after everything it requires.
///
/// C requires A and B, so A and B must be in the call log before C. Nothing else in the suite
/// asserts topological order: a graph executed in insertion order satisfies every other test,
/// because the mock succeeds whatever it is handed.
#[tokio::test]
async fn a_node_runs_only_after_everything_it_requires() {
    let kernel = TestKernel::new().await;
    let mut graph = StableDiGraph::new();

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

    let a = graph.add_node(GraphAction::Install(spec_a));
    let b = graph.add_node(GraphAction::Install(spec_b));
    let c = graph.add_node(GraphAction::Install(spec_c));

    graph.add_edge(a, c, ());
    graph.add_edge(b, c, ());

    let mut tx = Transaction::new(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
    );

    let result = tx.execute_with_telemetry().await;
    assert!(
        result.is_ok(),
        "Topological execution failed: {:?}",
        result.err()
    );

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

/// A cycle in the manifest closure is refused at plan time, and the refusal says it is a cycle.
///
/// V.45: naming the shape is the difference between a user finding the two lines and a user
/// re-reading their whole config.
#[tokio::test]
async fn a_cycle_is_refused_by_name_rather_than_planned() {
    let kernel = TestKernel::new().await;

    let state = StateRegistry::default();
    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config);

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

    let plan_result = planner
        .plan(&desired, PlanScope::Whole(HostBackends::default()))
        .await;

    assert!(
        plan_result.is_err(),
        "Planner allowed a circular dependency loop."
    );
    if let Err(shall::core::Error::Transaction(msg)) = plan_result {
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
async fn two_managers_run_at_once_and_one_manager_runs_one_at_a_time() {
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
