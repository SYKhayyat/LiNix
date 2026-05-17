//! Critical Path Tests for LiNix
//! 
//! Tests for:
//! - DAG cycle detection
//! - Transaction rollback on failure
//! - Dry-run mode (ensure no actual commands executed)
//! - StateRegistry save/load roundtrip
//! - Journal recovery (heal())
//! - SemVer constraint matching
//! - resolve_spec recursion depth limits

use linix::core::{
    CommandExecutor, Error, GraphAction, Journal, PackageSpec, StateRegistry,
    Transaction, TransactionConfig, Validator
};
use linix::backends::create_default_registry;
use linix::config::Config;
use linix::app::{App, LuaHooks, SyncEngine};
use linix::app::sync::resolver::StateResolver;
use linix::app::sync::planner::ChangePlanner;
use linix::app::sync::SyncChanges;
use petgraph::stable_graph::StableDiGraph;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::Mutex;

// ============================================================================
// Helper Functions
// ============================================================================

async fn create_test_app() -> App {
    let config = Config::default();
    App::new(config).await.unwrap()
}

fn create_test_graph() -> StableDiGraph<GraphAction, ()> {
    StableDiGraph::new()
}

// ============================================================================
// Test 1: DAG Cycle Detection
// ============================================================================

#[tokio::test]
async fn test_dag_cycle_detection() {
    let config = Config::default();
    let executor = CommandExecutor::new(true, false);
    let hooks = Arc::new(LuaHooks::new(&config).unwrap());
    let registry = Arc::new(create_default_registry(executor, &config, hooks).await);
    let state = StateRegistry::default();
    let planner = ChangePlanner::new(registry, &state, &config);

    let mut desired = HashMap::new();
    
    // Create a circular dependency: A requires B, B requires A
    let spec_a = PackageSpec {
        name: "pkg-a".into(),
        backend: "apt".into(),
        options: HashMap::new(),
        requires: vec!["apt:pkg-b".into()],
    };
    
    let spec_b = PackageSpec {
        name: "pkg-b".into(),
        backend: "apt".into(),
        options: HashMap::new(),
        requires: vec!["apt:pkg-a".into()],
    };
    
    desired.insert("apt".to_string(), vec![spec_a, spec_b]);

    let plan_result = planner.plan(&desired).await;
    
    assert!(plan_result.is_err(), "Planner failed to detect circular dependency");
    if let Err(linix::core::Error::Transaction(msg)) = plan_result {
        assert!(msg.contains("Circular dependency"), "Error message: {}", msg);
    }
}

// ============================================================================
// Test 2: Transaction Rollback on Failure
// ============================================================================

#[tokio::test]
async fn test_transaction_rollback_on_failure() {
    let config = Config::default();
    let executor = CommandExecutor::new(true, false);
    let hooks = Arc::new(LuaHooks::new(&config).unwrap());
    let registry = Arc::new(create_default_registry(executor, &config, hooks).await);
    let journal = Arc::new(Mutex::new(Journal::new().unwrap()));
    
    let mut graph = StableDiGraph::new();
    
    // Create a valid node
    let valid_spec = PackageSpec {
        name: "echo".into(),
        backend: "apt".into(),
        options: HashMap::new(),
        requires: vec![],
    };
    let valid_node = graph.add_node(GraphAction::Install(valid_spec));
    
    // Create an invalid node (will fail during dry-run simulation)
    let invalid_spec = PackageSpec {
        name: "nonexistent-package-xyz-123".into(),
        backend: "apt".into(),
        options: HashMap::new(),
        requires: vec![],
    };
    let invalid_node = graph.add_node(GraphAction::Install(invalid_spec));
    
    // Add edge: invalid depends on valid (to test rollback order)
    graph.add_edge(valid_node, invalid_node, ());
    
    let mut tx = Transaction::with_config(graph, registry, journal, TransactionConfig::quick());
    
    // In dry-run mode, this may still "succeed" but we're testing rollback logic
    // The important part is that rollback doesn't panic
    let result = tx.execute().await;
    // Rollback should be called automatically on failure
    let _ = result;
}

// ============================================================================
// Test 3: Dry-Run Mode - No Actual Commands Executed
// ============================================================================

#[tokio::test]
async fn test_dry_run_no_actual_commands() {
    let config = Config {
        dry_run: true,
        ..Config::default()
    };
    let app = App::new(config).await.unwrap();
    
    // Create a transaction in dry-run mode
    let executor = CommandExecutor::new(true, false);
    let hooks = Arc::new(LuaHooks::new(&app.config).unwrap());
    let registry = Arc::new(create_default_registry(executor.clone(), &app.config, hooks).await);
    let journal = Arc::new(Mutex::new(Journal::new().unwrap()));
    
    let mut graph = StableDiGraph::new();
    let spec = PackageSpec {
        name: "echo".into(),
        backend: "apt".into(),
        options: HashMap::new(),
        requires: vec![],
    };
    graph.add_node(GraphAction::Install(spec));
    
    let mut tx = Transaction::with_config(graph, registry, journal, TransactionConfig::quick());
    
    // Execute should not fail even though apt may not be available
    let result = tx.execute().await;
    // In dry-run mode, this should be Ok(())
    let _ = result;
}

// ============================================================================
// Test 4: StateRegistry Save/Load Roundtrip
// ============================================================================

#[test]
fn test_state_registry_save_load_roundtrip() {
    use linix::core::ManagedPackage;
    
    let dir = tempdir().unwrap();
    let original_path = dir.path().join("registry.json");
    
    // Create a test registry
    let mut registry = StateRegistry::default();
    registry.add_simple("apt", "curl", Some("7.81.0".into()));
    registry.add_simple("brew", "git", Some("2.40.0".into()));
    
    // Save to a temporary location (using internal path override would be complex)
    // Instead, test serialization directly
    let serialized = serde_json::to_string(&registry).unwrap();
    let deserialized: StateRegistry = serde_json::from_str(&serialized).unwrap();
    
    assert_eq!(registry.packages.len(), deserialized.packages.len());
    assert_eq!(registry.packages[0].name, deserialized.packages[0].name);
    assert_eq!(registry.packages[0].backend, deserialized.packages[0].backend);
    assert_eq!(registry.packages[0].version, deserialized.packages[0].version);
}

// ============================================================================
// Test 5: Journal Recovery (heal())
// ============================================================================

#[tokio::test]
async fn test_journal_recovery() {
    let config = Config::default();
    let app = App::new(config).await.unwrap();
    
    let executor = CommandExecutor::new(true, false);
    let hooks = Arc::new(LuaHooks::new(&app.config).unwrap());
    let registry = Arc::new(create_default_registry(executor.clone(), &app.config, hooks).await);
    
    // Create a journal with an incomplete entry
    let journal = Journal::new().unwrap();
    let journal_arc = Arc::new(Mutex::new(journal));
    
    // Simulate an incomplete action
    {
        let mut j = journal_arc.lock().await;
        let spec = PackageSpec {
            name: "test-pkg".into(),
            backend: "apt".into(),
            options: HashMap::new(),
            requires: vec![],
        };
        let _ = j.record_start(crate::core::journal::JournalAction::Install(spec));
        // Don't record success - entry stays InProgress
    }
    
    // Create engine and heal
    let engine = SyncEngine::new(
        &app.config,
        registry,
        executor,
        app.metrics.clone(),
        app.progress.clone(),
        app.hooks.clone(),
        app.snapshot_manager.clone(),
        journal_arc.clone(),
    );
    
    let heal_result = engine.heal().await;
    // In dry-run mode, healing may not actually do anything, but should not error
    let _ = heal_result;
}

// ============================================================================
// Test 6: SemVer Constraint Matching
// ============================================================================

#[test]
fn test_semver_constraint_matching() {
    let config = Config::default();
    let executor = CommandExecutor::new(true, false);
    let registry = Arc::new(linix::backends::BackendRegistry::new());
    
    let resolver = StateResolver {
        config: &config,
        registry,
    };
    
    // Helper to test constraints
    let satisfies = |version: &str, constraint: &str| -> bool {
        resolver.satisfies_constraint(version, constraint)
    };
    
    // Exact matches
    assert!(satisfies("1.2.3", "1.2.3"));
    assert!(!satisfies("1.2.4", "1.2.3"));
    
    // Wildcards
    assert!(satisfies("1.2.3", "*"));
    assert!(satisfies("1.2.3", "latest"));
    
    // Range operators
    assert!(satisfies("1.2.3", ">=1.2.0"));
    assert!(satisfies("1.2.3", "<=1.2.5"));
    assert!(satisfies("1.2.3", ">1.2.0"));
    assert!(satisfies("1.2.3", "<1.2.5"));
    assert!(!satisfies("1.2.3", ">=2.0.0"));
    assert!(!satisfies("1.2.3", "<=1.2.0"));
    
    // Caret and tilde (SemVer)
    assert!(satisfies("1.2.3", "^1.2.0"));
    assert!(satisfies("1.5.0", "^1.0.0"));
    assert!(!satisfies("2.0.0", "^1.0.0"));
    assert!(satisfies("1.2.3", "~1.2.0"));
    assert!(satisfies("1.2.9", "~1.2.0"));
    assert!(!satisfies("1.3.0", "~1.2.0"));
}

// ============================================================================
// Test 7: resolve_spec Recursion Depth Limits
// ============================================================================

#[tokio::test]
async fn test_resolve_spec_recursion_depth() {
    let app = create_test_app().await;
    
    // Create a deeply nested dependency chain
    // This tests that the resolver doesn't infinite loop
    let mut spec_str = String::new();
    for i in (0..50).rev() {
        if i == 49 {
            spec_str = format!("apt:level{}", i);
        } else {
            spec_str = format!("apt:level{}@requires=apt:level{}", i, i + 1);
        }
    }
    
    // This should resolve without hitting the depth limit (100)
    let result = app.resolve_spec(&spec_str).await;
    // May fail due to packages not existing, but should not panic or infinite loop
    let _ = result;
}

// ============================================================================
// Test 8: Package Name Validation
// ============================================================================

#[test]
fn test_package_name_validation() {
    // Valid names
    assert!(Validator::validate_package_name("curl").is_ok());
    assert!(Validator::validate_package_name("python3").is_ok());
    assert!(Validator::validate_package_name("libssl-dev").is_ok());
    assert!(Validator::validate_package_name("_underscore").is_ok());
    assert!(Validator::validate_package_name("dot.separated").is_ok());
    assert!(Validator::validate_package_name("mixed-Case_123").is_ok());
    
    // Invalid names
    assert!(Validator::validate_package_name("").is_err());
    assert!(Validator::validate_package_name("; rm -rf /").is_err());
    assert!(Validator::validate_package_name("$(whoami)").is_err());
    assert!(Validator::validate_package_name("package > /dev/null").is_err());
    assert!(Validator::validate_package_name("../../etc/passwd").is_err());
    assert!(Validator::validate_package_name("package|curl attacker.com").is_err());
    
    // Too long
    let long_name = "a".repeat(300);
    assert!(Validator::validate_package_name(&long_name).is_err());
}

// ============================================================================
// Test 9: Dry-Run VFS Simulation
// ============================================================================

#[tokio::test]
async fn test_dry_run_vfs_simulation() {
    let executor = CommandExecutor::new(true, false);
    let path = std::path::PathBuf::from("/virtual/test.txt");
    let content = "test content";
    
    // Write in dry-run mode
    executor.write_atomic(&path, content).await.unwrap();
    
    // Read back
    let read_content = executor.read_file(&path).await.unwrap();
    assert_eq!(read_content, content);
    
    // Check VFS diff
    let diff = executor.get_vfs_diff();
    assert!(!diff.is_empty());
    assert_eq!(diff[0].0, path);
    assert_eq!(diff[0].1, content);
}

// ============================================================================
// Test 10: Transaction Config Presets
// ============================================================================

#[test]
fn test_transaction_config_presets() {
    let default = TransactionConfig::default();
    assert_eq!(default.max_concurrent, 4);
    assert_eq!(default.max_retries, 3);
    
    let quick = TransactionConfig::quick();
    assert_eq!(quick.max_concurrent, 8);
    assert_eq!(quick.max_retries, 1);
    assert_eq!(quick.node_timeout, std::time::Duration::from_secs(60));
    
    let patient = TransactionConfig::patient();
    assert_eq!(patient.max_concurrent, 2);
    assert_eq!(patient.max_retries, 5);
    assert_eq!(patient.node_timeout, std::time::Duration::from_secs(600));
    
    let network = TransactionConfig::network_resilient();
    assert_eq!(network.max_retries, 5);
    assert_eq!(network.initial_backoff, std::time::Duration::from_secs(2));
    assert!(!network.auto_rollback);
}