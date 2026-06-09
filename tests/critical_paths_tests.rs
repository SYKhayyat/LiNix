use linix::core::{
    CommandExecutor, Error, GraphAction, Journal, PackageSpec, StateRegistry,
    Transaction, TransactionConfig, SnapshotManager
};
use linix::core::executor::{MockExecutor, DryRunOutput};
use linix::core::journal::JournalAction;
use linix::backends::create_default_registry;
use linix::config::Config;
use linix::app::{LuaHooks, MetricsCollector, SyncEngine};
use linix::app::sync::planner::ChangePlanner;
use linix::app::sync::resolver::StateResolver;
use linix::utils::progress;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;

// ============================================================================
// Test Harness
// ============================================================================

struct LogicTestEnv {
    pub registry: Arc<linix::backends::BackendRegistry>,
    pub state: Arc<Mutex<StateRegistry>>,
    pub config: Config,
    pub mock_layer: Arc<MockExecutor>,
    pub journal: Arc<Mutex<Journal>>,
    pub metrics: MetricsCollector,
    pub hooks: Arc<LuaHooks>,
    pub snapshot_manager: Arc<SnapshotManager>,
    pub _tmp: TempDir,
}

/// Creates a platform-agnostic environment for logic testing.
async fn create_logic_test_env() -> LogicTestEnv {
    let tmp = tempfile::Builder::new()
        .prefix("linix_logic_")
        .tempdir()
        .expect("Failed to create temp dir");

    let registry_path = tmp.path().join("registry.json");
    StateRegistry::set_test_path(registry_path.clone());

    let mut config = Config::default();
    config.groups_dir = tmp.path().join("groups");

    let mock_layer = Arc::new(MockExecutor::new());
    mock_layer.set_command_exists("brew", true);
    
    let executor = CommandExecutor::with_layer(true, false, mock_layer.clone());
    let hooks = Arc::new(LuaHooks::new(&config).expect("Failed to init hooks"));
    
    let registry = Arc::new(create_default_registry(executor.clone(), &config, hooks.clone()).await);
    let state = Arc::new(Mutex::new(StateRegistry::default()));
    let journal = Arc::new(Mutex::new(Journal::new().expect("Failed to init journal")));
    let metrics = MetricsCollector::new();
    let snapshot_manager = Arc::new(SnapshotManager::new(executor, &config).await);

    LogicTestEnv {
        registry,
        state,
        config,
        mock_layer,
        journal,
        metrics,
        hooks,
        snapshot_manager,
        _tmp: tmp,
    }
}

// ============================================================================
// Logic Tests
// ============================================================================

#[tokio::test]
async fn test_planner_recursive_native_dependencies() {
    let env = create_logic_test_env().await;
    let state_lock = env.state.lock().await;
    let planner = ChangePlanner::new(env.registry.clone(), &*state_lock, &env.config);

    let mock_output = "pkg-b\n";
    env.mock_layer.set_response(
        "brew deps pkg-a", 
        Ok(DryRunOutput { stdout: mock_output.as_bytes().to_vec(), stderr: vec![] }.into())
    );
    env.mock_layer.set_response(
        "brew deps pkg-b", 
        Ok(DryRunOutput::default().into())
    );

    let mut desired = HashMap::new();
    desired.insert("brew".to_string(), vec![PackageSpec {
        name: "pkg-a".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec![],
    }]);

    let plan = planner.plan(&desired).await.expect("Planning failed");
    assert_eq!(plan.graph.node_count(), 2, "Planner failed to resolve native transitive dependencies");
}

#[tokio::test]
async fn test_dag_cycle_detection_logic() {
    let env = create_logic_test_env().await;
    let state_lock = env.state.lock().await;
    let planner = ChangePlanner::new(env.registry.clone(), &*state_lock, &env.config);

    let mut desired = HashMap::new();
    
    let spec_a = PackageSpec {
        name: "pkg-a".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec!["brew:pkg-b".into()],
    };
    let spec_b = PackageSpec {
        name: "pkg-b".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec!["brew:pkg-a".into()],
    };
    
    desired.insert("brew".to_string(), vec![spec_a, spec_b]);

    let result = planner.plan(&desired).await;
    
    assert!(result.is_err(), "Planner failed to detect circular dependency");
    if let Err(Error::Transaction(msg)) = result {
        assert!(msg.contains("Circular dependency"));
    }
}

#[tokio::test]
async fn test_transaction_rollback_with_retries() {
    let env = create_logic_test_env().await;
    
    let failing_spec = PackageSpec {
        name: "fail-me".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec![],
    };
    
    env.mock_layer.set_response(
        "brew install fail-me", 
        Err(Error::CommandFailed("Simulated Network Timeout".into()))
    );

    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    graph.add_node(GraphAction::Install(failing_spec));

    let mut tx = Transaction::with_config(
        graph, 
        env.registry.clone(), 
        env.journal.clone(), 
        TransactionConfig::quick() 
    );

    let result = tx.execute_with_telemetry().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_journal_self_healing_logic() {
    let env = create_logic_test_env().await;
    
    {
        let mut j = env.journal.lock().await;
        let spec = PackageSpec {
            name: "stale-pkg".into(),
            backend: "brew".into(),
            options: HashMap::new(),
            requires: vec![],
        };
        let _ = j.record_start(JournalAction::Install(spec));
    }

    // Phase 4.1 Fix: Passed 9th argument (state) to SyncEngine::new
    let engine = SyncEngine::new(
        &env.config,
        env.registry.clone(),
        CommandExecutor::with_layer(true, false, env.mock_layer.clone()),
        env.metrics.clone(),
        progress::create_progress_reporter(false),
        env.hooks.clone(),
        env.snapshot_manager.clone(),
        env.journal.clone(),
        env.state.clone(),
    ).await;

    env.mock_layer.set_response("brew uninstall stale-pkg", Ok(DryRunOutput::default().into()));
    env.mock_layer.set_response("brew install stale-pkg", Ok(DryRunOutput::default().into()));

    let res = engine.heal().await;
    assert!(res.is_ok(), "Healing failed: {:?}", res.err());
}

#[tokio::test]
async fn test_semver_constraint_resolution() {
    let env = create_logic_test_env().await;
    let resolver = StateResolver::new(&env.config, env.registry.clone());

    let spec_line = "brew:curl@version=>=7.0.0";
    let spec = resolver.parse_and_probe_spec(spec_line).await.expect("Parsing failed");
    
    assert_eq!(spec.options.get("version").unwrap(), ">=7.0.0");
}

#[tokio::test]
async fn test_dry_run_vfs_simulation() {
    let executor = CommandExecutor::new(true, false);
    let path = std::path::PathBuf::from("/virtual/test.txt");
    let content = "test content";
    
    executor.write_atomic(&path, content).await.unwrap();
    
    let read_content = executor.read_file(&path).await.unwrap();
    assert_eq!(read_content, content);
    
    let diff = executor.get_vfs_diff();
    assert!(!diff.is_empty());
}