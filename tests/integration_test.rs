use linix::app::App;
use linix::config::Config;
use linix::core::Package;

#[tokio::test]
async fn test_app_creation() {
    let config = Config::default();
    let result = App::new(config).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_backend_detection() {
    let config = Config::default();
    let app = App::new(config).await.unwrap();
    let backends = app.available_backends();

    // Should return a list (may be empty on some systems)
    println!("Available backends: {:?}", backends);
}

#[tokio::test]
async fn test_package_struct() {
    let pkg = Package::new("test-package", "apt");
    assert_eq!(pkg.name, "test-package");
    assert_eq!(pkg.backend, "apt");
    assert!(pkg.version.is_none());

    let pkg_with_version = Package::with_version("test", "1.0.0", "apt");
    assert_eq!(pkg_with_version.version, Some("1.0.0".to_string()));
}

#[tokio::test]
async fn test_config_default() {
    let config = Config::default();
    assert!(!config.dry_run);
    assert!(!config.yes);
    assert_eq!(config.max_parallel, 4);
    assert!(config.show_progress);
}

#[tokio::test]
async fn test_config_merge_overrides() {
    let mut config = Config::default();
    config.merge_cli_overrides(
        Some(true),              // dry_run
        Some(true),              // yes
        Some("apt".to_string()), // backend
        None,
        None,
        Some(true), // verbose
    );

    assert!(config.dry_run);
    assert!(config.yes);
    assert!(config.verbose);
    assert_eq!(config.enabled_backends, vec!["apt"]);
}

#[tokio::test]
async fn test_validator() {
    use linix::core::Validator;

    // Valid package names
    assert!(Validator::validate_package_name("valid-package").is_ok());
    assert!(Validator::validate_package_name("package_name").is_ok());
    assert!(Validator::validate_package_name("@scope/package").is_ok());

    // Invalid package names
    assert!(Validator::validate_package_name("").is_err());
    assert!(Validator::validate_package_name("invalid package").is_err());
    assert!(Validator::validate_package_name("invalid;package").is_err());
}

#[tokio::test]
async fn test_cache() {
    use linix::core::PackageCache;

    let cache = PackageCache::new();

    // Initially empty
    assert!(cache.get_installed("apt").await.is_none());

    // Set and get
    let packages = vec!["pkg1".to_string(), "pkg2".to_string()];
    cache
        .set_installed("apt".to_string(), packages.clone())
        .await;

    let retrieved = cache.get_installed("apt").await;
    assert_eq!(retrieved, Some(packages));

    // Clear
    cache.clear_all().await;
    assert!(cache.get_installed("apt").await.is_none());
}

#[tokio::test]
async fn test_metrics_collector() {
    use linix::app::MetricsCollector;

    let metrics = MetricsCollector::new();

    metrics.start_operation("test");
    metrics.record_install(5);
    metrics.end_operation("test");

    let report = metrics.report();
    assert_eq!(report.packages_installed, 5);
    assert!(report.success);
}

#[tokio::test]
async fn test_rate_limiter() {
    use linix::core::RateLimiter;

    let limiter = RateLimiter::new(10);

    // First requests should succeed
    assert!(limiter.try_request().is_ok());
    assert!(limiter.try_request().is_ok());
}
