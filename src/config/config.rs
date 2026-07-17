use crate::core::{Error, Result};
use crate::model::Layout;
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

/// The `[guard]` table (II.10): the one home for all nine refusals. The v7 spec split them —
/// four removal rules lived as top-level `Config` fields and four install/change rules lived
/// in a separate `policy.toml` (II.17 deletes it) and a parallel `Policy` struct. Both are
/// now here. Every rule matches V.26's definition of a refusal ("I will not, and there is no
/// flag"), so `-y` cannot skip any of them. `allow_backends` is deliberately absent: the
/// `priority` file is what "only these backends" means now (V.15).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GuardSettings {
    // --- The removal rules (were top-level Config fields until the [guard] rename) ---
    /// Names removal must never touch. Matched exactly (case-insensitively), or as a prefix
    /// if the entry ends in `*` — `libpam*` covers `libpam0g`, while `libc` still does not
    /// cover `libc-bin`. User entries add to the built-in defaults.
    #[serde(default = "default_protected_packages")]
    pub protected_packages: Vec<String>,
    /// Names that are NOT protected even if a default (or an OS essential flag) says
    /// otherwise. Same matching rules, and it wins over both — the escape hatch for "I
    /// really do manage this one myself".
    #[serde(default)]
    pub unprotected_packages: Vec<String>,
    /// Refuse a plan that removes more than this many packages unless explicitly opted into.
    /// `0` disables the check.
    #[serde(default = "default_max_removals")]
    pub max_removals: usize,
    /// Refuse a plan that installs more than this many packages at once unless explicitly
    /// opted into. `0` (unset) disables it — installs are additive and far less dangerous.
    #[serde(default)]
    pub max_installs: usize,

    // --- The install/change rules (were policy.toml until the consolidation) ---
    /// Package names that may never be installed (matched case-insensitively).
    #[serde(default)]
    pub deny_packages: Vec<String>,
    /// Every desired package must carry an explicit `@version=` — no floating installs.
    #[serde(default)]
    pub pinned_only: bool,
    /// Refuse to change anything unless a snapshot can be taken first.
    #[serde(default)]
    pub require_snapshot: bool,
    /// Refuse to apply when `audit` reports a managed package as vulnerable.
    #[serde(default)]
    pub deny_vulnerable: bool,
}

impl Default for GuardSettings {
    fn default() -> Self {
        // The removal-safety defaults must survive the move off top-level Config — an empty
        // protected list or a zero max_removals here would silently disarm the guard, the
        // exact failure this project exists to prevent.
        Self {
            protected_packages: default_protected_packages(),
            unprotected_packages: Vec::new(),
            max_removals: default_max_removals(),
            max_installs: 0,
            deny_packages: Vec::new(),
            pinned_only: false,
            require_snapshot: false,
            deny_vulnerable: false,
        }
    }
}

impl GuardSettings {
    /// True when no *install/change* rule is active (so the pre-change gate is a no-op). The
    /// removal rules are deliberately excluded: `protected_packages` always has defaults, so
    /// including it would make this never-empty and run the install gate on every change.
    pub fn is_empty(&self) -> bool {
        self.deny_packages.is_empty()
            && !self.pinned_only
            && !self.require_snapshot
            && !self.deny_vulnerable
    }
}

/// Settings for automatic system snapshot management. Retention counts/ages moved to the one
/// retention engine (`[retention.snapshots]`); the legacy `max_age_days`/`max_count` keys were
/// deleted (NO LEGACY). Only the on/off switch remains here.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SnapshotSettings {
    /// If true, prune snapshots automatically after successful transactions.
    #[serde(default = "default_true")]
    pub auto_prune: bool,
}

impl Default for SnapshotSettings {
    fn default() -> Self {
        Self { auto_prune: true }
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

    /// User-defined CLI command shorthands, e.g. `up = "upgrade --all"`. Expanded before the
    /// command line is parsed. Distinct from `aliases` (which renames backends). An alias that
    /// shadows a built-in subcommand is ignored, so shorthands can never mask a real command.
    #[serde(default)]
    pub command_aliases: HashMap<String, String>,

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

    /// The root of your LiNix repo (II.1): the folder that holds `modules/`, `profiles/`,
    /// `active`, `priority`, `locks/` and `preferences.toml`. LiNix's own data (the registry,
    /// snapshots) lives BESIDE it, never inside it — see [`safe_data_dir`].
    #[serde(default = "default_config_root")]
    pub config_root: PathBuf,

    /// Where LiNix's own data lives (II.1): the registry, snapshots, journal — BESIDE the repo,
    /// never inside it. Derived from [`safe_data_dir`] by default (which honours `$LINIX_DATA_
    /// DIR`), but a stored field so a test harness can inject an isolated root ONCE, structurally,
    /// instead of every test remembering to set an env var (S11). `#[serde(skip)]`: it is not a
    /// config-file knob, only a runtime/derived path.
    #[serde(skip, default = "default_data_root")]
    pub data_root: PathBuf,

    #[serde(skip)]
    pub config_file: PathBuf,

    #[serde(default)]
    pub hooks: HashMap<String, HashMap<String, String>>,

    /// How long to retain each of LiNix's histories —
    /// generations, and filesystem snapshots — each configured independently. See
    /// [`crate::core::RetentionConfig`]. Empty/zero policies keep everything (default).
    #[serde(default)]
    pub retention: crate::core::RetentionConfig,

    /// Default SSH destinations for `linix fleet` when none are given on the command line.
    #[serde(default)]
    pub fleet_hosts: Vec<String>,

    #[serde(default = "default_true")]
    pub show_progress: bool,

    #[serde(default)]
    pub verbose: bool,

    /// Suppress non-essential output (flight plan, transaction summary). Errors still print.
    #[serde(default)]
    pub quiet: bool,

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

    /// Carry out an install the guard would refuse for being over `max_installs`. CLI-only
    /// (`--allow-mass-install`), and — like [`allow_mass_removal`] — deliberately kept out
    /// of the config file: a permanently-on "install anything" switch defeats the ceiling.
    #[serde(skip)]
    pub allow_mass_install: bool,

    /// The `[guard]` table (II.10): all nine refusals — protection, the removal/install
    /// count ceilings, and the install/change rules. See [`GuardSettings`].
    #[serde(default)]
    pub guard: GuardSettings,

    #[serde(default = "default_btrfs_path")]
    pub btrfs_path: String,

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

fn default_config_root() -> PathBuf {
    safe_config_dir()
}
fn default_data_root() -> PathBuf {
    safe_data_dir()
}
fn default_btrfs_path() -> String {
    "/.snapshots".to_string()
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
/// How many backend operations run at once by default (F1). Detect the machine's core
/// count rather than hardcoding 4 — a 32-core builder should not throttle itself to 4, and
/// a single-core VM should not fan out to 4. `available_parallelism` respects cgroup/CPU
/// affinity limits, so a container sees its quota, not the host's. Falls back to 4 if the
/// count is unavailable.
fn default_max_parallel() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}
fn default_network_timeout_secs() -> u64 {
    15
}
fn default_nix_gc_age() -> String {
    "30d".to_string()
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
            dry_run: false,
            yes: false,
            allow_mass_removal: false,
            config_root: default_config_root(),
            data_root: default_data_root(),
            config_file: safe_config_dir().join("config.toml"),
            hooks: HashMap::new(),
            retention: crate::core::RetentionConfig::default(),
            fleet_hosts: Vec::new(),
            show_progress: true,
            verbose: false,
            quiet: false,
            github_token: None,
            max_parallel: default_max_parallel(),
            network_timeout_secs: default_network_timeout_secs(),
            nix_gc_age: default_nix_gc_age(),
            confirm_destructive: false,
            backend_settings: HashMap::new(),
            allow_mass_install: false,
            guard: GuardSettings::default(),
            btrfs_path: default_btrfs_path(),
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
        // Empty protection means the built-in defaults, never "no protection" — a config
        // that omits (or empties) the list must not silently disarm the guard.
        if config.guard.protected_packages.is_empty() {
            config.guard.protected_packages = default_protected_packages();
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

    /// The repo root (II.1). NEVER resolves to the current working directory: a config that
    /// somehow carries an empty or relative `config_root` would make every `join` target
    /// whatever directory the user is standing in, so an unusable value falls back to the
    /// platform config dir.
    pub fn config_root(&self) -> PathBuf {
        if self.config_root.as_os_str().is_empty() || !self.config_root.is_absolute() {
            return safe_config_dir();
        }
        self.config_root.clone()
    }

    /// LiNix's data root (II.1) — where the registry, snapshots and journal live, beside the
    /// repo. Same empty/relative guard as [`config_root`], falling back to [`safe_data_dir`].
    /// One answer to "where is LiNix's data", whether it came from the platform dir,
    /// `$LINIX_DATA_DIR`, or a test's injected temp dir (P4/S11).
    pub fn data_root(&self) -> PathBuf {
        if self.data_root.as_os_str().is_empty() || !self.data_root.is_absolute() {
            return safe_data_dir();
        }
        self.data_root.clone()
    }

    /// This run's II.1 layout: your repo, and LiNix's data beside it but never inside it.
    ///
    /// Derived rather than stored so there is one answer to "where are the files", and it
    /// is the same answer whether it came from `$LINIX_CONFIG_DIR`, the platform dir, or a
    /// test's temporary directory (P4).
    pub fn layout(&self) -> Layout {
        Layout::new(self.config_root(), self.data_root())
    }

    pub fn merge_cli_overrides(
        &mut self,
        dry_run: Option<bool>,
        yes: Option<bool>,
        config_path: Option<PathBuf>,
        verbose: Option<bool>,
        allow_mass_removal: Option<bool>,
        allow_mass_install: Option<bool>,
    ) -> Result<()> {
        if let Some(dr) = dry_run {
            self.dry_run = dr;
        }
        if let Some(y) = yes {
            self.yes = y;
        }
        if let Some(a) = allow_mass_removal {
            self.allow_mass_removal = a;
        }
        if let Some(a) = allow_mass_install {
            self.allow_mass_install = a;
        }
        if let Some(cp) = config_path {
            self.config_file = cp;
        }
        if let Some(v) = verbose {
            self.verbose = v;
        }
        Ok(())
    }

    /// The snapshot-retention policy: `[retention.snapshots]`, the one config surface (the
    /// legacy `[snapshots]` count/age keys were deleted — NO LEGACY). Its default keeps the 10
    /// most-recent / 30 days, so out-of-the-box behaviour is unchanged; a user widens or
    /// narrows it by editing `[retention.snapshots]`.
    pub fn snapshot_retention(&self) -> crate::core::RetentionPolicy {
        self.retention.snapshots.clone()
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

    /// Case-insensitive. An entry matches exactly, or as a prefix if it ends in `*`
    /// (`libperl*`); it is never a substring match. Substring matching was a bug:
    /// protecting `libc`/`apt`/`kernel` also shielded `libc-bin`, `aptitude` and
    /// `kernelshark` from removal.
    pub fn is_protected(&self, package_name: &str) -> bool {
        self.protection_rule(package_name).is_some()
    }

    /// The `protected_packages` entry that protects `package_name`, or `None` if nothing
    /// does. Returning the rule rather than a bare bool lets a refusal say *why*.
    pub fn protection_rule(&self, package_name: &str) -> Option<&str> {
        let name_lower = package_name.to_lowercase();
        // An explicit unprotect entry always wins, including over a package the OS itself
        // flags as essential. Nothing overrides the user's stated intent.
        if Self::first_match(&self.guard.unprotected_packages, &name_lower).is_some() {
            return None;
        }
        Self::first_match(&self.guard.protected_packages, &name_lower)
    }

    /// The `unprotected_packages` entry exempting `package_name`, if any.
    pub fn unprotect_rule(&self, package_name: &str) -> Option<&str> {
        Self::first_match(&self.guard.unprotected_packages, &package_name.to_lowercase())
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

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_retention_reads_the_one_config_dialect() {
        // `[retention.snapshots]` is the only surface now (legacy `[snapshots]` count/age keys
        // deleted). The default preserves the old out-of-box behaviour: keep 10 / 30 days.
        let p = Config::default().snapshot_retention();
        assert_eq!((p.keep_last, p.keep_days), (10, 30), "default should keep 10 / 30 days");

        // And an explicit policy is read straight through.
        let mut cfg = Config::default();
        cfg.retention.snapshots = crate::core::RetentionPolicy {
            keep_last: 5,
            keep_days: 14,
            keep: Vec::new(),
        };
        let q = cfg.snapshot_retention();
        assert_eq!((q.keep_last, q.keep_days), (5, 14));
    }

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
            guard: GuardSettings {
                protected_packages: vec!["libc".into(), "apt".into(), "kernel".into()],
                ..Default::default()
            },
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
    fn a_trailing_star_protects_by_prefix() {
        // `is_protected` was documented as "EXACT only" while the code has always honoured
        // `*`, and the shipped defaults rely on it (`libperl*`). Believing the doc means
        // hand-expanding the wildcard and losing protection for whatever the list misses.
        let cfg = Config {
            guard: GuardSettings {
                protected_packages: vec!["libperl*".into(), "libc".into()],
                ..Default::default()
            },
            ..Config::default()
        };
        assert!(cfg.is_protected("libperl5.38t64"));
        assert!(cfg.is_protected("LIBPERL-BASE"));
        // A bare entry stays exact: the wildcard has to be asked for.
        assert!(!cfg.is_protected("libc-bin"));
    }
}
