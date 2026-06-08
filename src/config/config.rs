use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use crate::utils::{safe_data_dir, safe_config_dir};

/// Configuration for platform-specific sandboxing behaviors.
/// Fulfills Phase 3.1 of the Mission-Critical Plan.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SandboxSettings {
    /// On Linux, if true, LiNix will fail if 'bwrap' is missing. 
    /// If false, it falls back to PATH-only isolation.
    #[serde(default = "default_false")]
    pub require_bwrap: bool,

    /// Optional path to a custom macOS sandbox (.sb) profile.
    pub macos_profile_template: Option<PathBuf>,

    /// On Windows, if true, LiNix will fail if Windows Sandbox is unavailable.
    /// If false, it falls back to restricted integrity levels.
    #[serde(default = "default_false")]
    pub windows_require_sandbox: bool,

    /// If true, LiNix will allow running without true OS-level isolation 
    /// if the primary sandbox mechanism is missing.
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

/// The primary configuration for LiNix Version 3.5.0 "Consistency & Integrity."
/// Central source of truth for execution parameters, backend behavior, and system identity.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Maps generic backend names to specific implementations (e.g., "sys" -> "apt").
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    
    /// User-defined package groups in config.toml (e.g., "dev" -> ["git", "vim"]).
    #[serde(default)]
    pub groups: HashMap<String, Vec<String>>,
    
    /// If true, no system modifications will be performed.
    #[serde(default)]
    pub dry_run: bool,
    
    /// If true, all confirmation prompts are skipped.
    #[serde(default)]
    pub yes: bool,
    
    /// The directory where .txt group files are stored.
    #[serde(default = "default_groups_dir")]
    pub groups_dir: PathBuf,
    
    /// The path to the active configuration file (internal tracking).
    #[serde(skip)]
    pub config_file: PathBuf,
    
    /// A whitelist of backends allowed to run. If empty, all available backends are used.
    #[serde(default)]
    pub enabled_backends: Vec<String>,

    /// The order in which backends are queried during a universal search or discovery.
    #[serde(default = "default_priority")]
    pub backend_priority: Vec<String>,
    
    /// Lua and Rhai scripts mapped to package lifecycle events.
    #[serde(default)]
    pub hooks: HashMap<String, HashMap<String, String>>,
    
    /// Packages to be installed only on specific machines, identified by hostname.
    #[serde(default)]
    pub hostname_packages: HashMap<String, Vec<String>>,
    
    /// Path to the list of packages designated for removal during a 'clean' sync.
    #[serde(default = "default_bloatware_file")]
    pub bloatware_file: PathBuf,
    
    /// Whether to automatically remove bloatware during system sync.
    #[serde(default)]
    pub remove_bloatware: bool,

    /// Whether to automatically prune unused dependencies (orphans) during sync.
    #[serde(default = "default_false")]
    pub purge_orphans: bool,

    /// Whether to automatically lock checksums into manifests for web/github resources.
    #[serde(default = "default_true")]
    pub auto_lock_checksums: bool,
    
    /// Toggles the indicatif progress bars.
    #[serde(default = "default_true")]
    pub show_progress: bool,
    
    /// Toggles debug-level logging output.
    #[serde(default)]
    pub verbose: bool,
    
    /// Time-to-live for internal package metadata caches (seconds).
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl: u64,
    
    /// Personal Access Token for GitHub API to increase rate limits.
    #[serde(default)]
    pub github_token: Option<String>,
    
    /// Maximum number of parallel tasks in the JoinSet worker pool.
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
    
    /// Arbitrary key-value settings passed to individual backends.
    #[serde(default)]
    pub backend_settings: HashMap<String, HashMap<String, String>>,
    
    /// The backend used if none is specified in a package string.
    #[serde(default)]
    pub default_backend: Option<String>,
    
    /// Configurable list of protected packages that should never be removed.
    #[serde(default = "default_protected_packages")]
    pub protected_packages: Vec<String>,

    /// Path to BTRFS snapshot directory.
    #[serde(default = "default_btrfs_path")]
    pub btrfs_path: String,

    /// Template path for Timeshift snapshots.
    #[serde(default = "default_timeshift_path")]
    pub timeshift_path: String,

    /// Optional specific ZFS dataset to snapshot.
    pub zfs_dataset: Option<String>,

    /// Global temporary directory for build artifacts and downloads.
    #[serde(default = "default_tmp_dir")]
    pub tmp_dir: PathBuf,

    /// Installation root for GitHub-sourced binaries.
    #[serde(default = "default_github_dir")]
    pub github_dir: PathBuf,

    /// Installation root for direct web downloads.
    #[serde(default = "default_web_dir")]
    pub web_dir: PathBuf,

    /// Installation root for standalone AppImages.
    #[serde(default = "default_appimage_dir")]
    pub appimage_dir: PathBuf,

    /// Sandboxing settings for isolated execution.
    #[serde(default)]
    pub sandbox: SandboxSettings,
}

fn default_groups_dir() -> PathBuf { safe_config_dir().join("groups") }
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
        packages.extend(vec!["windows".into(), "win32".into(), "kernel32".into()]);
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
            config_file: PathBuf::from("/etc/linix/config.toml"),
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

    pub fn to_file(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| Error::Config(format!("Failed to serialize config: {}", e)))?;
        fs::write(path, content).map_err(|e| Error::Config(format!("Failed to write config file: {}", e)))?;
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
        Ok(())
    }
    
    pub fn get_protected_packages(&self) -> &[String] {
        &self.protected_packages
    }
    
    pub fn is_protected(&self, package_name: &str) -> bool {
        let name_lower = package_name.to_lowercase();
        self.protected_packages.iter().any(|p| {
            let p_lower = p.to_lowercase();
            name_lower == p_lower || name_lower.contains(&p_lower)
        })
    }
}