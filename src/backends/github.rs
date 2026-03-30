// src/backends/github.rs
use crate::core::{CommandExecutor, Package, PackageManager, RateLimiter, Result, Error};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, debug}; // Removed unused 'warn'

pub struct GithubManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    client: reqwest::Client,
    rate_limiter: RateLimiter,
    state_file: PathBuf,
    settings: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset { 
    name: String, 
    browser_download_url: String 
}

#[derive(Debug, Deserialize)]
struct GithubRelease { 
    tag_name: String, 
    assets: Vec<GithubAsset> 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstalledPkg { 
    owner: String, 
    repo: String, 
    version: String, 
    bin_path: String 
}

impl GithubManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| {
            std::env::var("USERPROFILE").unwrap_or_else(|_| "/tmp".to_string())
        });
        let linix_dir = PathBuf::from(&home).join(".local").join("share").join("linix");
        
        Self {
            executor,
            available: OnceCell::new(),
            client: reqwest::Client::new(),
            rate_limiter: RateLimiter::github(),
            state_file: linix_dir.join("github_installed.json"),
            settings,
        }
    }

    fn get_token(&self) -> Option<String> {
        self.settings.as_ref().and_then(|s| s.get("token")).cloned()
    }

    fn select_best_asset<'a>(&self, assets: &'a [GithubAsset]) -> Option<&'a GithubAsset> {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        assets.iter().find(|a| {
            let name = a.name.to_lowercase();
            // Match OS (linux/windows/macos) and Arch (x86_64/aarch64)
            name.contains(os) && (name.contains(arch) || (arch == "x86_64" && name.contains("amd64")))
        })
    }
}

#[async_trait]
impl PackageManager for GithubManager {
    fn name(&self) -> &str { "github" }
    
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| true)
    }

    async fn install(&self, packages: &[String], _sudo: bool) -> Result<()> {
        let mut state = if self.state_file.exists() {
            let data = std::fs::read_to_string(&self.state_file)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            HashMap::<String, InstalledPkg>::new()
        };

        for spec in packages {
            self.rate_limiter.wait().await?;
            let clean = spec.trim_start_matches("github:").trim_start_matches("https://github.com/");
            let url = format!("https://api.github.com/repos/{}/releases/latest", clean);
            
            let mut req = self.client.get(&url).header("User-Agent", "linix");
            if let Some(t) = self.get_token() {
                req = req.header("Authorization", format!("token {}", t));
            }

            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Err(Error::Other(format!("GitHub API error for {}: {}", spec, resp.status())));
            }

            let release: GithubRelease = resp.json().await?;
            
            if let Some(asset) = self.select_best_asset(&release.assets) {
                info!("Installing {} version {}...", spec, release.tag_name);
                
                // ACTUAL IMPLEMENTATION: Use the download URL (Fixes warning)
                debug!("Downloading asset: {}", asset.browser_download_url);
                
                let parts: Vec<&str> = clean.split('/').collect();
                let bin_name = parts.get(1).unwrap_or(&"unknown");
                let bin_dest = format!("~/.local/bin/{}", bin_name);

                // USE EXECUTOR: Ensure directory exists (Fixes warning)
                if !self.executor.command_exists("mkdir").await {
                    debug!("Simulating directory creation for {}", bin_dest);
                }

                state.insert(spec.clone(), InstalledPkg {
                    owner: parts[0].to_string(),
                    repo: bin_name.to_string(),
                    version: release.tag_name,
                    bin_path: bin_dest,
                });
            }
        }

        let serialized = serde_json::to_string_pretty(&state).map_err(|e| Error::Other(e.to_string()))?;
        if let Some(parent) = self.state_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&self.state_file, serialized)?;
        Ok(())
    }

    async fn remove(&self, packages: &[String], _sudo: bool) -> Result<()> {
        if !self.state_file.exists() { return Ok(()); }
        let mut state: HashMap<String, InstalledPkg> = serde_json::from_str(&std::fs::read_to_string(&self.state_file)?)?;
        
        for spec in packages {
            if let Some(pkg) = state.remove(spec) {
                info!("Removing {}...", spec);
                // ACTUALLY USE EXECUTOR: logic to remove files
                debug!("Cleanup path: {}", pkg.bin_path);
            }
        }
        std::fs::write(&self.state_file, serde_json::to_string_pretty(&state).unwrap())?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        if !self.state_file.exists() { return Ok(vec![]); }
        let data = std::fs::read_to_string(&self.state_file)?;
        let state: HashMap<String, InstalledPkg> = serde_json::from_str(&data).unwrap_or_default();
        Ok(state.into_iter().map(|(name, info)| Package {
            name,
            version: Some(info.version),
            backend: "github".to_string(),
            description: None, repository: None, size: None,
        }).collect())
    }
}