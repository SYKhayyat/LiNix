// tests/time_travel_tests.rs

use linix::app::sync::planner::ChangePlanner;
use linix::core::GraphAction;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

mod mock_providers;
use mock_providers::TestKernel;

fn get_now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// ============================================================================
// FEATURE 7: TEMPORAL LEASES (@lease logic)
// ============================================================================

#[tokio::test]
async fn test_lease_expiration_pruning_logic() {
    let kernel = TestKernel::new().await;
    let mut state = kernel.state.lock().await;
    let now = get_now_ts();

    // Add expired package
    state.add(
        "brew",
        "expired-binary",
        Some("1.0.0".into()),
        HashMap::from([("lease".into(), "1h".into())]),
        None,
        false,
    );
    if let Some(pkg) = state
        .packages
        .iter_mut()
        .find(|p| p.name == "expired-binary")
    {
        pkg.expires_at = Some(now - 60);
    }

    // Add valid package that should be kept – include it in desired state
    state.add(
        "brew",
        "active-binary",
        Some("2.0.0".into()),
        HashMap::new(),
        None,
        false,
    );
    if let Some(pkg) = state
        .packages
        .iter_mut()
        .find(|p| p.name == "active-binary")
    {
        pkg.expires_at = Some(now + 7200);
    }

    // Desired state includes the valid package, so it won't be removed by drift
    let mut desired = HashMap::new();
    desired.insert(
        "brew".to_string(),
        vec![linix::core::PackageSpec {
            name: "active-binary".into(),
            backend: "brew".into(),
            options: HashMap::new(),
            requires: vec![],
            present: true,
        }],
    );

    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config);
    let plan = planner
        .plan(&desired, None)
        .await
        .expect("Temporal logic failure: Planning pass crashed.");

    // Only the expired package (not in desired) should be removed
    let removals = plan.total_remove();
    assert_eq!(
        removals, 1,
        "Planner failed to identify exactly one expired package."
    );

    let mut found_expired = false;
    for node in plan.graph.node_weights() {
        if let GraphAction::Remove { name, backend } = node {
            if name == "expired-binary" && backend == "brew" {
                found_expired = true;
            }
        }
    }
    assert!(
        found_expired,
        "The expired package 'expired-binary' was not found in the removal set."
    );
}

#[tokio::test]
async fn test_lease_api_timestamp_resolution() {
    let kernel = TestKernel::new().await;
    let mut state = kernel.state.lock().await;

    state.add("brew", "persistent-tool", None, HashMap::new(), None, false);
    assert!(state
        .get_package("brew", "persistent-tool")
        .unwrap()
        .expires_at
        .is_none());

    state
        .update_lease("brew", "persistent-tool", "30d")
        .expect("Lease API failure");

    let pkg = state
        .get_package("brew", "persistent-tool")
        .expect("Registry logic error");
    let expiry = pkg.expires_at.expect("Lease timestamp was not calculated.");

    let expected_offset = 30 * 24 * 3600;
    let actual_offset = expiry - get_now_ts();

    assert!(
        (actual_offset as i64 - expected_offset as i64).abs() < 2,
        "Timestamp resolution error. Expected {}, got {}",
        expected_offset,
        actual_offset
    );
}

#[tokio::test]
async fn test_lease_manifest_override_logic() {
    let kernel = TestKernel::new().await;
    let mut state = kernel.state.lock().await;
    let now = get_now_ts();

    // Expired package in local state
    state.add(
        "brew",
        "manifest-protected",
        None,
        HashMap::new(),
        None,
        false,
    );
    if let Some(pkg) = state
        .packages
        .iter_mut()
        .find(|p| p.name == "manifest-protected")
    {
        pkg.expires_at = Some(now - 1000);
    }

    // Desired state manifest explicitly includes the package (should override expiry)
    let mut desired = HashMap::new();
    desired.insert(
        "brew".to_string(),
        vec![linix::core::PackageSpec {
            name: "manifest-protected".into(),
            backend: "brew".into(),
            options: HashMap::new(),
            requires: vec![],
            present: true,
        }],
    );

    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config);
    let plan = planner.plan(&desired, None).await.unwrap();

    assert_eq!(
        plan.total_remove(),
        0,
        "Package in manifest was incorrectly scheduled for removal due to lease."
    );
}
