use linix::core::{GraphAction, PackageSpec, Transaction, Journal, StateRegistry};
use linix::core::executor::MockExecutor;
use linix::backends::create_default_registry;
use linix::config::Config;
use linix::app::LuaHooks;
use linix::core::CommandExecutor;
use petgraph::stable_graph::StableDiGraph;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use tempfile::tempdir;

/// Helper to create a hermetic environment for DAG testing.
async fn create_dag_test_env() -> (Arc<linix::backends::BackendRegistry>, Arc<Mutex<Journal>>, Config) {
    let tmp = tempdir().unwrap();
    StateRegistry::set_test_path(tmp.path().join("registry.json"));

    let config = Config::default();
    let mock_layer = Arc::new(MockExecutor::new());
    
    // Phase 6.1: Ensure 'brew' exists for cross-platform test reliability
    mock_layer.set_command_exists("brew", true);
    
    let executor = CommandExecutor::with_layer(true, false, mock_layer);
    let hooks = Arc::new(LuaHooks::new(&config).unwrap());
    let registry = Arc::new(create_default_registry(executor, &config, hooks).await);
    let journal = Arc::new(Mutex::new(Journal::new().unwrap()));
    
    (registry, journal, config)
}

#[tokio::test]
async fn test_dag_execution_order_wiring() {
    let (registry, journal, _) = create_dag_test_env().await;
    let mut graph = StableDiGraph::new();

    // Scenario: Node C depends on A and B
    // Using 'brew' for universal OS compatibility in tests
    let spec_a = PackageSpec {
        name: "gcc".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec![],
    };
    let spec_b = PackageSpec {
        name: "make".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec![],
    };
    let spec_c = PackageSpec {
        name: "neovim".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec!["brew:gcc".into(), "brew:make".into()],
    };

    let a = graph.add_node(GraphAction::Install(spec_a));
    let b = graph.add_node(GraphAction::Install(spec_b));
    let c = graph.add_node(GraphAction::Install(spec_c));

    graph.add_edge(a, c, ());
    graph.add_edge(b, c, ());

    let mut tx = Transaction::new(graph, registry, journal);
    
    // Phase 2.2 Alignment: Use telemetry entry point
    let result = tx.execute_with_telemetry().await;
    assert!(result.is_ok(), "Parallel execution of DAG failed: {:?}", result.err());
}

#[tokio::test]
async fn test_circular_dependency_detection_wiring() {
    let (registry, _, config) = create_dag_test_env().await;
    let state = StateRegistry::default();
    
    // FIX: Fulfills Phase 4.1. Pass the required 3rd argument (&config)
    let planner = linix::app::sync::planner::ChangePlanner::new(registry, &state, &config);

    // Create a circular dependency: A requires B, B requires A
    let mut desired = HashMap::new();
    desired.insert("brew".to_string(), vec![
        PackageSpec {
            name: "pkg-a".into(),
            backend: "brew".into(),
            options: HashMap::new(),
            requires: vec!["brew:pkg-b".into()],
        },
        PackageSpec {
            name: "pkg-b".into(),
            backend: "brew".into(),
            options: HashMap::new(),
            requires: vec!["brew:pkg-a".into()],
        }
    ]);

    let plan_result = planner.plan(&desired).await;
    
    assert!(plan_result.is_err(), "Planner failed to detect circular dependency");
    if let Err(linix::core::Error::Transaction(msg)) = plan_result {
        assert!(msg.contains("Circular dependency"));
    }
}

#[tokio::test]
async fn test_parallel_task_isolation_wiring() {
    let mock_layer = Arc::new(MockExecutor::new());
    let executor = CommandExecutor::with_layer(true, false, mock_layer);
    
    // Verifies that the LockMap allows different backends to run in parallel
    let lock1 = executor.run_exclusive("brew", "ls", &[], false);
    let lock2 = executor.run_exclusive("cargo", "ls", &[], false);
    
    let (res1, res2) = tokio::join!(lock1, lock2);
    assert!(res1.is_ok());
    assert!(res2.is_ok());
}