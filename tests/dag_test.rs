use linix::core::{GraphAction, PackageSpec, Transaction, Journal};
use linix::backends::create_default_registry;
use linix::config::Config;
use linix::app::LuaHooks;
use linix::core::CommandExecutor;
use petgraph::stable_graph::StableDiGraph;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;

#[tokio::test]
async fn test_dag_execution_order() {
    let config = Config::default();
    let executor = CommandExecutor::new(true, false); // Dry run
    let hooks = Arc::new(LuaHooks::new(&config).unwrap());
    let registry = Arc::new(create_default_registry(executor.clone(), &config, hooks).await);
    let journal = Arc::new(Mutex::new(Journal::new().unwrap()));

    let mut graph = StableDiGraph::new();

    // Create a scenario where:
    // Node A (gcc)
    // Node B (make)
    // Node C (neovim) depends on A and B
    
    let spec_a = PackageSpec {
        name: "gcc".into(),
        backend: "apt".into(),
        options: HashMap::new(),
        requires: vec![],
    };
    let spec_b = PackageSpec {
        name: "make".into(),
        backend: "apt".into(),
        options: HashMap::new(),
        requires: vec![],
    };
    let spec_c = PackageSpec {
        name: "neovim".into(),
        backend: "apt".into(),
        options: HashMap::new(),
        requires: vec!["apt:gcc".into(), "apt:make".into()],
    };

    let a = graph.add_node(GraphAction::Install(spec_a));
    let b = graph.add_node(GraphAction::Install(spec_b));
    let c = graph.add_node(GraphAction::Install(spec_c));

    // Add edges: A -> C and B -> C (C must happen after A and B)
    graph.add_edge(a, c, ());
    graph.add_edge(b, c, ());

    let mut tx = Transaction::new(graph, registry, journal);
    
    // Execute the transaction. In dry-run mode, this verifies the DAG draining logic.
    let result = tx.execute().await;
    
    assert!(result.is_ok(), "Parallel execution of DAG failed: {:?}", result.err());
}

#[tokio::test]
async fn test_circular_dependency_detection() {
    let mut config = Config::default();
    let executor = CommandExecutor::new(true, false);
    let state = linix::core::StateRegistry::default();
    let hooks = Arc::new(LuaHooks::new(&config).unwrap());
    let registry = Arc::new(create_default_registry(executor, &config, hooks).await);
    
    let planner = linix::app::sync::planner::ChangePlanner::new(registry, &state);

    // Create a circular dependency: A requires B, B requires A
    let mut desired = HashMap::new();
    desired.insert("apt".to_string(), vec![
        PackageSpec {
            name: "pkg-a".into(),
            backend: "apt".into(),
            options: HashMap::new(),
            requires: vec!["apt:pkg-b".into()],
        },
        PackageSpec {
            name: "pkg-b".into(),
            backend: "apt".into(),
            options: HashMap::new(),
            requires: vec!["apt:pkg-a".into()],
        }
    ]);

    let plan_result = planner.plan(&desired).await;
    
    assert!(plan_result.is_err(), "Planner failed to detect circular dependency");
    if let Err(linix::core::Error::Transaction(msg)) = plan_result {
        assert!(msg.contains("Circular dependency"));
    } else {
        panic!("Expected Transaction error for circular dependency");
    }
}

#[tokio::test]
async fn test_parallel_task_isolation() {
    // Verifies that the LockMap allows different backends to run in parallel
    let executor = CommandExecutor::new(true, false);
    
    // Attempt to acquire two different locks simultaneously
    let lock1 = executor.run_exclusive("apt", "ls", &[], false);
    let lock2 = executor.run_exclusive("cargo", "ls", &[], false);
    
    // They should both resolve without blocking each other because keys are different
    let (res1, res2) = tokio::join!(lock1, lock2);
    assert!(res1.is_ok());
    assert!(res2.is_ok());
}