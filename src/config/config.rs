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

/// Which commands the removal guard is enforced on.
///
/// Every command that can delete a package is listed explicitly rather than implied, so
/// the whole surface is visible in one place — a guard nobody can enumerate is a guard
/// nobody can trust. All default to `true`; set one to `false` to opt that command out.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EnforceOn {
    /// `linix apply` — executing a saved plan.
    #[serde(default = "default_true")]
    pub apply: bool,
    /// `linix prune` — the dedicated drift-removal command.
    #[serde(default = "default_true")]
    pub prune: bool,
    /// `linix sync`, when `prune_on_sync` is enabled.
    #[serde(default = "default_true")]
    pub sync: bool,
    /// `linix watch` — reconciles unattended, so nobody is present to notice.
    #[serde(default = "default_true")]
    pub watch: bool,
    /// `linix upgrade`.
    #[serde(default = "default_true")]
    pub upgrade: bool,
    /// `linix rollback` — reverting to an earlier generation removes packages.
    #[serde(default = "default_true")]
    pub rollback: bool,
    /// `linix canary`.
    #[serde(default = "default_true")]
    pub canary: bool,
    /// `linix remove` — the direct, imperative uninstall.
    #[serde(default = "default_true")]
    pub remove: bool,
    /// Ghost-shell exit, which force-removes transient packages.
    /// Spelled `shell-exit` in config.toml to match how the command reads in prose and in
    /// `linix protected`; the underscore form is accepted too so neither spelling is a
    /// silently-ignored typo.
    #[serde(default = "default_true", rename = "shell-exit", alias = "shell_exit")]
    pub shell_exit: bool,
    /// Expired-lease sweeps, which run after every state-changing command.
    #[serde(default = "default_true")]
    pub leases: bool,
}

impl Default for EnforceOn {
    fn default() -> Self {
        Self {
            apply: true,
            prune: true,
            sync: true,
            watch: true,
            upgrade: true,
            rollback: true,
            canary: true,
            remove: true,
            shell_exit: true,
            leases: true,
        }
    }
}

/// Settings for the removal guard — the check that refuses to delete too much, or to
/// delete something the system needs.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GuardSettings {
    #[serde(default)]
    pub enforce_on: EnforceOn,
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

    /// User-defined CLI command shorthands, e.g. `up = "upgrade --all"`. Expanded before the
    /// command line is parsed. Distinct from `aliases` (which renames backends). An alias that
    /// shadows a built-in subcommand is ignored, so shorthands can never mask a real command.
    #[serde(default)]
    pub command_aliases: HashMap<String, String>,

    #[serde(default)]
    pub groups: HashMap<String, Vec<String>>,

    #[serde(default)]
    pub dry_run: bool,

    #[serde(default)]
    pub yes: bool,

    /// Carry out a removal the guard would refuse (over `max_removals`, or touching a
    /// protected/essential package). CLI-only by design — `serde(skip)` keeps it out of
    /// config.toml, because a permanently-on "yes, purge anything" switch is exactly the
    /// setting this guard exists to make impossible. Deliberately distinct from `yes`:
    /// scripts and CI pass `-y` universally, and an unattended run is the one that cannot
    /// notice a system being dismantled.
    #[serde(skip)]
    pub allow_mass_removal: bool,

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

    /// Suppress non-essential output (flight plan, transaction summary). Errors still print.
    #[serde(default)]
    pub quiet: bool,

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

    /// Names removal must never touch. An entry is matched exactly (case-insensitively),
    /// or as a prefix if it ends in `*` — `libpam*` covers `libpam0g`, while `libc` still
    /// does not cover `libc-bin`. User entries add to the built-in defaults.
    #[serde(default = "default_protected_packages")]
    pub protected_packages: Vec<String>,

    /// Names that are NOT protected even if a default (or an essential flag reported by
    /// the OS) says otherwise. Same matching rules as `protected_packages`, and it wins
    /// over both — the escape hatch for "I really do manage this one myself".
    #[serde(default)]
    pub unprotected_packages: Vec<String>,

    /// Refuse a plan that removes more than this many packages unless it is explicitly
    /// opted into. Guards against a mis-scoped manifest quietly purging a system. `0`
    /// disables the check.
    #[serde(default = "default_max_removals")]
    pub max_removals: usize,

    /// Which commands the removal guard is enforced on. See `EnforceOn`.
    #[serde(default)]
    pub guard: GuardSettings,

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

/// Refuse a plan removing more than this many packages without an explicit opt-in.
/// Twenty is comfortably above a routine cleanup and far below the ~100 that adopting a
/// stock Ubuntu's manual set produces.
fn default_max_removals() -> usize {
    20
}

fn default_protected_packages() -> Vec<String> {
    let mut packages = vec!["sudo".into(), "bash".into(), "linix".into()];
    #[cfg(target_os = "linux")]
    {
        // These are packages whose removal breaks the machine (or LiNix's own ability to
        // run and repair it), and which a manifest is unlikely to ever declare. Prefix
        // entries (`libpam*`) are deliberate: the library families ship under versioned
        // names (libpam0g, libperl5.38t64) no fixed list could keep up with.
        //
        // Note this list is not redundant with the OS's own essential flags: on Ubuntu,
        // `python3` and `libpam0g` are `Priority: optional` and NOT `Essential`, yet both
        // appear in `apt-mark showmanual` and purging either wrecks the system.
        packages.extend(vec![
            "linux-image".into(),
            "linux-headers".into(),
            "kernel".into(),
            "systemd".into(),
            "init".into(),
            "libc6".into(),
            "libc".into(),
            "libc-bin".into(),
            "glibc".into(),
            "grub".into(),
            "grub2".into(),
            "coreutils".into(),
            "util-linux".into(),
            "filesystem".into(),
            "dash".into(),
            "login".into(),
            "passwd".into(),
            "base-files".into(),
            "base-passwd".into(),
            "busybox".into(),
            "alpine-baselayout".into(),
            "apk-tools".into(),
            "apt".into(),
            "dpkg".into(),
            "pacman".into(),
            "dnf".into(),
            "yum".into(),
            "rpm".into(),
            "libpam*".into(),
            "openssl".into(),
            "ca-certificates".into(),
            "perl-base".into(),
            "libperl*".into(),
            // The interpreter itself, not the family: `python3*` would also cover
            // python3-pip, python3-dev and every apt-packaged python library, which
            // people legitimately manage. Removing `python3` breaks the machine;
            // removing `python3-requests` is a Tuesday.
            "python3".into(),
            "python3-minimal".into(),
            "libpython3*".into(),
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
            command_aliases: HashMap::new(),
            groups: HashMap::new(),
            dry_run: false,
            yes: false,
            allow_mass_removal: false,
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
            quiet: false,
            cache_ttl: 300,
            github_token: None,
            max_parallel: 4,
            network_timeout_secs: default_network_timeout_secs(),
            nix_gc_age: default_nix_gc_age(),
            confirm_destructive: false,
            backend_settings: HashMap::new(),
            default_backend: None,
            protected_packages: default_protected_packages(),
            unprotected_packages: Vec::new(),
            max_removals: default_max_removals(),
            guard: GuardSettings::default(),
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
        allow_mass_removal: Option<bool>,
    ) {
        if let Some(dr) = dry_run {
            self.dry_run = dr;
        }
        if let Some(y) = yes {
            self.yes = y;
        }
        if let Some(a) = allow_mass_removal {
            self.allow_mass_removal = a;
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
        self.protection_rule(package_name).is_some()
    }

    /// The `protected_packages` entry that protects `package_name`, or `None` if nothing
    /// does. Returning the rule rather than a bare bool lets a refusal say *why*.
    pub fn protection_rule(&self, package_name: &str) -> Option<&str> {
        let name_lower = package_name.to_lowercase();
        // An explicit unprotect entry always wins, including over a package the OS itself
        // flags as essential. Nothing overrides the user's stated intent.
        if Self::first_match(&self.unprotected_packages, &name_lower).is_some() {
            return None;
        }
        Self::first_match(&self.protected_packages, &name_lower)
    }

    /// The `unprotected_packages` entry exempting `package_name`, if any.
    pub fn unprotect_rule(&self, package_name: &str) -> Option<&str> {
        Self::first_match(&self.unprotected_packages, &package_name.to_lowercase())
    }

    /// The first pattern matching `name_lower`: exact (case-insensitive), or a prefix when
    /// the pattern ends in `*`. Bare entries stay exact so `libc` never silently swallows
    /// `libc-bin` — the wildcard has to be asked for.
    fn first_match<'a>(patterns: &'a [String], name_lower: &str) -> Option<&'a str> {
        patterns
            .iter()
            .find(|p| {
                let p = p.to_lowercase();
                match p.strip_suffix('*') {
                    Some(prefix) => name_lower.starts_with(prefix),
                    None => name_lower == p,
                }
            })
            .map(|s| s.as_str())
    }

    /// Path to the user-editable keep-list: a plain manifest (`keep.txt`, in the groups dir)
    /// of package names that drift removal must never touch — the file-based companion to
    /// `protected_packages`. It is separate from `local.txt` so "keep this if present" is
    /// never confused with "install this".
    pub fn keep_file_path(&self) -> PathBuf {
        self.groups_dir.join("keep.txt")
    }

    /// Merge the entries of `keep.txt` (one package name per line; `#` comments and blanks
    /// ignored) into `protected_packages`, de-duplicated. Idempotent. Call once after the
    /// groups dir is finalized so all consumers of `is_protected` honor the keep-list.
    pub fn merge_keep_file(&mut self) {
        let path = self.keep_file_path();
        let Ok(body) = std::fs::read_to_string(&path) else {
            return; // no keep file is fine
        };
        let existing: std::collections::HashSet<String> = self
            .protected_packages
            .iter()
            .map(|p| p.to_lowercase())
            .collect();
        let mut existing = existing;
        for line in body.lines() {
            let name = line.trim();
            if name.is_empty() || name.starts_with('#') {
                continue;
            }
            if existing.insert(name.to_lowercase()) {
                self.protected_packages.push(name.to_string());
            }
        }
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

    #[test]
    fn merge_keep_file_folds_entries_into_protected_set() {
        let dir = std::env::temp_dir().join(format!("linix-keeptest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let keep = dir.join("keep.txt");
        std::fs::write(&keep, "# my keeps\nsteam\nnvidia-driver\nsteam\n").unwrap();

        let mut cfg = Config {
            protected_packages: vec!["libc".into()],
            groups_dir: dir.clone(),
            ..Config::default()
        };
        cfg.merge_keep_file();

        assert!(cfg.is_protected("steam"));
        assert!(cfg.is_protected("NVIDIA-DRIVER")); // case-insensitive
        assert!(cfg.is_protected("libc")); // original preserved
        // Deduped: "steam" listed twice in the file appears once.
        let count = cfg
            .protected_packages
            .iter()
            .filter(|p| p.eq_ignore_ascii_case("steam"))
            .count();
        assert_eq!(count, 1);

        // Idempotent: a second merge adds nothing.
        let before = cfg.protected_packages.len();
        cfg.merge_keep_file();
        assert_eq!(cfg.protected_packages.len(), before);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_keep_file_is_noop_without_file() {
        let mut cfg = Config {
            groups_dir: std::env::temp_dir().join("linix-nonexistent-keepdir-xyz"),
            protected_packages: vec!["libc".into()],
            ..Config::default()
        };
        cfg.merge_keep_file();
        assert_eq!(cfg.protected_packages, vec!["libc".to_string()]);
    }
}
