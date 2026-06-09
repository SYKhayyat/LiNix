use linix::backends::create_default_registry;
use linix::config::Config;
use linix::app::LuaHooks;
use linix::core::{
    CommandExecutor, PackageSpec, BackendCapabilities, 
    StateRegistry
};
use linix::core::executor::{MockExecutor, DryRunOutput};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

// ============================================================================
// LiNix Test Harness
// ============================================================================

struct TestEnv {
    pub registry: Arc<linix::backends::BackendRegistry>,
    pub executor: CommandExecutor,
    pub _config: Arc<Config>,
    pub _tmp: TempDir,
    pub mock_layer: Arc<MockExecutor>,
}

/// Creates a fully isolated hermetic environment for testing backends.
/// Fulfills Phase 1: Total isolation from the host system.
async fn create_test_env() -> TestEnv {
    let tmp = tempfile::Builder::new()
        .prefix("linix_test_")
        .tempdir()
        .expect("Failed to create temp dir");

    // Redirect the StateRegistry to the temp directory
    let registry_path = tmp.path().join("registry.json");
    StateRegistry::set_test_path(registry_path);

    let mut config = Config::default();
    config.groups_dir = tmp.path().join("groups");
    config.tmp_dir = tmp.path().join("tmp");
    config.github_dir = tmp.path().join("github");
    config.web_dir = tmp.path().join("web");
    config.appimage_dir = tmp.path().join("appimages");

    let mock_layer = Arc::new(MockExecutor::new());
    // Important: check_command in MockExecutor defaults to true, 
    // so backends will report as "available" even if binaries aren't on the host.
    let executor = CommandExecutor::with_layer(true, false, mock_layer.clone());
    let hooks = Arc::new(LuaHooks::new(&config).expect("Failed to init hooks"));
    
    let registry = Arc::new(create_default_registry(executor.duplicate(), &config, hooks).await);
    
    TestEnv {
        registry,
        executor,
        _config: Arc::new(config),
        _tmp: tmp,
        mock_layer,
    }
}

/// Helper to execute a standard install/remove test on a capability set.
async fn run_capability_test(
    backend: Arc<BackendCapabilities>,
    package_name: &str,
) {
    assert!(backend.is_available(), "Backend {} should be available in mock mode", backend.name());
    let installer = backend.as_installable().expect("Backend must be installable");

    let spec = PackageSpec {
        name: package_name.to_string(),
        backend: backend.name().to_string(),
        options: HashMap::new(),
        requires: vec![],
    };

    // 1. Test Install
    let inst_res = installer.install(&[spec], false).await;
    assert!(inst_res.is_ok(), "Install failed for {}: {:?}", backend.name(), inst_res.err());

    // 2. Test Remove
    let rem_res = installer.remove(&[package_name.to_string()], false).await;
    assert!(rem_res.is_ok(), "Remove failed for {}: {:?}", backend.name(), rem_res.err());
}

// ============================================================================
// OS-Specific Backend Tests
// ============================================================================

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_apt_backend_hermetic() {
    let env = create_test_env().await;
    let backend = env.registry.get("apt").expect("Missing apt");
    
    env.mock_layer.set_response("apt install -y echo", Ok(DryRunOutput::default().into()));
    env.mock_layer.set_response("apt purge -y echo", Ok(DryRunOutput::default().into()));

    timeout(Duration::from_secs(5), run_capability_test(backend, "echo"))
        .await
        .expect("Test timed out");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_pacman_backend_hermetic() {
    let env = create_test_env().await;
    let backend = env.registry.get("pacman").expect("Missing pacman");
    
    env.mock_layer.set_response("pacman -S --noconfirm --needed echo", Ok(DryRunOutput::default().into()));
    env.mock_layer.set_response("pacman -Rs --noconfirm echo", Ok(DryRunOutput::default().into()));

    timeout(Duration::from_secs(5), run_capability_test(backend, "echo"))
        .await
        .expect("Test timed out");
}

#[cfg(target_os = "windows")]
#[tokio::test]
async fn test_winget_backend_hermetic() {
    let env = create_test_env().await;
    let backend = env.registry.get("winget").expect("Missing winget");
    
    env.mock_layer.set_response("winget install --silent echo", Ok(DryRunOutput::default().into()));
    env.mock_layer.set_response("winget uninstall --silent echo", Ok(DryRunOutput::default().into()));

    timeout(Duration::from_secs(5), run_capability_test(backend, "echo"))
        .await
        .expect("Test timed out");
}

// ============================================================================
// Cross-Platform & Specialized Tests
// ============================================================================

#[tokio::test]
async fn test_brew_backend_hermetic() {
    let env = create_test_env().await;
    let backend = env.registry.get("brew").expect("Missing brew");
    
    env.mock_layer.set_response("brew install echo", Ok(DryRunOutput::default().into()));
    env.mock_layer.set_response("brew uninstall echo", Ok(DryRunOutput::default().into()));

    timeout(Duration::from_secs(5), run_capability_test(backend, "echo"))
        .await
        .expect("Test timed out");
}

#[tokio::test]
async fn test_cargo_backend_hermetic() {
    let env = create_test_env().await;
    let backend = env.registry.get("cargo").expect("Missing cargo");
    
    env.mock_layer.set_response("cargo install ripgrep", Ok(DryRunOutput::default().into()));
    env.mock_layer.set_response("cargo uninstall ripgrep", Ok(DryRunOutput::default().into()));

    timeout(Duration::from_secs(5), run_capability_test(backend, "ripgrep"))
        .await
        .expect("Test timed out");
}

#[tokio::test]
async fn test_npm_backend_hermetic() {
    let env = create_test_env().await;
    let backend = env.registry.get("npm").expect("Missing npm");
    
    env.mock_layer.set_response("npm add --global typescript", Ok(DryRunOutput::default().into()));
    env.mock_layer.set_response("npm uninstall --global typescript", Ok(DryRunOutput::default().into()));

    timeout(Duration::from_secs(5), run_capability_test(backend, "typescript"))
        .await
        .expect("Test timed out");
}

#[tokio::test]
async fn test_link_backend_hermetic() {
    let env = create_test_env().await;
    let backend = env.registry.get("link").expect("Missing link");

    let source_path = env._tmp.path().join("source_file.txt");
    tokio::fs::write(&source_path, "content").await.unwrap();
    let target_path = env._tmp.path().join("target_link.txt");

    let mut options = HashMap::new();
    options.insert("target".to_string(), target_path.to_string_lossy().to_string());

    let spec = PackageSpec {
        name: source_path.to_string_lossy().to_string(),
        backend: "link".to_string(),
        options,
        requires: vec![],
    };

    let installer = backend.as_installable().unwrap();
    
    // Test Install (VFS based check since it's a dry-run)
    installer.install(&[spec], false).await.expect("Link install failed");
    
    let diff = env.executor.get_vfs_diff();
    let link_created = diff.iter().any(|(path, _)| path == &target_path);
    assert!(link_created, "Link was not created in the virtual filesystem");

    // Test Remove
    installer.remove(&[target_path.to_string_lossy().to_string()], false).await.expect("Link remove failed");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_metadata_provider_hermetic() {
    let env = create_test_env().await;
    let backend = env.registry.get("apt").expect("Missing apt");
    let provider = backend.as_metadata_provider().expect("Apt must provide metadata");

    let mock_output = "Depends: libc6\nDepends: bash\n";
    env.mock_layer.set_response(
        "apt depends --no-recommends --no-suggests curl", 
        Ok(DryRunOutput { stdout: mock_output.as_bytes().to_vec(), stderr: vec![] }.into())
    );

    let deps = provider.get_dependencies("curl").await.expect("Failed to get deps");
    assert!(deps.contains(&"libc6".to_string()));
    assert!(deps.contains(&"bash".to_string()));
}