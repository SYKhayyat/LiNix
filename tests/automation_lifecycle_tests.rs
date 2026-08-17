use shall::core::SnapshotManager;

// Import our exhaustive A+ Test Infrastructure
use crate::mock_providers::{MockSnapshotProvider, TestKernel};

// ============================================================================
// FEATURE 2: SNAPSHOT LIFECYCLE (PHYSICAL PRUNING)
// ============================================================================

/// The one retention engine (`prune_with_policy`): it reaps only Shall-owned snapshots, and
/// always keeps the most recent one — the floor the old `prune_stale_snapshots` lacked.
#[tokio::test]
async fn test_snapshot_retention_reaps_only_shall_owned_and_keeps_the_newest() {
    use shall::core::RetentionPolicy;
    let _kernel = TestKernel::new().await;
    let mock_provider = MockSnapshotProvider::new();

    // Shall-owned (the id carries `shall_`): one recent, one ancient.
    mock_provider
        .add_historical_snapshot("shall_recent", 1)
        .await;
    mock_provider
        .add_historical_snapshot("shall_ancient", 45)
        .await;
    // NOT Shall's — a user or other-tool snapshot. Retention must never touch it.
    mock_provider
        .add_historical_snapshot("weekly_backup", 90)
        .await;

    let manager = SnapshotManager::with_provider(Box::new(mock_provider));

    // Keep the single newest, plus anything under 30 days.
    let policy = RetentionPolicy {
        keep_last: 1,
        keep_days: 30,
        keep: vec![],
    };
    manager
        .prune_with_policy(&policy, chrono::Utc::now(), false)
        .await
        .expect("retention prune crashed");

    let remaining: Vec<String> = manager
        .list_snapshots()
        .await
        .unwrap()
        .iter()
        .map(|s| s.id.clone())
        .collect();

    // shall_ancient: not the newest, and 45 > 30 days -> reaped.
    assert!(
        !remaining.contains(&"shall_ancient".to_string()),
        "the ancient Shall snapshot should have been reaped"
    );
    // shall_recent: the newest -> kept by the floor.
    assert!(remaining.contains(&"shall_recent".to_string()));
    // weekly_backup: not Shall's -> never touched, even at 90 days.
    assert!(
        remaining.contains(&"weekly_backup".to_string()),
        "retention must not reap a snapshot Shall did not create"
    );
}

/// Dry-run identifies what it *would* delete but touches nothing.
#[tokio::test]
async fn test_snapshot_retention_respects_dry_run() {
    use shall::core::RetentionPolicy;
    let mock_provider = MockSnapshotProvider::new();
    // Two owned snapshots so one is past the always-keep-newest floor.
    mock_provider
        .add_historical_snapshot("shall_newest", 1)
        .await;
    mock_provider
        .add_historical_snapshot("shall_stale", 100)
        .await;
    let manager = SnapshotManager::with_provider(Box::new(mock_provider));

    let policy = RetentionPolicy {
        keep_last: 0,
        keep_days: 1,
        keep: vec![],
    };
    let doomed = manager
        .prune_with_policy(&policy, chrono::Utc::now(), true)
        .await
        .unwrap();

    // shall_stale is past the floor and older than 1 day -> identified...
    assert!(doomed.contains(&"shall_stale".to_string()));
    // ...but dry-run physically deletes nothing.
    assert_eq!(
        manager.list_snapshots().await.unwrap().len(),
        2,
        "dry-run must not physically delete"
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

    // Execute scheduling logic via the kernel scheduler
    kernel
        .app
        .scheduler
        .provision(
            &kernel.app.executor,
            &shall::config::config::ScheduleConfig {
                name: "weekly-sync-task".into(),
                cron: cron.into(),
                command: "sync".into(),
                notification: None,
                enabled: None,
                persistent: None,
                jitter: None,
                elevated: None,
            },
        )
        .await
        .expect("Scheduler failed to provision Systemd mock units.");

    // Inspect the Virtual File System (VFS) for the generated .timer content
    let vfs_diff = kernel.app.executor.get_vfs_diff();
    let (_, timer_content) = vfs_diff
        .iter()
        .find(|(path, _)| {
            path.to_string_lossy()
                .contains("shall-weekly-sync-task.timer")
        })
        .expect("Systemd timer unit was not written to VFS.");

    // A+ Validation: The "hourly" stub must be replaced by a precise OnCalendar mapping
    assert!(
        timer_content.contains("OnCalendar=Mon *-*-* 04:30:00"),
        "Incorrect Systemd OnCalendar translation generated.\nContent: {}",
        timer_content
    );

    // Verify Call Log: Check if systemctl reload/enable was "recorded"
    // `--no-pager` is part of the argv, not decoration: systemctl decides to page from the
    // terminal and from $SYSTEMD_PAGER, and a pager waits for a keypress no captured child
    // receives (S43).
    kernel
        .assert_called("systemctl --no-pager --user daemon-reload")
        .await;
    kernel
        .assert_called("systemctl --no-pager --user enable --now shall-weekly-sync-task.timer")
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

    kernel
        .app
        .scheduler
        .provision(
            &kernel.app.executor,
            &shall::config::config::ScheduleConfig {
                name: "monthly-maintenance-job".into(),
                cron: cron.into(),
                command: "upgrade".into(),
                notification: None,
                enabled: None,
                persistent: None,
                jitter: None,
                elevated: None,
            },
        )
        .await
        .expect("Scheduler failed to provision macOS mock Plist.");

    // Verify VFS Content
    let vfs_diff = kernel.app.executor.get_vfs_diff();
    let (_, plist_content) = vfs_diff
        .iter()
        .find(|(p, _)| {
            p.to_string_lossy()
                .contains("com.shall.monthly-maintenance-job.plist")
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

    kernel
        .app
        .scheduler
        .provision(
            &kernel.app.executor,
            &shall::config::config::ScheduleConfig {
                name: "reboot-cleanup".into(),
                cron: "@reboot".into(),
                command: "clean".into(),
                notification: None,
                enabled: None,
                persistent: None,
                jitter: None,
                elevated: None,
            },
        )
        .await
        .unwrap();

    #[cfg(target_os = "linux")]
    {
        let vfs_diff = kernel.app.executor.get_vfs_diff();
        let (_, service_content) = vfs_diff
            .iter()
            .find(|(p, _)| p.to_string_lossy().contains("shall-reboot-cleanup.service"))
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
                    .contains("com.shall.reboot-cleanup.plist")
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
