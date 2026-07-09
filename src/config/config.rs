use crate::core::{Error, Result};
use crate::utils::{safe_config_dir, safe_data_dir};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Which installed packages drift removal (`prune`, or `sync` with `prune_on_sync`)
/// is allowed to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PruneScope {
    /// Only remove packages under LiNix management that are no longer in the desired
    /// state. Installed-but-unmanaged software is never touched. Safe default.
    #[default]
    Managed,
    /// Remove ANY installed package (across every backend) not present in the desired
    /// state — a true "make the system exactly match my manifests" mode. Protected
    /// packages are always spared. Dangerous: enable deliberately.
    System,
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

    /// Per-host backend allow-lists. When the current host has a (non-empty) entry here,
    /// it overrides the global `enabled_backends` for that host, so a machine can manage
    /// only a subset of backends (e.g. no `npm`/`cargo` on a server). Empty = inherit
    /// the global list (which, when itself empty, means "all backends").
    #[serde(default)]
    pub hostname_backends: HashMap<String, Vec<String>>,

    /// Config files whose *contents* are declared inline here (target path -> file body),
    /// instead of pointing the `link` backend at a separate source file. Each entry is
    /// materialized as a managed `link` file: written on `sync`, self-healed if edited,
    /// tracked for drift, and removed by `prune` when the entry is deleted. A pre-existing
    /// file at the target is backed up once before it is overwritten.
    #[serde(default)]
    pub managed_files: HashMap<String, String>,

    /// How long to retain each of LiNix's three histories — archived manifests,
    /// generations, and filesystem snapshots — each configured independently. See
    /// [`crate::core::RetentionConfig`]. Empty/zero policies keep everything (default).
    #[serde(default)]
    pub retention: crate::core::RetentionConfig,

    #[serde(default = "default_bloatware_file")]
    pub bloatware_file: PathBuf,

    #[serde(default)]
    pub remove_bloatware: bool,

    /// Whether `sync` removes drift (packages installed but no longer in the manifests).
    /// Default false: `sync` only installs/upgrades, and drift removal is an explicit,
    /// separate step (`linix prune`). Set true to fold pruning back into `sync`.
    #[serde(default = "default_false")]
    pub prune_on_sync: bool,

    #[serde(default = "default_false")]
    pub purge_orphans: bool,

    /// Drift-removal scope for `prune`/`sync`. `Managed` (default) only removes
    /// LiNix-managed packages; `System` removes anything installed that isn't in your
    /// manifests (except protected packages).
    #[serde(default)]
    pub prune_scope: PruneScope,

    /// When true, packages you installed imperatively (`linix install ...`) are never
    /// removed by drift pruning, even if they aren't in any manifest. Safe default: true.
    #[serde(default = "default_true")]
    pub protect_imperative: bool,

    /// Default SSH destinations for `linix fleet` when none are given on the command line.
    #[serde(default)]
    pub fleet_hosts: Vec<String>,

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

    /// Timeout (seconds) for outbound HTTP requests (registry/PyPI/marketplace search).
    #[serde(default = "default_network_timeout_secs")]
    pub network_timeout_secs: u64,

    /// Retention window passed to `nix-collect-garbage --delete-older-than` during
    /// orphan cleanup (e.g. "30d", "2w"). Replaces the previously hardcoded "30d".
    #[serde(default = "default_nix_gc_age")]
    pub nix_gc_age: String,

    /// When true, destructive operations (removals) require interactive confirmation
    /// unless `yes` is set. Extra guard on top of the normal preview.
    #[serde(default = "default_false")]
    pub confirm_destructive: bool,

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

fn default_groups_dir() -> PathBuf {
    safe_config_dir().join("groups")
}
fn default_modules_dir() -> PathBuf {
    safe_config_dir().join("modules")
}
fn default_bloatware_file() -> PathBuf {
    safe_config_dir().join("bloatware.txt")
}
fn default_btrfs_path() -> String {
    "/.snapshots".to_string()
}
fn default_timeshift_path() -> String {
    "/run/timeshift/backup/timeshift/snapshots".to_string()
}
fn default_tmp_dir() -> PathBuf {
    safe_data_dir().join("tmp")
}
fn default_github_dir() -> PathBuf {
    safe_data_dir().join("github")
}
fn default_web_dir() -> PathBuf {
    safe_data_dir().join("web")
}
fn default_appimage_dir() -> PathBuf {
    safe_data_dir().join("appimages")
}
fn default_true() -> bool {
    true
}
fn default_false() -> bool {
    false
}
fn default_cache_ttl() -> u64 {
    300
}
fn default_max_parallel() -> usize {
    4
}
fn default_network_timeout_secs() -> u64 {
    15
}
fn default_nix_gc_age() -> String {
    "30d".to_string()
}
fn default_max_age() -> u32 {
    30
}
fn default_max_count() -> u32 {
    10
}

fn default_priority() -> Vec<String> {
    vec![
        "apt".into(),
        "pacman".into(),
        "dnf".into(),
        "winget".into(),
        "brew".into(),
        "flatpak".into(),
        "snap".into(),
        "cargo".into(),
        "npm".into(),
        "pip".into(),
    ]
}

fn default_protected_packages() -> Vec<String> {
    let mut packages = vec!["sudo".into(), "bash".into(), "linix".into()];
    #[cfg(target_os = "linux")]
    {
        packages.extend(vec![
            "linux-image".into(),
            "linux-headers".into(),
            "kernel".into(),
            "systemd".into(),
            "libc6".into(),
            "libc".into(),
            "glibc".into(),
            "grub".into(),
            "grub2".into(),
            "coreutils".into(),
            "filesystem".into(),
            "apt".into(),
            "pacman".into(),
            "dnf".into(),
            "rpm".into(),
        ]);
    }
    #[cfg(target_os = "windows")]
    {
        packages.extend(vec![
            "windows".into(),
            "win32".into(),
            "kernel32".into(),
            "ntdll.dll".into(),
        ]);
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
            hostname_backends: HashMap::new(),
            managed_files: HashMap::new(),
            retention: crate::core::RetentionConfig::default(),
            bloatware_file: default_bloatware_file(),
            remove_bloatware: false,
            prune_on_sync: false,
            purge_orphans: false,
            prune_scope: PruneScope::default(),
            protect_imperative: true,
            fleet_hosts: Vec::new(),
            auto_lock_checksums: true,
            show_progress: true,
            verbose: false,
            cache_ttl: 300,
            github_token: None,
            max_parallel: 4,
            network_timeout_secs: default_network_timeout_secs(),
            nix_gc_age: default_nix_gc_age(),
            confirm_destructive: false,
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
        // Avoid TOCTOU: don't pre-check existence then read (the file could vanish in
        // between, turning a graceful default into a hard error). Read directly and treat
        // NotFound as "use defaults".
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(Error::Config(format!("Failed to read config file: {}", e))),
        };
        let mut config: Self = toml::from_str(&content)?;
        config.config_file = path.to_path_buf();
        if config.protected_packages.is_empty() {
            config.protected_packages = default_protected_packages();
        }
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("Failed to serialize config: {}", e)))?;
        if let Some(parent) = self.config_file.parent() {
            fs::create_dir_all(parent).map_err(Error::from)?;
        }
        fs::write(&self.config_file, content)
            .map_err(|e| Error::Config(format!("Failed to write config file: {}", e)))?;
        Ok(())
    }

    pub fn get_hostname() -> String {
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// The set of backends LiNix manages on the current host. A non-empty per-host
    /// override in `[hostname_backends]` wins; otherwise the global `enabled_backends`.
    /// An empty result means "all backends" — the default when nothing is configured.
    pub fn effective_enabled_backends(&self) -> Vec<String> {
        let host = Self::get_hostname();
        match self.hostname_backends.get(&host) {
            Some(list) if !list.is_empty() => list.clone(),
            _ => self.enabled_backends.clone(),
        }
    }

    /// Whether `backend` is managed on this host. An empty effective set enables every
    /// backend, preserving the zero-config default where nothing is filtered.
    pub fn is_backend_enabled(&self, backend: &str) -> bool {
        let effective = self.effective_enabled_backends();
        effective.is_empty() || effective.iter().any(|b| b == backend)
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
        if let Some(dr) = dry_run {
            self.dry_run = dr;
        }
        if let Some(y) = yes {
            self.yes = y;
        }
        if let Some(b) = backend {
            self.enabled_backends = vec![b];
        }
        if let Some(cp) = config_path {
            self.config_file = cp;
        }
        if let Some(gd) = groups_dir {
            self.groups_dir = gd;
        }
        if let Some(v) = verbose {
            self.verbose = v;
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.max_parallel == 0 {
            return Err(Error::Config("max_parallel must be greater than 0".into()));
        }
        // Verify cron strings for any schedules. Standard 5-field cron is normalized to
        // the `cron` crate's 6-field (with-seconds) form; `@`-macros are accepted as-is.
        for schedule in &self.schedules {
            if schedule.cron.starts_with('@') {
                continue;
            }
            let normalized = if schedule.cron.split_whitespace().count() == 5 {
                format!("0 {}", schedule.cron)
            } else {
                schedule.cron.clone()
            };
            if let Err(e) = normalized.parse::<cron::Schedule>() {
                return Err(Error::Config(format!(
                    "Invalid cron expression for task '{}': {}",
                    schedule.name, e
                )));
            }
        }
        Ok(())
    }

    /// True only on an EXACT (case-insensitive) match against a protected entry.
    /// Substring matching was a bug: protecting `libc`/`apt`/`kernel` also shielded
    /// `libc-bin`, `aptitude`, `kernelshark`, etc. from removal.
    pub fn is_protected(&self, package_name: &str) -> bool {
        let name_lower = package_name.to_lowercase();
        self.protected_packages
            .iter()
            .any(|p| p.to_lowercase() == name_lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule(cron: &str) -> ScheduleConfig {
        ScheduleConfig {
            name: "t".into(),
            cron: cron.into(),
            command: "sync".into(),
            notification: None,
            last_synced: None,
        }
    }

    #[test]
    fn backend_gating_defaults_to_all_enabled() {
        let cfg = Config::default();
        // Zero config → every backend is managed.
        assert!(cfg.is_backend_enabled("apt"));
        assert!(cfg.is_backend_enabled("cargo"));
        assert!(cfg.effective_enabled_backends().is_empty());
    }

    #[test]
    fn global_enabled_backends_restricts() {
        let cfg = Config {
            enabled_backends: vec!["apt".into(), "cargo".into()],
            ..Default::default()
        };
        assert!(cfg.is_backend_enabled("apt"));
        assert!(!cfg.is_backend_enabled("npm"));
    }

    #[test]
    fn per_host_override_wins_over_global() {
        // A per-host entry for THIS machine replaces the global list entirely.
        let mut hostname_backends = HashMap::new();
        hostname_backends.insert(Config::get_hostname(), vec!["cargo".into(), "npm".into()]);
        let cfg = Config {
            enabled_backends: vec!["apt".into()],
            hostname_backends,
            ..Default::default()
        };
        assert!(cfg.is_backend_enabled("cargo"));
        assert!(cfg.is_backend_enabled("npm"));
        // 'apt' was only in the global list, which the host override supersedes.
        assert!(!cfg.is_backend_enabled("apt"));
    }

    #[test]
    fn validate_accepts_standard_and_macro_cron() {
        // standard 5-field cron is accepted (normalized to the crate's 6-field form)
        let cfg = Config {
            schedules: vec![schedule("30 4 * * 1")],
            ..Config::default()
        };
        assert!(cfg.validate().is_ok(), "5-field cron should be valid");
        // explicit 6-field also accepted
        let cfg = Config {
            schedules: vec![schedule("0 30 4 * * 1")],
            ..Config::default()
        };
        assert!(cfg.validate().is_ok(), "6-field cron should be valid");
        // @-macros accepted
        let cfg = Config {
            schedules: vec![schedule("@daily")],
            ..Config::default()
        };
        assert!(cfg.validate().is_ok(), "@daily should be valid");
        // garbage rejected
        let cfg = Config {
            schedules: vec![schedule("not a cron")],
            ..Config::default()
        };
        assert!(cfg.validate().is_err(), "garbage cron should be rejected");
    }

    #[test]
    fn is_protected_is_exact_not_substring() {
        let cfg = Config {
            protected_packages: vec!["libc".into(), "apt".into(), "kernel".into()],
            ..Config::default()
        };
        // exact matches (case-insensitive) are protected
        assert!(cfg.is_protected("libc"));
        assert!(cfg.is_protected("APT"));
        // substrings/superstrings are NOT protected (the old bug)
        assert!(!cfg.is_protected("libc-bin"));
        assert!(!cfg.is_protected("aptitude"));
        assert!(!cfg.is_protected("kernelshark"));
    }
}
