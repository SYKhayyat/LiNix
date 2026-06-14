use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use crate::utils::{safe_data_dir, safe_config_dir};

/// Configuration for platform-specific sandboxing behaviors.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SandboxSettings {
    /// On Linux, if true, LiNix will fail if 'bwrap' is missing. 
    #[serde(default = "default_false")]
    pub require_bwrap: bool,

    /// Optional path to a custom macOS sandbox (.sb) profile.
    pub macos_profile_template: Option<PathBuf>,

    /// On Windows, if true, LiNix will fail if Windows Sandbox is unavailable.
    #[serde(default = "default_false")]
    pub windows_require_sandbox: bool,

    /// If true, allow running without OS-level isolation if mechanisms are missing.
    #[serde(default = "default_true")]
    pub fallback_allowed: bool,
}

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            require_bwrap: false,
            macos_profile_template: None,
            windows_require_sandbox: false,
            fallback_allowed: true,
        }
    }
}

/// Feature 2: Settings for automatic system snapshot management.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SnapshotSettings {
    /// Maximum age of a snapshot in days before it is pruned.
    #[serde(default = "default_max_age")]
    pub max_age_days: u32,

    /// Maximum number of snapshots to keep.
    #[serde(default = "default_max_count")]
    pub max_count: u32,

    /// If true, prune snapshots automatically after successful transactions.
    #[serde(default = "default_true")]
    pub auto_prune: bool,
}

impl Default for SnapshotSettings {
    fn default() -> Self {
        Self {
            max_age_days: 30,
            max_count: 10,
            auto_prune: true,
        }
    }
}

/// Feature 5: Configuration for background scheduled tasks.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScheduleConfig {
    /// Unique identifier for the task.
    pub name: String,
    /// Cron expression (e.g., "0 2 * * *").
    pub cron: String,
    /// The LiNix command to run.
    pub command: String,
    /// Notification channel: "desktop", "email", or "none".
    pub notification: Option<String>,
    /// Last time the task was successfully verified in the system scheduler.
    pub last_synced: Option<chrono::DateTime<chrono::Utc>>,
}

/// The primary configuration for LiNix Version 3.6.0.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    
    #[serde(default)]
    pub groups: HashMap<String, Vec<String>>,
    
    #[serde(default)]
    pub dry_run: bool,
    
    #[serde(default)]
    pub yes: bool,
    
    #[serde(default = "default_groups_dir")]
    pub groups_dir: PathBuf,

    /// Feature 3: Directory containing reusable .module.txt files.
    #[serde(default = "default_modules_dir")]
    pub modules_dir: PathBuf,
    
    #[serde(skip)]
    pub config_file: PathBuf,
    
    #[serde(default)]
    pub enabled_backends: Vec<String>,

    #[serde(default = "default_priority")]
    pub backend_priority: Vec<String>,
    
    #[serde(default)]
    pub hooks: HashMap<String, HashMap<String, String>>,
    
    #[serde(default)]
    pub hostname_packages: HashMap<String, Vec<String>>,
    
    #[serde(default = "default_bloatware_file")]
    pub bloatware_file: PathBuf,
    
    #[serde(default)]
    pub remove_bloatware: bool,

    #[serde(default = "default_false")]
    pub purge_orphans: bool,

    #[serde(default = "default_true")]
    pub auto_lock_checksums: bool,
    
    #[serde(default = "default_true")]
    pub show_progress: bool,
    
    #[serde(default)]
    pub verbose: bool,
    
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl: u64,
    
    #[serde(default)]
    pub github_token: Option<String>,
    
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
    
    #[serde(default)]
    pub backend_settings: HashMap<String, HashMap<String, String>>,
    
    #[serde(default)]
    pub default_backend: Option<String>,
    
    #[serde(default = "default_protected_packages")]
    pub protected_packages: Vec<String>,

    #[serde(default = "default_btrfs_path")]
    pub btrfs_path: String,

    #[serde(default = "default_timeshift_path")]
    pub timeshift_path: String,

    pub zfs_dataset: Option<String>,

    #[serde(default = "default_tmp_dir")]
    pub tmp_dir: PathBuf,

    #[serde(default = "default_github_dir")]
    pub github_dir: PathBuf,

    #[serde(default = "default_web_dir")]
    pub web_dir: PathBuf,

    #[serde(default = "default_appimage_dir")]
    pub appimage_dir: PathBuf,

    #[serde(default)]
    pub sandbox: SandboxSettings,

    /// Feature 2: Snapshot pruning configuration.
    #[serde(default)]
    pub snapshots: SnapshotSettings,

    /// Feature 5: Native background schedules.
    #[serde(default)]
    pub schedules: Vec<ScheduleConfig>,
}

fn default_groups_dir() -> PathBuf { safe_config_dir().join("groups") }
fn default_modules_dir() -> PathBuf { safe_config_dir().join("modules") }
fn default_bloatware_file() -> PathBuf { safe_config_dir().join("bloatware.txt") }
fn default_btrfs_path() -> String { "/.snapshots".to_string() }
fn default_timeshift_path() -> String { "/run/timeshift/backup/timeshift/snapshots".to_string() }
fn default_tmp_dir() -> PathBuf { safe_data_dir().join("tmp") }
fn default_github_dir() -> PathBuf { safe_data_dir().join("github") }
fn default_web_dir() -> PathBuf { safe_data_dir().join("web") }
fn default_appimage_dir() -> PathBuf { safe_data_dir().join("appimages") }
fn default_true() -> bool { true }
fn default_false() -> bool { false }
fn default_cache_ttl() -> u64 { 300 }
fn default_max_parallel() -> usize { 4 }
fn default_max_age() -> u32 { 30 }
fn default_max_count() -> u32 { 10 }

fn default_priority() -> Vec<String> {
    vec![
        "apt".into(), "pacman".into(), "dnf".into(), "winget".into(),
        "brew".into(), "flatpak".into(), "snap".into(), "cargo".into(),
        "npm".into(), "pip".into(),
    ]
}

fn default_protected_packages() -> Vec<String> {
    let mut packages = vec!["sudo".into(), "bash".into(), "linix".into()];
    #[cfg(target_os = "linux")]
    {
        packages.extend(vec![
            "linux-image".into(), "linux-headers".into(), "kernel".into(), "systemd".into(),
            "libc6".into(), "libc".into(), "glibc".into(), "grub".into(), "grub2".into(),
            "coreutils".into(), "filesystem".into(), "apt".into(), "pacman".into(),
            "dnf".into(), "rpm".into(),
        ]);
    }
    #[cfg(target_os = "windows")]
    {
        packages.extend(vec!["windows".into(), "win32".into(), "kernel32".into(), "ntdll.dll".into()]);
    }
    #[cfg(target_os = "macos")]
    {
        packages.extend(vec!["darwin".into(), "xnu".into()]);
    }
    packages
}

impl Default for Config {
    fn default() -> Self {
        Self {
            aliases: HashMap::new(),
            groups: HashMap::new(),
            dry_run: false,
            yes: false,
            groups_dir: default_groups_dir(),
            modules_dir: default_modules_dir(),
            config_file: safe_config_dir().join("config.toml"),
            enabled_backends: Vec::new(),
            backend_priority: default_priority(),
            hooks: HashMap::new(),
            hostname_packages: HashMap::new(),
            bloatware_file: default_bloatware_file(),
            remove_bloatware: false,
            purge_orphans: false,
            auto_lock_checksums: true,
            show_progress: true,
            verbose: false,
            cache_ttl: 300,
            github_token: None,
            max_parallel: 4,
            backend_settings: HashMap::new(),
            default_backend: None,
            protected_packages: default_protected_packages(),
            btrfs_path: default_btrfs_path(),
            timeshift_path: default_timeshift_path(),
            zfs_dataset: None,
            tmp_dir: default_tmp_dir(),
            github_dir: default_github_dir(),
            web_dir: default_web_dir(),
            appimage_dir: default_appimage_dir(),
            sandbox: SandboxSettings::default(),
            snapshots: SnapshotSettings::default(),
            schedules: Vec::new(),
        }
    }
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        if !path.exists() { return Ok(Self::default()); }
        let content = fs::read_to_string(path).map_err(|e| Error::Config(format!("Failed to read config file: {}", e)))?;
        let mut config: Self = toml::from_str(&content)?;
        config.config_file = path.to_path_buf();
        if config.protected_packages.is_empty() {
            config.protected_packages = default_protected_packages();
        }
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| Error::Config(format!("Failed to serialize config: {}", e)))?;
        if let Some(parent) = self.config_file.parent() {
            fs::create_dir_all(parent).map_err(Error::from)?;
        }
        fs::write(&self.config_file, content).map_err(|e| Error::Config(format!("Failed to write config file: {}", e)))?;
        Ok(())
    }

    pub fn get_hostname() -> String {
        hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_else(|| "unknown".to_string())
    }

    pub fn merge_cli_overrides(
        &mut self,
        dry_run: Option<bool>,
        yes: Option<bool>,
        backend: Option<String>,
        config_path: Option<PathBuf>,
        groups_dir: Option<PathBuf>,
        verbose: Option<bool>,
    ) {
        if let Some(dr) = dry_run { self.dry_run = dr; }
        if let Some(y) = yes { self.yes = y; }
        if let Some(b) = backend { self.enabled_backends = vec![b]; }
        if let Some(cp) = config_path { self.config_file = cp; }
        if let Some(gd) = groups_dir { self.groups_dir = gd; }
        if let Some(v) = verbose { self.verbose = v; }
    }

    pub fn validate(&self) -> Result<()> {
        if self.max_parallel == 0 {
            return Err(Error::Config("max_parallel must be greater than 0".into()));
        }
        // Verify cron strings for any schedules
        for schedule in &self.schedules {
            if let Err(e) = schedule.cron.parse::<cron::Schedule>() {
                return Err(Error::Config(format!("Invalid cron expression for task '{}': {}", schedule.name, e)));
            }
        }
        Ok(())
    }
    
    pub fn is_protected(&self, package_name: &str) -> bool {
        let name_lower = package_name.to_lowercase();
        self.protected_packages.iter().any(|p| {
            let p_lower = p.to_lowercase();
            name_lower == p_lower || name_lower.contains(&p_lower)
        })
    }
}