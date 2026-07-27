use linix::app::sync::planner::{ChangePlanner, Scope};
use linix::app::sync::resolver::StateResolver;
use linix::core::{Error, PackageSpec};
use std::collections::HashMap;
use tokio::fs;

// Import our exhaustive A+ Test Infrastructure
mod mock_providers;
use mock_providers::TestKernel;

// ============================================================================
// MODULES: `use` and recursive expansion
// ============================================================================

/// A module `use`ing a module, reached by a profile, expands to a flat closure.
///
/// 1. `use NAME` takes a name — never a path, never a URL.
/// 2. Deep nesting resolves without cycles.
/// 3. Each package records where it came from and what it belongs to.
#[tokio::test]
async fn test_recursive_module_expansion_logic() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();

    // 1. The leaf module.
    fs::write(root.join("modules/network.txt"), "brew:curl\nbrew:wget\n")
        .await
        .unwrap();

    // 2. A module that uses it. A module may use a module; it may never name a profile.
    fs::write(root.join("modules/bundle.txt"), "use network\nbrew:git\n")
        .await
        .unwrap();

    // 3. A profile to reach it, and the machine set to that profile. Only profiles can be
    //    activated, and nothing is active unless a profile names it.
    fs::write(root.join("profiles/Work"), "use bundle\n")
        .await
        .unwrap();
    fs::write(root.join("active"), "Work\n").await.unwrap();

    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let desired = resolver
        .resolve_desired_state()
        .await
        .expect("State resolution failed");

    let brew_specs = desired
        .get("brew")
        .expect("Missing brew backend specs in resolution map");
    let names: Vec<&str> = brew_specs.iter().map(|s| s.name.as_str()).collect();

    assert!(
        names.contains(&"curl"),
        "Resolver failed to expand nested leaf 'curl' from `network`"
    );
    assert!(
        names.contains(&"wget"),
        "Resolver failed to expand nested leaf 'wget' from `network`"
    );
    assert!(
        names.contains(&"git"),
        "Resolver failed to expand direct member 'git' from `bundle`"
    );
    assert_eq!(
        names.len(),
        3,
        "Expanded closure count mismatch. Expected 3 packages."
    );

    // Where the line is, for a human; and what it belongs to, for `--module` / `--profile`.
    let curl_spec = brew_specs.iter().find(|s| s.name == "curl").unwrap();
    assert!(curl_spec
        .options
        .get("__source")
        .unwrap()
        .contains("network.txt:1"));
    let scopes = curl_spec.options.get("__scopes").unwrap();
    assert!(scopes.contains("module:network"), "{}", scopes);
    assert!(scopes.contains("profile:Work"), "{}", scopes);
}

/// W13: the plan explains a variable-driven change by diffing this run's variables against the
/// last successful sync (HEAD). This proves the baseline half — `vars_at_last_sync` reads the
/// committed `vars`, `resolve_vars` reads the working tree, and `vars::diff` names the change.
#[tokio::test]
async fn vars_change_is_measured_against_the_committed_baseline() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root().to_path_buf();

    fs::write(root.join("vars"), "role = travel\n")
        .await
        .unwrap();
    let git = kernel.app.git_manager();
    git.init().unwrap();
    git.commit_all("baseline").unwrap();

    // Edit the working tree without committing — this is the "you edited vars" state.
    fs::write(root.join("vars"), "role = desktop\n")
        .await
        .unwrap();

    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let baseline = resolver
        .vars_at_last_sync(&git)
        .await
        .unwrap()
        .expect("HEAD has a vars file, so there is a baseline");
    let now = resolver.resolve_vars().await.unwrap();

    assert_eq!(
        baseline["role"],
        linix::model::vars::Value::Str("travel".into())
    );
    assert_eq!(
        now["role"],
        linix::model::vars::Value::Str("desktop".into())
    );

    let changed = linix::model::vars::diff(&baseline, &now);
    assert_eq!(changed.len(), 1, "only role changed: {:?}", changed);
    assert_eq!(changed[0].0, "role");
}

/// A `use` of a module that does not exist is a descriptive error, never a silent skip and
/// never a package named `ghost-module-123`.
#[tokio::test]
async fn test_module_resolution_failure_handling() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();

    fs::write(root.join("profiles/Work"), "use ghost-module-123\n")
        .await
        .unwrap();
    fs::write(root.join("active"), "Work\n").await.unwrap();

    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let result = resolver.resolve_desired_state().await;

    assert!(
        result.is_err(),
        "Resolution should have failed for a missing module reference"
    );
    if let Err(Error::Config(msg)) = result {
        assert!(
            msg.contains("ghost-module-123"),
            "Error message should identify the specific missing module: {}",
            msg
        );
    } else {
        panic!("Incorrect error type returned: {:?}", result.err());
    }
}

// ============================================================================
// FEATURE 1: STRUCTURED JSON DRY-RUN REPORTING
// ============================================================================

/// Verifies that the ChangePlanner generates a SyncReport with accurate fields
/// suitable for high-fidelity JSON serialization.
#[tokio::test]
async fn test_sync_report_generation_schema_fidelity() {
    let kernel = TestKernel::new().await;
    let state_guard = kernel.state.lock().await;

    let planner = ChangePlanner::new(
        kernel.app.registry.clone(),
        &state_guard,
        &kernel.app.config,
    );

    // Seed a desired state containing source metadata
    let mut desired = HashMap::new();
    desired.insert(
        "brew".to_string(),
        vec![PackageSpec {
            name: "ripgrep".into(),
            backend: "brew".into(),
            options: HashMap::from([("__source".into(), "module:dev-tools".into())]),
            requires: vec![],
            present: true,
        }],
    );

    // Plan with None (Global Sync)
    let plan = planner.plan(&desired, None).await.unwrap();
    let report = plan.generate_report();

    // 1. Verify business logic mapping to the Report structure
    assert_eq!(report.change_count, 1);
    assert_eq!(report.install.len(), 1);
    assert_eq!(report.install[0].name, "ripgrep");
    assert_eq!(
        report.install[0].source,
        Some("module:dev-tools".to_string())
    );

    // 2. Verify JSON Serialization (Schema Integrity)
    let json_output = serde_json::to_string(&report).expect("SyncReport failed JSON serialization");
    assert!(
        json_output.contains("\"change_count\":1"),
        "JSON missing change_count field"
    );
    assert!(
        json_output.contains("\"name\":\"ripgrep\""),
        "JSON missing package name"
    );
    assert!(
        json_output.contains("\"source\":\"module:dev-tools\""),
        "JSON missing source metadata"
    );
}

// ============================================================================
// FEATURE 4: SCOPED UPGRADE LOGIC
// ============================================================================

/// Verifies that a Scope correctly prunes the DAG to only include
/// nodes matching the requested source origin.
#[tokio::test]
async fn test_scoped_planner_filtering_accuracy() {
    let kernel = TestKernel::new().await;
    let state_guard = kernel.state.lock().await;
    let planner = ChangePlanner::new(
        kernel.app.registry.clone(),
        &state_guard,
        &kernel.app.config,
    );

    // Setup a mixed desired state. A package belongs to the module that holds it and the
    // profile that reaches it, and the resolver records both.
    let mut desired = HashMap::new();
    desired.insert(
        "brew".to_string(),
        vec![
            PackageSpec {
                name: "pkg-work".into(),
                backend: "brew".into(),
                options: HashMap::from([("__scopes".into(), "module:dev;profile:Work".into())]),
                requires: vec![],
                present: true,
            },
            PackageSpec {
                name: "pkg-home".into(),
                backend: "brew".into(),
                options: HashMap::from([("__scopes".into(), "module:media;profile:Home".into())]),
                requires: vec![],
                present: true,
            },
        ],
    );

    let plan = planner
        .plan(&desired, Some(Scope::Profile("Work".into())))
        .await
        .unwrap();

    // Verification
    assert_eq!(
        plan.total_install(),
        1,
        "Planner failed to prune the graph based on scope"
    );
    let report = plan.generate_report();
    assert_eq!(report.install[0].name, "pkg-work");
    assert!(
        !report.install.iter().any(|r| r.name == "pkg-home"),
        "Package from outside the scope (profile Home) was incorrectly included in the plan"
    );
}

// ============================================================================
// BUG FIX 3: --LOCKED MODE INTEGRITY
// ============================================================================

/// Verifies that Locked Mode prevents resolution if manifest versions deviate
/// from the cryptographically tracked locks.json.
#[tokio::test]
async fn test_locked_mode_version_conflict_enforcement() {
    let kernel = TestKernel::new().await;

    // 1. Setup a lock file with version 1.2.3
    let lock_content = r#"{ "locks": { "brew:vim": "1.2.3" } }"#;
    fs::create_dir_all(kernel.app.config.config_root().join("locks"))
        .await
        .unwrap();
    fs::write(
        kernel
            .app
            .config
            .config_root()
            .join("locks")
            .join("versions.json"),
        lock_content,
    )
    .await
    .unwrap();

    // 2. A module requesting a conflicting version 2.0.0, and a profile reaching it.
    let root = kernel.app.config.config_root();
    fs::write(root.join("modules/main.txt"), "brew:vim@version=2.0.0\n")
        .await
        .unwrap();
    fs::write(root.join("profiles/Work"), "use main\n")
        .await
        .unwrap();
    fs::write(root.join("active"), "Work\n").await.unwrap();

    // 3. Resolve in Locked Mode (locked = true)
    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), true).await;
    let result = resolver.resolve_desired_state().await;

    // 4. Assert Failure
    assert!(
        result.is_err(),
        "Resolver should have rejected the version mismatch in locked mode"
    );
    if let Err(Error::Validation(msg)) = result {
        assert!(
            msg.contains("version mismatch"),
            "Incorrect validation error message: {}",
            msg
        );
        assert!(
            msg.contains("brew:vim"),
            "Error should identify the offending package"
        );
    }
}

// ============================================================================
// EXTRAS: the teardown ledger
// ============================================================================

/// An undo that fails must not be forgotten. `reconcile` records what is declared now,
/// so a key whose teardown failed used to vanish from `locks/extras.toml` after one warning —
/// leaving a service or a timer in place that LiNix no longer knows it owns. It stays recorded
/// until the undo succeeds.
#[tokio::test]
async fn a_failed_undo_stays_in_the_extras_ledger() {
    let kernel = TestKernel::new().await;
    let locks = kernel.app.config.config_root().join("locks");
    let path = linix::core::ExtrasLedger::path_in(&locks);

    // `no-such-backend` cannot be resolved, so its teardown fails for a reason no host can
    // fix by luck. Nothing declares it, so it is drift the moment the ledger is read.
    let mut ledger = linix::core::ExtrasLedger::new();
    ledger.record(
        ["repo:no-such-backend:ppa/example".to_string()]
            .into_iter()
            .collect(),
    );
    ledger.save(&path).unwrap();

    let state = linix::model::DesiredState::default();
    kernel
        .app
        .extras()
        .reconcile(&state)
        .await
        .expect("a failed undo is reported, not fatal");

    let after = linix::core::ExtrasLedger::load(&path).unwrap();
    assert!(
        after.applied().contains("repo:no-such-backend:ppa/example"),
        "the failed teardown was dropped from the ledger: {:?}",
        after.applied()
    );
}

/// And the other half: a teardown that succeeds does leave the ledger, or every sync would
/// retry an undo forever.
#[tokio::test]
async fn a_successful_undo_leaves_the_extras_ledger() {
    let kernel = TestKernel::new().await;
    let locks = kernel.app.config.config_root().join("locks");
    let path = linix::core::ExtrasLedger::path_in(&locks);

    // An unknown *kind* has no undo to fail: `undo_extra` warns and reports success, which is
    // the success path this asserts without needing a real service manager on the host.
    let mut ledger = linix::core::ExtrasLedger::new();
    ledger.record(["nosuchkind:whatever".to_string()].into_iter().collect());
    ledger.save(&path).unwrap();

    let state = linix::model::DesiredState::default();
    kernel.app.extras().reconcile(&state).await.unwrap();

    let after = linix::core::ExtrasLedger::load(&path).unwrap();
    assert!(
        after.applied().is_empty(),
        "a successful teardown was left recorded: {:?}",
        after.applied()
    );
}
