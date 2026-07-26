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

/// The `[guard]` table (II.10): the one home for all ten refusals. The v7 spec split them —
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
    /// Refuse an `@bin=` value that names a file outside the backend's bin directory (SEC1).
    /// On by default: off restores an unchecked join, where `@bin=../../.bashrc` is a symlink
    /// over your shell profile pointing at a downloaded file.
    #[serde(default = "default_confine_bin")]
    pub confine_bin: bool,
    /// Refuse to roll back to a commit git does not vouch for (II.13). Off by default: a fresh
    /// repo signs nothing, and a refusal that fires on every rollback out of the box would be
    /// turned off before it ever caught anything.
    #[serde(default)]
    pub require_signed_history: bool,
    /// Commands a `schedules` entry may not run (K13). Matched against the first word of a
    /// `run =` line, so `run = sync --locked` is unaffected by `sync` never being listed.
    /// Taking a name out is how a machine permits that command unattended; the shipped pair
    /// is the set that removes software without a human present to read the refusal.
    #[serde(default = "default_never_unattended")]
    pub never_unattended: Vec<String>,
}

/// `rebuild` and `purge-unmanaged`: the two commands that remove declared software. Unattended,
/// a failed rebuild leaves a machine missing software at 2am with nobody watching, and a purge
/// answers a question — "is this machine adopted?" — that only a human can have asked.
fn default_never_unattended() -> Vec<String> {
    vec!["rebuild".to_string(), "purge-unmanaged".to_string()]
}

fn default_confine_bin() -> bool {
    true
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
            confine_bin: default_confine_bin(),
            require_signed_history: false,
            never_unattended: default_never_unattended(),
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

/// The `[remove]` table (II.11c): what a removal means.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RemoveSettings {
    /// Also destroy the package's configuration in `/etc` (Debian's `purge`). A deleted
    /// module line means "stop installing this", which is not "destroy how I had it set up",
    /// so this is off unless the machine's owner turns it on.
    #[serde(default)]
    pub purge: bool,
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

/// Refusals and behaviour: `<config_root>/preferences.toml` (II.1).
///
/// `deny_unknown_fields`: a key that no longer exists must fail loudly. Silently ignoring one
/// means a `[guard]` setting can be deleted from the code and every config still claiming it
/// keeps parsing — the guard reads as configured while being off.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub aliases: HashMap<String, String>,

    /// User-defined CLI command shorthands, e.g. `up = "upgrade --all"`. Expanded before the
    /// command line is parsed. Distinct from `aliases` (which renames backends). An alias that
    /// shadows a built-in subcommand is ignored, so shorthands can never mask a real command.
    #[serde(default)]
    pub command_aliases: HashMap<String, String>,

    /// User-defined verbs (U35): a name that runs a *sequence* of built-in verbs, e.g.
    /// `refresh = ["sync", "upgrade --all"]`. Where a `command_alias` renames one command, a
    /// verb composes several — the `defun` over the command surface (XIII.31). **Composition
    /// only:** every step must name a built-in subcommand; a step that names anything else is
    /// refused, because a verb that could run arbitrary argv is `exec:` wearing a command's
    /// clothes (U33's territory, off by default). A verb never shadows a built-in.
    #[serde(default)]
    pub verbs: HashMap<String, Vec<String>>,

    #[serde(default)]
    pub dry_run: bool,

    #[serde(default)]
    pub yes: bool,

    /// This run is an unattended `watch` tick, so nobody is present to answer a prompt (T4).
    /// CLI/runtime only (`serde(skip)`): it is a property of *how LiNix was invoked*, not a
    /// preference. `watch` sets it; every other command leaves it false. A touch-required
    /// `@decrypt` is skipped under it rather than hanging the whole reconcile.
    #[serde(skip)]
    pub unattended: bool,

    /// Carry out a removal the guard would refuse (over `max_removals`, or touching a
    /// protected/essential package). CLI-only by design — `serde(skip)` keeps it out of
    /// preferences.toml, because a permanently-on "yes, purge anything" switch is exactly the
    /// setting this guard exists to make impossible. Deliberately distinct from `yes`:
    /// scripts and CI pass `-y` universally, and an unattended run is the one that cannot
    /// notice a system being dismantled.
    #[serde(skip)]
    pub allow_mass_removal: bool,

    /// Replace what a `dotfiles:` tree would overwrite instead of refusing (U23). CLI-only,
    /// for the same reason as `allow_mass_removal`: the refusal exists so a home directory
    /// full of distribution defaults is not silently replaced, and a machine that always
    /// bypasses it is a machine where the check does not exist.
    #[serde(skip)]
    pub replace_existing: bool,

    /// The root of your LiNix repo (II.1): the folder that holds `modules/`, `profiles/`,
    /// `active`, `priority`, `locks/` and `preferences.toml`. LiNix's own data (the registry,
    /// snapshots) lives BESIDE it, never inside it — see [`safe_data_dir`].
    ///
    /// `#[serde(skip)]`: `preferences.toml` lives *inside* this directory, so a key here that
    /// moved it could only be read from the place it was moving away from. It is resolved
    /// before this file is opened — `--config-dir`, `$LINIX_CONFIG_DIR`, the settings file,
    /// the default — by [`crate::app::locate`].
    #[serde(skip, default = "default_config_root")]
    pub config_root: PathBuf,

    /// Where LiNix's own data lives (II.1): the registry, snapshots, journal — BESIDE the repo,
    /// never inside it. Derived from [`safe_data_dir`] by default (which honours `$LINIX_DATA_
    /// DIR`), but a stored field so a test harness can inject an isolated root ONCE, structurally,
    /// instead of every test remembering to set an env var (S11). `#[serde(skip)]`: it is not a
    /// config-file knob, only a runtime/derived path.
    #[serde(skip, default = "default_data_root")]
    pub data_root: PathBuf,

    /// Where this file was read from: `<config_root>/preferences.toml` (II.1).
    #[serde(skip)]
    pub preferences_file: PathBuf,

    #[serde(default)]
    pub hooks: HashMap<String, HashMap<String, String>>,

    /// This machine's hooks on LiNix's own events (XIII.13, U15): `[events] on_drift = "..."`.
    ///
    /// **A separate table from `[hooks]` on purpose.** `[hooks]` is the package-lifecycle table
    /// (`before_install`, `after_install`, …), run by the embedded Lua/Rhai interpreter. These
    /// are LiNix's own events (`after_sync`, `on_drift`, `on_guard_refusal`), run as scripts
    /// with the event on stdin as JSON. They overlapped on `after_sync` when both read
    /// `[hooks]` — a hook there fired twice, once each way — so the event table has its own
    /// name and the two can never collide. The repo half lives in the `hooks/` directory.
    #[serde(default)]
    pub events: HashMap<String, String>,

    /// Machine-wide health checks (XIII.5, U7): `health = ["port:22", "systemctl is-system-running"]`.
    ///
    /// The half a package cannot see — the boot, the network, the thing two packages away.
    /// `@health=` on a line answers *did this upgrade break this*; these answer *is the machine
    /// still working*. Not alternatives: both run after a change, and both revert through the
    /// same path.
    #[serde(default)]
    pub health: Vec<String>,

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

    /// Suppress non-essential output (planned changes, transaction summary). Errors still print.
    #[serde(default)]
    pub quiet: bool,

    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,

    /// Timeout (seconds) for outbound HTTP requests (registry/PyPI/marketplace search).
    #[serde(default = "default_network_timeout_secs")]
    pub network_timeout_secs: u64,

    /// How long to wait out a remote rate limit before giving up and naming it (S26). A CI
    /// job and a laptop want different answers, which is why it is a key; the default is
    /// short because the wait happens while the data lock is held, and a command that looks
    /// hung is the one people kill — which is the interruption that leaves work to recover.
    #[serde(default = "default_rate_limit_max_wait_secs")]
    pub rate_limit_max_wait_secs: u64,

    /// Retention window passed to `nix-collect-garbage --delete-older-than` during
    /// orphan cleanup (e.g. "30d", "2w"). Replaces the previously hardcoded "30d".
    #[serde(default = "default_nix_gc_age")]
    pub nix_gc_age: String,

    /// K4: when a package installed by a *download* backend (`github:`/`web:`/`appimage:`) is
    /// removed, also delete any cached copy of the fetched file from the cache locations LiNix
    /// knows. Off by default. **Download-backends only, and the key says so:** LiNix knows the
    /// file only where it fetched it itself — on apt/dnf/pacman the manager owns its own cache
    /// and this setting does nothing, which is why it is scoped rather than pretending to be
    /// universal.
    #[serde(default)]
    pub clean_cache_on_remove: bool,

    /// K4: extra directories to search when `clean_cache_on_remove` cleans up — anywhere else a
    /// machine keeps downloads. LiNix already searches the standard locations
    /// (`$XDG_CACHE_HOME`, `~/.cache`, `/var/cache`); this points it at the rest.
    #[serde(default)]
    pub cache_dirs: Vec<std::path::PathBuf>,

    #[serde(default)]
    pub backend_settings: HashMap<String, HashMap<String, String>>,

    /// Carry out an install the guard would refuse for being over `max_installs`. CLI-only
    /// (`--allow-mass-install`), and — like [`allow_mass_removal`] — deliberately kept out
    /// of the config file: a permanently-on "install anything" switch defeats the ceiling.
    #[serde(skip)]
    pub allow_mass_install: bool,

    /// Allow `generate:` — a command whose stdout is treated as declarations (U33). **Off by
    /// default**: this is the one surface where the config computes its state instead of stating
    /// it, so it stays dormant unless deliberately enabled. Even on, the output passes the guard,
    /// the removal preview and the II.12 ledger, and a generator that fails is a failed sync.
    #[serde(default)]
    pub allow_generators: bool,

    /// The `[guard]` table (II.10): all ten refusals — protection, the removal/install
    /// count ceilings, and the install/change rules. See [`GuardSettings`].
    #[serde(default)]
    pub guard: GuardSettings,

    /// The `[remove]` table (II.11c). `purge = true` makes every removal on this machine also
    /// destroy the package's configuration. Off by default and machine-wide by construction:
    /// a removal happens after the line that would have carried a per-package option is gone,
    /// so the only place the choice can live is the machine, and a machine-wide destructive
    /// default nobody typed is exactly what must not happen.
    #[serde(default)]
    pub remove: RemoveSettings,

    /// A `uninstall --purge` for this run only. Never serialized — the file form is
    /// `[remove] purge`, and this is its per-invocation sibling.
    #[serde(skip)]
    pub purge_this_run: bool,

    #[serde(default = "default_btrfs_path")]
    pub btrfs_path: String,

    pub zfs_dataset: Option<String>,

    /// The order snapshot providers are chosen in (U28), the `priority`-file shape applied to the
    /// safety net: the first *available* provider in this list wins, built-in or config-declared.
    /// Empty means "the first available in registration order" — built-ins first, then the config
    /// providers from `adapters/snapshot.toml`, which is the pre-U28 behaviour unchanged.
    #[serde(default)]
    pub snapshot_priority: Vec<String>,

    #[serde(default = "default_tmp_dir")]
    pub tmp_dir: PathBuf,

    #[serde(default = "default_github_dir")]
    pub github_dir: PathBuf,

    #[serde(default = "default_web_dir")]
    pub web_dir: PathBuf,

    #[serde(default = "default_appimage_dir")]
    pub appimage_dir: PathBuf,

    /// Where shims are deployed. `~/.local/bin` on every platform, and **not a preference**:
    /// it is skipped by serde so a repo cannot move LiNix's shims onto a machine's PATH by
    /// declaration. It is a field rather than a constant so a sandbox can move it, which is
    /// what stops a test writing an executable into the developer's real `~/.local/bin`.
    #[serde(skip, default = "default_bin_dir")]
    pub bin_dir: PathBuf,

    #[serde(default)]
    pub sandbox: SandboxSettings,

    /// The `[vars]` table (Part IX): which variable provider is active. A repo may hold more
    /// than one provider file (`vars`, `vars.py`, `vars.linix`); this picks the active one.
    #[serde(default)]
    pub vars: VarsSettings,

}

/// The `[vars]` table (Part IX): selecting a variable provider when the repo holds several.
///
/// Providers are chosen by filename, and several may coexist — but exactly one is active per
/// machine. With `source` unset and more than one present, resolution refuses rather than
/// guessing which one wins.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VarsSettings {
    /// The filename of the active provider, relative to the repo root — `vars`, `vars.py`,
    /// `vars.linix`. Unset selects the sole provider file if there is exactly one, or none.
    #[serde(default)]
    pub source: Option<String>,
}

/// The one spelling of the refusals-and-behaviour file (II.1). [`Layout::preferences_file`]
/// joins it to the repo root; nothing else may name it.
pub const PREFERENCES_FILE_NAME: &str = "preferences.toml";

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
/// `~/.local/bin` — the directory a user's PATH already has, shared with every other tool,
/// which is why a shim is never removed by name alone (S1/S4). Falls back to the data root
/// when there is no home directory, so a shim never lands in the process's working directory.
fn default_bin_dir() -> PathBuf {
    match dirs::home_dir() {
        Some(home) => home.join(".local").join("bin"),
        None => safe_data_dir().join("bin"),
    }
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
/// 30 seconds: long enough to ride out GitHub's secondary-limit backoffs, short enough that
/// a user watching a held data lock does not conclude the process has hung. The old behaviour
/// was to sleep until the primary limit reset — up to an hour.
fn default_rate_limit_max_wait_secs() -> u64 {
    30
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
            verbs: HashMap::new(),
            dry_run: false,
            unattended: false,
            yes: false,
            allow_mass_removal: false,
            replace_existing: false,
            config_root: default_config_root(),
            data_root: default_data_root(),
            preferences_file: default_config_root().join(PREFERENCES_FILE_NAME),
            hooks: HashMap::new(),
            events: HashMap::new(),
            health: Vec::new(),
            retention: crate::core::RetentionConfig::default(),
            fleet_hosts: Vec::new(),
            show_progress: true,
            verbose: false,
            quiet: false,
            max_parallel: default_max_parallel(),
            network_timeout_secs: default_network_timeout_secs(),
            rate_limit_max_wait_secs: default_rate_limit_max_wait_secs(),
            nix_gc_age: default_nix_gc_age(),
            clean_cache_on_remove: false,
            cache_dirs: Vec::new(),
            backend_settings: HashMap::new(),
            allow_mass_install: false,
            guard: GuardSettings::default(),
            remove: RemoveSettings::default(),
            purge_this_run: false,
            btrfs_path: default_btrfs_path(),
            zfs_dataset: None,
            snapshot_priority: Vec::new(),
            allow_generators: false,
            tmp_dir: default_tmp_dir(),
            github_dir: default_github_dir(),
            web_dir: default_web_dir(),
            appimage_dir: default_appimage_dir(),
            bin_dir: default_bin_dir(),
            sandbox: SandboxSettings::default(),
            vars: VarsSettings::default(),
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
            // A missing file still fixes WHERE it was missing from: `config init` and
            // `config show` report this path, and a default that forgot it would name the
            // built-in location while `--config-dir` pointed somewhere else entirely.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    preferences_file: path.to_path_buf(),
                    ..Self::default()
                })
            }
            Err(e) => return Err(Error::Config(format!("Failed to read config file: {}", e))),
        };
        // Named, because this refusal stops every command LiNix has and the bare TOML error
        // says only "line 17" — of which of several files, it does not say. A key deleted in
        // the rewrite (NO LEGACY) is still on disk in configs written by an older build, so
        // this is the first thing a returning user meets.
        let mut config: Self = toml::from_str(&content).map_err(|e| {
            Error::Config(format!(
                "{} is not readable:\n{}\nDelete the line it names — the key no longer \
                 exists, and LiNix refuses a setting it would otherwise silently ignore.",
                path.display(),
                e
            ))
        })?;
        config.preferences_file = path.to_path_buf();
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
        if let Some(parent) = self.preferences_file.parent() {
            fs::create_dir_all(parent).map_err(Error::from)?;
        }
        fs::write(&self.preferences_file, content)
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

    /// A config whose every path — config root, data root, and each derived artifact dir —
    /// lives under `sandbox`.
    ///
    /// S11: hermeticity has to be structural, not remembered. `Config::default()` fills the
    /// data paths from `safe_data_dir()`, so a fixture that sets `config_root` and forgets
    /// `data_root` writes the registry, journal and snapshots into the developer's real
    /// state — which is what happened, silently, for as long as the journal existed. Every
    /// test fixture goes through here so there is one place to forget, and it does not.
    pub fn sandboxed(sandbox: &std::path::Path) -> Self {
        Self {
            config_root: sandbox.to_path_buf(),
            data_root: sandbox.to_path_buf(),
            preferences_file: sandbox.join(PREFERENCES_FILE_NAME),
            tmp_dir: sandbox.join("tmp"),
            github_dir: sandbox.join("github"),
            web_dir: sandbox.join("web"),
            appimage_dir: sandbox.join("appimages"),
            bin_dir: sandbox.join("bin"),
            ..Self::default()
        }
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
            self.preferences_file = cp;
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
    fn preferences_cannot_move_the_repo_it_lives_in() {
        // The file is at <config_root>/preferences.toml, so a `config_root` key here could
        // only ever be read from the directory it was trying to move away from. It is
        // resolved before this file is opened; serde must not quietly accept it back.
        // A fresh directory that deletes itself. A fixed name under `temp_dir()` is one
        // directory shared by every concurrent run of this suite, and the cleanup at the end
        // then deletes a directory another run is still using.
        let dir = tempfile::tempdir().expect("a temp dir");
        let dir = dir.path();
        let file = dir.join(PREFERENCES_FILE_NAME);
        std::fs::write(&file, "config_root = \"/somewhere/else\"\n").unwrap();

        let err = Config::from_file(&file)
            .expect_err("`config_root` is being read out of preferences.toml again");
        assert!(
            err.to_string().contains("config_root"),
            "the refusal must name the key: {}",
            err
        );

    }

    #[test]
    fn a_missing_file_still_remembers_where_it_looked() {
        // `--config-dir DIR config init` wrote to the built-in location instead of DIR,
        // because the not-found path returned a bare default.
        let target = std::env::temp_dir()
            .join("linix-absent-root")
            .join(PREFERENCES_FILE_NAME);
        let cfg = Config::from_file(&target).unwrap();
        assert_eq!(cfg.preferences_file, target);
    }

    #[test]
    fn the_shipped_example_is_a_file_linix_would_accept() {
        // It documented eight keys that had been deleted from this struct. Every one was
        // silently ignored, so the example read as a working config and was not one.
        let example = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/preferences.toml");
        let text = std::fs::read_to_string(example).expect("examples/preferences.toml");
        if let Err(e) = toml::from_str::<Config>(&text) {
            panic!("examples/preferences.toml documents a key that no longer exists: {e}");
        }
    }

    #[test]
    fn the_preferences_file_sits_inside_the_repo() {
        let sandbox = std::path::Path::new(if cfg!(windows) {
            r"C:\linix-test-sandbox"
        } else {
            "/tmp/linix-test-sandbox"
        });
        let cfg = Config::sandboxed(sandbox);
        assert_eq!(cfg.preferences_file, cfg.layout().preferences_file());
    }

    #[test]
    fn sandboxed_puts_every_path_under_the_sandbox() {
        // S11: the escape this exists to stop is a fixture that sets `config_root` and
        // forgets `data_root` -- the registry, journal and snapshots then land in the
        // developer's real state. Assert every path, not just the two obvious ones.
        let sandbox = std::path::Path::new(if cfg!(windows) {
            r"C:\linix-test-sandbox"
        } else {
            "/tmp/linix-test-sandbox"
        });
        let cfg = Config::sandboxed(sandbox);
        for (label, path) in [
            ("config_root", cfg.config_root()),
            ("data_root", cfg.data_root()),
            ("tmp_dir", cfg.tmp_dir.clone()),
            ("github_dir", cfg.github_dir.clone()),
            ("web_dir", cfg.web_dir.clone()),
            ("appimage_dir", cfg.appimage_dir.clone()),
            // `bin_dir` was the seventh, and it escaped: shims went to the real
            // `~/.local/bin` because the path was read from `dirs::home_dir()` at three call
            // sites instead of from here. A test run deployed an executable named `rg` onto
            // the developer's PATH.
            ("bin_dir", cfg.bin_dir.clone()),
        ] {
            assert!(
                path.starts_with(sandbox),
                "{} ({:?}) escaped the sandbox {:?}",
                label,
                path,
                sandbox
            );
        }
    }

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
