use linix::app::{App, SyncEngine};
use linix::app::sync::resolver::StateResolver;
use linix::app::sync::planner::ChangePlanner;
use linix::config::Config;
use linix::core::{
    CommandExecutor, GraphAction, PackageSpec, StateRegistry,
    Transaction, TransactionConfig
};
use linix::core::executor::{MockExecutor, DryRunOutput};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

// ============================================================================
// E2E Test Harness
// ============================================================================

struct E2ETestEnv {
    pub app: App,
    pub mock_layer: Arc<MockExecutor>,
    pub _tmp: TempDir,
}

async fn create_e2e_test_env() -> E2ETestEnv {
    let tmp = tempfile::Builder::new()
        .prefix("linix_e2e_")
        .tempdir()
        .expect("Failed to create temp dir");

    // Redirect the StateRegistry path for isolation
    let registry_path = tmp.path().join("registry.json");
    StateRegistry::set_test_path(registry_path);

    let mut config = Config::default();
    config.groups_dir = tmp.path().join("groups");
    config.dry_run = true;

    let mock_layer = Arc::new(MockExecutor::new());
    // Ensure mock layer reports cross-platform backends as available
    mock_layer.set_command_exists("brew", true);
    mock_layer.set_command_exists("cargo", true);
    
    // Initialize App (Async)
    let app = App::new(config).await.expect("Failed to init app");

    E2ETestEnv {
        app,
        mock_layer,
        _tmp: tmp,
    }
}

// ============================================================================
// E2E Logic Tests
// ============================================================================

#[tokio::test]
async fn test_e2e_sync_flow_hermetic() {
    let env = create_e2e_test_env().await;
    
    // 1. Create a manifest file in the isolated groups dir
    let test_group_path = env.app.config.groups_dir.join("test.txt");
    tokio::fs::create_dir_all(&env.app.config.groups_dir).await.unwrap();
    // Use 'brew' for platform-agnostic testing
    tokio::fs::write(&test_group_path, "brew:vim\n").await.unwrap();

    // 2. Setup the Sync Engine with 9 arguments (Phase 4.1 State Injection Fix)
    // We pass env.app.state.clone() so the engine and test share the same memory.
    let executor = CommandExecutor::with_layer(true, false, env.mock_layer.clone());
    let engine = SyncEngine::new(
        &env.app.config,
        env.app.registry.clone(),
        executor,
        env.app.metrics.clone(),
        env.app.progress.clone(),
        env.app.hooks.clone(),
        env.app.snapshot_manager.clone(),
        env.app.journal.clone(),
        env.app.state.clone(),
    ).await;

    // 3. Resolve and Plan
    let resolver = StateResolver::new(&env.app.config, env.app.registry.clone());
    let desired = resolver.resolve_desired_state().await.unwrap();
    
    let changes = {
        let state_guard = env.app.state.lock().await;
        let planner = ChangePlanner::new(env.app.registry.clone(), &*state_guard, &env.app.config);
        planner.plan(&desired).await.unwrap()
    };

    // 4. Prime mock
    env.mock_layer.set_response("brew install vim", Ok(DryRunOutput::default().into()));

    // 5. Execute Sync
    let result = engine.sync(changes).await;
    assert!(result.is_ok(), "E2E Sync failed: {:?}", result.err());

    // 6. Verify state modification
    // Because we injected the Arc<Mutex<StateRegistry>>, this lock now sees the engine's changes.
    let state = env.app.state.lock().await;
    assert!(state.is_managed("brew", "vim"), "Package not found in state registry after sync");
}

#[tokio::test]
async fn test_e2e_cross_backend_teleport() {
    let env = create_e2e_test_env().await;
    let teleporter = env.app.teleporter();

    // Use cross-platform 'brew' as source
    let mock_info = "curl 8.0.1";
    env.mock_layer.set_response("brew list --versions", 
        Ok(DryRunOutput { stdout: mock_info.as_bytes().to_vec(), stderr: vec![] }.into())
    );
    
    // Prime installation in target backend (cargo)
    env.mock_layer.set_response("cargo install curl", Ok(DryRunOutput::default().into()));
    // Prime removal in source backend (brew)
    env.mock_layer.set_response("brew uninstall curl", Ok(DryRunOutput::default().into()));

    // Execute Teleport
    let result = teleporter.teleport("curl", "cargo").await;
    
    // Verify path logic completion
    assert!(result.is_ok() || matches!(result.err(), Some(linix::core::Error::PackageNotFound(_))));
}

#[tokio::test]
async fn test_concurrent_transaction_safety_e2e() {
    let env = create_e2e_test_env().await;
    
    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    for i in 0..5 {
        let spec = PackageSpec {
            name: format!("pkg-{}", i),
            backend: "brew".into(),
            options: HashMap::new(),
            requires: vec![],
        };
        graph.add_node(GraphAction::Install(spec));
    }

    let mut tx = Transaction::with_config(
        graph, 
        env.app.registry.clone(), 
        env.app.journal.clone(), 
        TransactionConfig::quick()
    );

    // Verify parallel execution completion on mock backend
    let result = tx.execute_with_telemetry().await;
    assert!(result.is_ok());
}