// tests/mock_providers.rs
#![allow(clippy::field_reassign_with_default, dead_code)]

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use dashmap::DashMap;
use linix::app::scheduler::TaskProvisioner;
use linix::app::App;
use linix::config::config::ScheduleConfig;
use linix::config::Config;
use linix::core::executor::MockExecutor;
use linix::core::{
    CommandExecutor, Error, PackageSpec, Result, Snapshot, SnapshotProvider, StateRegistry,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;

#[allow(dead_code)]
pub struct TestKernel {
    pub app: App,
    pub mock_executor: Arc<MockExecutor>,
    pub tmp: TempDir,
    pub state: Arc<Mutex<StateRegistry>>,
    vfs: Arc<DashMap<std::path::PathBuf, String>>,
    lock_map: Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl TestKernel {
    pub async fn new() -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("linix_hermetic_")
            .tempdir()
            .expect("Failed to create test sandbox directory.");

        let registry_path = tmp.path().join("registry.json");

        let mut config = Config::default();
        config.config_root = tmp.path().to_path_buf();
        // Isolate LiNix's DATA root too, structurally and in one place (S11): the layout's
        // snapshots (and anything else derived from the data root) now target the sandbox, not
        // the developer's real data dir — no per-test `$LINIX_DATA_DIR` remembering required.
        config.data_root = tmp.path().to_path_buf();
        config.tmp_dir = tmp.path().join("tmp");
        config.dry_run = true;

        // The II.1 layout, because the resolver now reads a repo rather than a folder of
        // manifests. `priority` says which package managers this machine uses: without it
        // LiNix refuses to guess, so a fixture without one is a fixture that cannot resolve
        // anything. These are the three the mock executor pretends to have.
        std::fs::write(tmp.path().join("priority"), "apt\nbrew\ncargo\n")
            .expect("Failed to write test `priority`.");
        std::fs::create_dir_all(tmp.path().join("modules"))
            .expect("Failed to create test `modules/`.");
        std::fs::create_dir_all(tmp.path().join("profiles"))
            .expect("Failed to create test `profiles/`.");
        // A profile has to be active or there is nowhere for a line to go: a module no
        // profile reaches is one LiNix never reads, so writing to one is refused rather
        // than done silently. A fixture with nothing active is a fixture that cannot
        // install. Tests that care about a specific profile overwrite these two files.
        std::fs::write(tmp.path().join("profiles/Main"), "")
            .expect("Failed to write test profile.");
        std::fs::write(tmp.path().join("active"), "Main\n")
            .expect("Failed to write test `active`.");

        let vfs = Arc::new(DashMap::new());
        let lock_map = Arc::new(DashMap::new());

        let mock_executor = Arc::new(MockExecutor::new(vfs.clone()));
        mock_executor.set_command_exists("brew", true);
        mock_executor.set_command_exists("apt", true);
        mock_executor.set_command_exists("cargo", true);

        let executor = CommandExecutor::with_layer(
            true,
            false,
            mock_executor.clone(),
            vfs.clone(),
            lock_map.clone(),
        );

        let app = App::new_with_executor_and_state_path(config, executor, Some(registry_path))
            .await
            .expect("Failed to bootstrap Test Kernel.");

        let state = app.state.clone();

        Self {
            app,
            mock_executor,
            tmp,
            state,
            vfs,
            lock_map,
        }
    }

    #[allow(dead_code)]
    pub async fn assert_called(&self, command_fragment: &str) {
        let calls: Vec<String> = self.mock_executor.get_calls().await;
        assert!(
            calls.iter().any(|c| c.contains(command_fragment)),
            "\nCOMMAND VERIFICATION FAILED\nExpected Fragment: '{}'\nActual Call Log:   {:?}\n",
            command_fragment,
            calls
        );
    }
}

#[allow(dead_code)]
pub struct MockSnapshotProvider {
    pub store: Arc<Mutex<Vec<Snapshot>>>,
    pub deletions: Arc<Mutex<Vec<String>>>,
}

#[allow(dead_code)]
impl Default for MockSnapshotProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockSnapshotProvider {
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(Vec::new())),
            deletions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn add_historical_snapshot(&self, id: &str, days_ago: i64) {
        let timestamp = Utc::now() - ChronoDuration::days(days_ago);
        let mut list = self.store.lock().await;
        list.push(Snapshot {
            id: id.to_string(),
            timestamp: timestamp.to_rfc3339(),
            description: format!("Historical state from {} days ago", days_ago),
            backend: "mock".into(),
        });
    }
}

#[async_trait]
impl SnapshotProvider for MockSnapshotProvider {
    fn name(&self) -> &str {
        "mock"
    }
    async fn is_available(&self) -> bool {
        true
    }
    async fn create(&self, label: &str) -> Result<Snapshot> {
        let s = Snapshot {
            id: format!("snap_{}", Utc::now().timestamp()),
            timestamp: Utc::now().to_rfc3339(),
            description: label.to_string(),
            backend: "mock".into(),
        };
        self.store.lock().await.push(s.clone());
        Ok(s)
    }
    async fn list(&self) -> Result<Vec<Snapshot>> {
        Ok(self.store.lock().await.clone())
    }
    async fn delete(&self, id: &str) -> Result<()> {
        let mut list = self.store.lock().await;
        if let Some(pos) = list.iter().position(|s| s.id == id) {
            list.remove(pos);
            self.deletions.lock().await.push(id.to_string());
            Ok(())
        } else {
            Err(Error::Snapshot(format!(
                "Mock Logic Failure: Snapshot {} missing.",
                id
            )))
        }
    }
    async fn restore(&self, _id: &str) -> Result<()> {
        Ok(())
    }
}

#[allow(dead_code)]
pub struct MockTaskProvisioner {
    pub active_tasks: Arc<Mutex<HashMap<String, ScheduleConfig>>>,
}

#[allow(dead_code)]
impl Default for MockTaskProvisioner {
    fn default() -> Self {
        Self::new()
    }
}

impl MockTaskProvisioner {
    pub fn new() -> Self {
        Self {
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl TaskProvisioner for MockTaskProvisioner {
    async fn add_task(
        &self,
        _executor: &CommandExecutor,
        config: &ScheduleConfig,
        _linix_path: &Path,
    ) -> Result<()> {
        let mut map = self.active_tasks.lock().await;
        map.insert(config.name.clone(), config.clone());
        Ok(())
    }
    async fn remove_task(&self, _executor: &CommandExecutor, name: &str) -> Result<()> {
        let mut map = self.active_tasks.lock().await;
        map.remove(name);
        Ok(())
    }
    async fn is_task_active(&self, _executor: &CommandExecutor, name: &str) -> bool {
        let map = self.active_tasks.lock().await;
        map.contains_key(name)
    }
}

#[allow(dead_code)]
pub fn create_dummy_spec(name: &str, backend: &str, source: Option<&str>) -> PackageSpec {
    let mut options = HashMap::new();
    if let Some(src) = source {
        options.insert("__source".to_string(), src.to_string());
    }
    PackageSpec {
        name: name.to_string(),
        backend: backend.to_string(),
        options,
        requires: Vec::new(),
        present: true,
    }
}
