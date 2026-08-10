//! **`linix shell` — packages live for the session and go when it ends.**
//!
//! The ephemeral shell is the one path that installs without declaring, so the only thing
//! keeping it from being drift is that the teardown is exact: it removes what this session
//! registered and nothing another session did. Four properties — the transient registration,
//! the purge that is scoped to one session, the manifest discovered from the working directory,
//! and the mount point a session resolves to.

use linix::core::executor::DryRunOutput;
use linix::core::PackageSpec;
use tokio::fs;

use crate::mock_providers::TestKernel;

#[tokio::test]
async fn a_session_registers_its_packages_as_transient() {
    let kernel = TestKernel::new().await;
    let shell = kernel.app.shell();

    kernel
        .mock_executor
        .set_response("brew install -- vim", Ok(DryRunOutput::default().into()));

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
async fn a_purge_takes_this_sessions_packages_and_leaves_another_sessions() {
    let kernel = TestKernel::new().await;
    let shell = kernel.app.shell();

    let target_session_id = "purge-this-session";
    let other_session_id = "keep-this-session";

    {
        let mut state = kernel.state.lock().await;

        state.add("brew", "git", None, Default::default(), "test", false);

        state.active_session_id = Some(target_session_id.to_string());
        state.add(
            "brew",
            "temp-tool-1",
            None,
            Default::default(),
            "test",
            true,
        );

        state.active_session_id = Some(other_session_id.to_string());
        state.add(
            "brew",
            "temp-tool-2",
            None,
            Default::default(),
            "test",
            true,
        );
    }

    kernel.mock_executor.set_response(
        // With the terminator, because that is the argv. Without it the registration matched
        // nothing and the purge ran against the mock's default (`LX-8`).
        "brew uninstall -- temp-tool-1",
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

/// What a project-local `linix.txt` declares.
///
/// This test used to write a file, read it back, and then split it with its own copy of
/// `auto_shell`'s five lines — so it asserted on `str::lines` and would have passed against a
/// LiNix that had no manifest discovery at all. It now drives the reader the shell uses.
#[tokio::test]
async fn a_session_finds_the_manifest_in_the_directory_it_started_in() {
    let kernel = TestKernel::new().await;

    let manifest_path = kernel.tmp.path().join("linix.txt");
    fs::write(
        &manifest_path,
        "# Dev Stack\n\
         brew:jq\n\
         \n\
         brew:htop  # the one I actually use\n\
         web:https://example.com/tool.tar.gz#sha256-fragment\n",
    )
    .await
    .unwrap();

    let content = fs::read_to_string(&manifest_path).await.unwrap();
    let pkgs = linix::app::shell::manifest_lines(&content);

    assert_eq!(
        pkgs,
        vec![
            "brew:jq".to_string(),
            "brew:htop".to_string(),
            "web:https://example.com/tool.tar.gz#sha256-fragment".to_string(),
        ],
        "the reader must drop the heading, drop the blank, cut the trailing comment after \
         whitespace, and leave a `#` that follows a non-space alone — a URL fragment is data"
    );
}

#[tokio::test]
async fn a_session_resolves_to_its_own_mount_point() {
    let kernel = TestKernel::new().await;
    let shell = kernel.app.shell();

    let simulated_root = kernel.tmp.path().join("store").join("curl-8.5.0");
    fs::create_dir_all(&simulated_root).await.unwrap();

    // Clear previous calls for clean debugging
    {
        let mut log = kernel.mock_executor.call_log.lock().await;
        log.clear();
    }

    let expected_cmd = "brew info --json=v1 -- curl";

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
        options: Default::default(),
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
