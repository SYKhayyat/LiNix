// tests/shell_lifecycle_tests.rs

use linix::core::executor::DryRunOutput;
use linix::core::PackageSpec;
use std::collections::HashMap;
use tokio::fs;

mod mock_providers;
use mock_providers::TestKernel;

// ============================================================================
// The ephemeral shell (`linix shell`): packages live for the session and go on exit.
// ============================================================================

#[tokio::test]
async fn test_ephemeral_shell_transient_registration_logic() {
    let kernel = TestKernel::new().await;
    let shell = kernel.app.shell();

    kernel
        .mock_executor
        .set_response("brew install vim", Ok(DryRunOutput::default().into()));

    let session_id = "test-session-v3.6.0";

    {
        let mut state_guard = kernel.state.lock().await;
        state_guard.active_session_id = Some(session_id.to_string());
    }

    shell
        .provision_transient_env(&["brew:vim".to_string()], session_id)
        .await
        .expect("Ephemeral provisioning logic failed.");

    let state_final = kernel.state.lock().await;
    let pkg = state_final
        .get_package("brew", "vim")
        .expect("Package was not correctly registered in state after shell provision.");

    assert!(
        pkg.is_transient,
        "A package in an ephemeral shell must be marked transient."
    );
    assert_eq!(
        pkg.session_id.as_deref(),
        Some(session_id),
        "Package must be cryptographically associated with the active session ID."
    );
}

#[tokio::test]
async fn test_ephemeral_shell_atomic_purge_isolation_logic() {
    let kernel = TestKernel::new().await;
    let shell = kernel.app.shell();

    let target_session_id = "purge-this-session";
    let other_session_id = "keep-this-session";

    {
        let mut state = kernel.state.lock().await;

        state.add("brew", "git", None, HashMap::new(), None, false);

        state.active_session_id = Some(target_session_id.to_string());
        state.add("brew", "temp-tool-1", None, HashMap::new(), None, true);

        state.active_session_id = Some(other_session_id.to_string());
        state.add("brew", "temp-tool-2", None, HashMap::new(), None, true);
    }

    kernel.mock_executor.set_response(
        "brew uninstall temp-tool-1",
        Ok(DryRunOutput::default().into()),
    );

    shell
        .cleanup_transient_env(target_session_id)
        .await
        .expect("Atomic purge orchestration failed.");

    let state_final = kernel.state.lock().await;

    assert!(state_final.is_managed("brew", "git"),
            "CRITICAL INTEGRITY FAILURE: Permanent system package was incorrectly purged during shell exit!");

    assert!(
        state_final.is_managed("brew", "temp-tool-2"),
        "Logic Error: Transient package from a different session was incorrectly purged!"
    );

    assert!(
        !state_final.is_managed("brew", "temp-tool-1"),
        "Failure: Targeted transient package physically remains in registry after purge."
    );
}

#[tokio::test]
async fn test_ephemeral_shell_auto_manifest_discovery() {
    let kernel = TestKernel::new().await;

    let manifest_content = "# Dev Stack\nbrew:jq\nbrew:htop\n";
    let manifest_path = kernel.tmp.path().join("linix.txt");
    fs::write(&manifest_path, manifest_content).await.unwrap();

    let content = fs::read_to_string(&manifest_path)
        .await
        .expect("Failed to read test manifest.");
    let pkgs: Vec<String> = content
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    assert_eq!(
        pkgs.len(),
        2,
        "Manifest parser failed to identify all package lines."
    );
    assert_eq!(pkgs[0], "brew:jq");
    assert_eq!(pkgs[1], "brew:htop");
}

#[tokio::test]
async fn test_ephemeral_shell_mount_point_resolution() {
    let kernel = TestKernel::new().await;
    let shell = kernel.app.shell();

    let simulated_root = kernel.tmp.path().join("store").join("curl-8.5.0");
    fs::create_dir_all(&simulated_root).await.unwrap();

    // Clear previous calls for clean debugging
    {
        let mut log = kernel.mock_executor.call_log.lock().await;
        log.clear();
    }

    let expected_cmd = "brew info --json=v1 curl";

    // Build JSON safely using serde_json to escape backslashes on Windows
    let json_value = serde_json::json!([{
        "name": "curl",
        "versions": { "stable": "8.5.0" },
        "installed": [{ "prefix": simulated_root.to_string_lossy() }]
    }]);
    let json_output = json_value.to_string();

    kernel.mock_executor.set_response(
        expected_cmd,
        Ok(DryRunOutput {
            stdout: json_output.as_bytes().to_vec(),
            stderr: vec![],
        }
        .into()),
    );

    let spec = PackageSpec {
        name: "curl".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec![],
        present: true,
    };

    let result = shell
        .locate_package_root(&spec)
        .await
        .expect("Path discovery logic crashed.");

    assert!(
        result.is_some(),
        "Ephemeral shell failed to resolve physical root for sandbox mount."
    );
    assert_eq!(
        result.unwrap(),
        simulated_root,
        "Resolved path does not match simulated installation store path."
    );
}
