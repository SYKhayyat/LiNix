use crate::core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
	#[serde(default)]
    pub aliases: HashMap<String, String>, // e.g., "system" -> "apt"
    #[serde(default)]
    pub groups: HashMap<String, Vec<String>>, // e.g., "dev" -> ["git", "vim"]
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub yes: bool,
    #[serde(default = "default_groups_dir")]
    pub groups_dir: PathBuf,
    #[serde(skip)]
    pub config_file: PathBuf,
    #[serde(default)]
    pub enabled_backends: Vec<String>,
    #[serde(default)]
    pub hooks: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    pub hostname_packages: HashMap<String, Vec<String>>,
    #[serde(default = "default_bloatware_file")]
    pub bloatware_file: PathBuf,
    #[serde(default)]
    pub remove_bloatware: bool,
    #[serde(default = "default_true")]
    pub show_progress: bool,
    #[serde(default)]
    pub verbose: bool,
    #[serde(default)]
    pub windows_backends: Option<Vec<String>>,
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
}

fn default_groups_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("linix").join("groups")
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
fn default_cache_ttl() -> u64 { 300 }
fn default_max_parallel() -> usize { 4 }

// src/config/config.rs

// ... (existing imports like use std::collections::HashMap; should be at the top)

// src/config/config.rs
// ... existing imports ...

impl Default for Config {
    fn default() -> Self {
        Self {
            dry_run: false,
            yes: false,
            groups_dir: default_groups_dir(),
            config_file: PathBuf::from("/etc/linix/config.toml"),
            enabled_backends: Vec::new(),
            hooks: HashMap::new(),
            hostname_packages: HashMap::new(),
            bloatware_file: default_bloatware_file(),
            remove_bloatware: false,
            show_progress: true,
            verbose: false,
            windows_backends: None,
            cache_ttl: 300,
            github_token: None,
            max_parallel: 4,
            backend_settings: HashMap::new(),
            default_backend: None,
            aliases: HashMap::new(), // Added
            groups: HashMap::new(),  // Added
        }
    }
}

impl Config {
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

    pub fn to_file(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("Failed to serialize config: {}", e)))?;
        fs::write(path, content)
            .map_err(|e| Error::Config(format!("Failed to write config file: {}", e)))?;
        Ok(())
    }

    pub fn get_hostname() -> String {
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string())
    }

    pub fn get_hostname_packages(&self) -> Vec<String> {
        let hostname = Self::get_hostname();
        self.hostname_packages.get(&hostname).cloned().unwrap_or_default()
    }

    pub fn merge_cli_overrides(&mut self, dry_run: Option<bool>, yes: Option<bool>,
                               backend: Option<String>, config_path: Option<PathBuf>,
                               groups_dir: Option<PathBuf>, verbose: Option<bool>) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert!(!config.dry_run);
        assert_eq!(config.max_parallel, 4);
        assert!(config.default_backend.is_none());
    }

    #[test]
    fn test_config_from_toml() {
        let toml_content = r#"
            dry_run = true
            enabled_backends = ["apt", "flatpak"]
            max_parallel = 8
            default_backend = "apt"
            [hostname_packages]
            myhost = ["package1", "package2"]
        "#;
        let config: Config = toml::from_str(toml_content).unwrap();
        assert!(config.dry_run);
        assert_eq!(config.enabled_backends, vec!["apt", "flatpak"]);
        assert_eq!(config.max_parallel, 8);
        assert_eq!(config.default_backend, Some("apt".to_string()));
    }
}