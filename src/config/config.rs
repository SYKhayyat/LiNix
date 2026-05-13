use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The primary configuration for LiNix Version 3.4.0 "Consistency & Integrity."
/// This struct acts as the central source of truth for execution parameters,
/// backend behavior, and system identity.
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

    /// Point 9: Whether to automatically prune unused dependencies (orphans) during sync.
    #[serde(default = "default_false")]
    pub purge_orphans: bool,

    /// Point 1: Whether to automatically lock checksums into manifests for web/github resources.
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
    
    /// Roadmap 2.3: Maximum number of parallel tasks in the JoinSet worker pool.
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
    
    /// Arbitrary key-value settings passed to individual backends (e.g., npm registry).
    #[serde(default)]
    pub backend_settings: HashMap<String, HashMap<String, String>>,
    
    /// The backend used if none is specified in a package string.
    #[serde(default)]
    pub default_backend: Option<String>,
}

fn default_groups_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("linix").join("groups")
    } else if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("linix").join("groups")
    } else {
        PathBuf::from("/etc/linix/groups")
    }
}

fn default_bloatware_file() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("linix").join("bloatware.txt")
    } else {
        PathBuf::from("/etc/linix/bloatware.txt")
    }
}

fn default_true() -> bool { true }
fn default_false() -> bool { false }
fn default_cache_ttl() -> u64 { 300 }
fn default_max_parallel() -> usize { 4 }

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
        }
    }
}

impl Config {
    /// Loads the configuration from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("Failed to read config file: {}", e)))?;
        let mut config: Self = toml::from_str(&content)?;
        config.config_file = path.to_path_buf();
        Ok(config)
    }

    /// Serializes and saves the configuration back to disk.
    pub fn to_file(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("Failed to serialize config: {}", e)))?;
        fs::write(path, content)
            .map_err(|e| Error::Config(format!("Failed to write config file: {}", e)))?;
        Ok(())
    }

    /// Returns the canonical hostname of the current machine.
    pub fn get_hostname() -> String {
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Merges CLI flags into the config object to override persistent settings.
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

    /// Validates the configuration for logical errors.
    pub fn validate(&self) -> Result<()> {
        if self.max_parallel == 0 {
            return Err(Error::Config("max_parallel must be greater than 0".into()));
        }
        Ok(())
    }
}