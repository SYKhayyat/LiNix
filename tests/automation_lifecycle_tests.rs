use linix::core::SnapshotManager;

// Import our exhaustive A+ Test Infrastructure
mod mock_providers;
use mock_providers::{MockSnapshotProvider, TestKernel};

// ============================================================================
// FEATURE 2: SNAPSHOT LIFECYCLE (PHYSICAL PRUNING)
// ============================================================================

/// Verifies that the SnapshotManager correctly identifies stale snapshots
/// by age and count, and physically invokes the provider's delete API
/// when is_dry_run is false.
#[tokio::test]
async fn test_snapshot_pruning_physical_logic() {
    // 1. Initialize hermetic test environment (Async DI bootstrap)
    let _kernel = TestKernel::new().await;

    // 2. Setup Mock Provider with 4 snapshots of varying ages
    let mock_provider = MockSnapshotProvider::new();

    // Kept: Recent (Taken 1 day ago - Within 30 day limit)
    mock_provider
        .add_historical_snapshot("snap_keep_recent", 1)
        .await;
    // Kept: Recent (Taken 2 days ago - Fits within max_count of 2)
    mock_provider
        .add_historical_snapshot("snap_keep_count", 2)
        .await;
    // Pruned: Stale (Taken 45 days ago - Exceeds 30 day limit)
    mock_provider
        .add_historical_snapshot("snap_prune_age", 45)
        .await;
    // Pruned: Overflow (Taken 5 days ago - Within age limit but exceeds count of 2)
    mock_provider
        .add_historical_snapshot("snap_prune_count_overflow", 5)
        .await;

    // Use the DI Factory to inject the mock provider
    let manager = SnapshotManager::with_provider(Box::new(mock_provider));

    // 3. Execute Pruning Logic: Max Age: 30, Max Count: 2, is_dry_run: false
    // Resolves E0282: Type is inferred from the awaited call
    manager
        .prune_stale_snapshots(
            30, 2, false, // Physical deletion enabled
        )
        .await
        .expect("Critical Failure: Snapshot pruning orchestration crashed.");

    // 4. Verification: Logic check of remaining closure
    let remaining = manager.list_snapshots().await.unwrap();
    let remaining_ids: Vec<String> = remaining.iter().map(|s| s.id.clone()).collect();

    assert_eq!(
        remaining.len(),
        2,
        "Pruning engine failed to respect the max_count limit."
    );
    assert!(remaining_ids.contains(&"snap_keep_recent".to_string()));
    assert!(remaining_ids.contains(&"snap_keep_count".to_string()));

    // Verify physical removal from provider
    assert!(
        !remaining_ids.contains(&"snap_prune_age".to_string()),
        "Age-based pruning failed to physically delete snapshot."
    );
    assert!(
        !remaining_ids.contains(&"snap_prune_count_overflow".to_string()),
        "Count-based pruning failed to physically delete snapshot."
    );
}

/// Verifies that the SnapshotManager respects the dry-run flag, ensuring
/// no physical deletions occur even if snapshots are identified as stale.
#[tokio::test]
async fn test_snapshot_pruning_respects_dry_run_safety() {
    let mock_provider = MockSnapshotProvider::new();
    mock_provider
        .add_historical_snapshot("snap_stale_target", 100)
        .await;

    let manager = SnapshotManager::with_provider(Box::new(mock_provider));

    // Execute with is_dry_run = true
    manager.prune_stale_snapshots(1, 0, true).await.unwrap();

    // Verification: Snapshot must physically remain in the store
    let list = manager.list_snapshots().await.unwrap();
    assert_eq!(
        list.len(),
        1,
        "Snapshot was physically deleted despite dry-run being active."
    );
}

// ============================================================================
// FEATURE 5: SCHEDULER (CRON TRANSLATION ACCURACY)
// ============================================================================

/// Verifies that standard cron strings are translated to Systemd OnCalendar
/// specs with high precision.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_systemd_oncalendar_translation_logic() {
    let kernel = TestKernel::new().await;

    // Input: Every Monday at 4:30 AM ("30 4 * * 1")
    let cron = "30 4 * * 1";
    let mut config = kernel.app.config.as_ref().clone();

    // Execute scheduling logic via the kernel scheduler
    kernel
        .app
        .scheduler
        .add_schedule(
            &kernel.app.executor,
            &mut config,
            "weekly-sync-task".into(),
            cron.into(),
            "sync".into(),
            None,
        )
        .await
        .expect("Scheduler failed to provision Systemd mock units.");

    // Inspect the Virtual File System (VFS) for the generated .timer content
    let vfs_diff = kernel.app.executor.get_vfs_diff();
    let (_, timer_content) = vfs_diff
        .iter()
        .find(|(path, _)| {
            path.to_string_lossy()
                .contains("linix-weekly-sync-task.timer")
        })
        .expect("Systemd timer unit was not written to VFS.");

    // A+ Validation: The "hourly" stub must be replaced by a precise OnCalendar mapping
    assert!(
        timer_content.contains("OnCalendar=Mon *-*-* 04:30:00"),
        "Incorrect Systemd OnCalendar translation generated.\nContent: {}",
        timer_content
    );

    // Verify Call Log: Check if systemctl reload/enable was "recorded"
    kernel.assert_called("systemctl --user daemon-reload").await;
    kernel
        .assert_called("systemctl --user enable --now linix-weekly-sync-task.timer")
        .await;
}

/// Verifies that cron strings are accurately translated to macOS Launchd
/// dictionaries for the StartCalendarInterval key.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_launchd_plist_translation_logic() {
    let kernel = TestKernel::new().await;

    // Input: 15th of every month at 2:15 AM ("15 2 15 * *")
    let cron = "15 2 15 * *";
    let mut config = kernel.app.config.as_ref().clone();

    kernel
        .app
        .scheduler
        .add_schedule(
            &kernel.app.executor,
            &mut config,
            "monthly-maintenance-job".into(),
            cron.into(),
            "upgrade".into(),
            None,
        )
        .await
        .expect("Scheduler failed to provision macOS mock Plist.");

    // Verify VFS Content
    let vfs_diff = kernel.app.executor.get_vfs_diff();
    let (_, plist_content) = vfs_diff
        .iter()
        .find(|(p, _)| {
            p.to_string_lossy()
                .contains("com.linix.monthly-maintenance-job.plist")
        })
        .expect("macOS Plist was not written to VFS.");

    // A+ Validation: Verify XML Keys for StartCalendarInterval dictionary
    assert!(
        plist_content.contains("<key>Day</key><integer>15</integer>"),
        "Missing Month Day mapping in Plist."
    );
    assert!(
        plist_content.contains("<key>Hour</key><integer>2</integer>"),
        "Missing Hour mapping in Plist."
    );
    assert!(
        plist_content.contains("<key>Minute</key><integer>15</integer>"),
        "Missing Minute mapping in Plist."
    );

    // Verify Call Log
    kernel.assert_called("launchctl load").await;
}

/// Verifies that the @reboot special string correctly triggers platform-native
/// boot-time execution logic.
#[tokio::test]
async fn test_scheduler_reboot_mapping_fidelity() {
    let kernel = TestKernel::new().await;
    let mut config = kernel.app.config.as_ref().clone();

    kernel
        .app
        .scheduler
        .add_schedule(
            &kernel.app.executor,
            &mut config,
            "reboot-cleanup".into(),
            "@reboot".into(),
            "clean".into(),
            None,
        )
        .await
        .unwrap();

    #[cfg(target_os = "linux")]
    {
        let vfs_diff = kernel.app.executor.get_vfs_diff();
        let (_, service_content) = vfs_diff
            .iter()
            .find(|(p, _)| p.to_string_lossy().contains("linix-reboot-cleanup.service"))
            .expect("Systemd service file missing from VFS.");

        // A+ Logic: @reboot in Linux must use the default.target dependency
        assert!(
            service_content.contains("WantedBy=default.target"),
            "Systemd @reboot mapping failed to use correct target dependency."
        );
    }

    #[cfg(target_os = "macos")]
    {
        let vfs_diff = kernel.app.executor.get_vfs_diff();
        let (_, plist_content) = vfs_diff
            .iter()
            .find(|(p, _)| {
                p.to_string_lossy()
                    .contains("com.linix.reboot-cleanup.plist")
            })
            .expect("macOS Plist missing from VFS.");

        // A+ Logic: @reboot in macOS must use the RunAtLoad key
        assert!(
            plist_content.contains("<key>RunAtLoad</key><true/>"),
            "macOS @reboot mapping failed to use RunAtLoad key."
        );
    }

    #[cfg(target_os = "windows")]
    {
        // On Windows, @reboot maps to the ONSTART trigger in schtasks
        kernel.assert_called("schtasks /Create").await;
        kernel.assert_called("/SC ONSTART").await;
    }
}
