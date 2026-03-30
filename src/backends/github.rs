// src/backends/github.rs
use crate::core::{CommandExecutor, Package, PackageManager, RateLimiter, Result, Error, PackageSpec};
use crate::utils::archive::extract_archive;
use crate::core::security::verify_checksum;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info; // Removed warn/debug if unused

pub struct GithubManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    client: reqwest::Client,
    rate_limiter: RateLimiter,
    install_dir: PathBuf,
    state_file: PathBuf,
    settings: Option<HashMap<String, String>>,
}
// ... [rest of implementation same as previous Turn] ...

#[derive(Debug, Deserialize)]
struct GithubAsset { name: String, browser_download_url: String }
#[derive(Debug, Deserialize)]
struct GithubRelease { tag_name: String, assets: Vec<GithubAsset> }
#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstalledPkg { owner: String, repo: String, version: String, bin_path: String }

impl GithubManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let base_dir = home.join(".local").join("share").join("linix");
        Self {
            executor,
            available: OnceCell::new(),
            client: reqwest::Client::new(),
            rate_limiter: RateLimiter::github(),
            install_dir: base_dir.join("github"),
            state_file: base_dir.join("github_installed.json"),
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
            let n = a.name.to_lowercase();
            n.contains(os) && (n.contains(arch) || (arch == "x86_64" && n.contains("amd64")))
        })
    }
}

#[async_trait]
impl PackageManager for GithubManager {
    fn name(&self) -> &str { "github" }
    fn is_available(&self) -> bool { *self.available.get_or_init(|| true) }

    async fn install_with_options(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        let mut state: HashMap<String, InstalledPkg> = if self.state_file.exists() {
            serde_json::from_str(&std::fs::read_to_string(&self.state_file)?)?
        } else { HashMap::new() };

        for spec in specs {
            self.rate_limiter.wait().await?;
            let clean_name = spec.name.trim_start_matches("github:").trim_start_matches("https://github.com/");
            let url = format!("https://api.github.com/repos/{}/releases/latest", clean_name);
            
            let mut req = self.client.get(&url).header("User-Agent", "linix");
            if let Some(t) = self.get_token() {
                req = req.header("Authorization", format!("token {}", t));
            }
            
            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Err(Error::Other(format!("GitHub API error for {}: {}", spec.name, resp.status())));
            }
            let release: GithubRelease = resp.json().await?;
            
            let asset = self.select_best_asset(&release.assets)
                .ok_or_else(|| Error::Other(format!("No compatible asset found for {} on {}/{}", spec.name, std::env::consts::OS, std::env::consts::ARCH)))?;

            info!("Installing {} version {}...", spec.name, release.tag_name);

            let tmp_dir = tempfile::tempdir()?;
            let download_path = tmp_dir.path().join(&asset.name);
            let bytes = self.client.get(&asset.browser_download_url).send().await?.bytes().await?;
            std::fs::write(&download_path, bytes)?;

            if let Some(expected_hash) = spec.options.get("sha256") {
                verify_checksum(&download_path, expected_hash)?;
            }

            let pkg_dir = self.install_dir.join(clean_name.replace('/', "_"));
            let _ = std::fs::create_dir_all(&pkg_dir);
            extract_archive(&download_path, &pkg_dir)?;

            let bin_name = clean_name.split('/').nth(1).unwrap_or("");
            let bin_dest = dirs::home_dir().unwrap_or_default().join(".local").join("bin").join(bin_name);
            
            // ACTUALLY USE EXECUTOR: Create symlink and set permissions
            let _ = self.executor.run("ln", &["-sf", &pkg_dir.join(bin_name).to_string_lossy(), &bin_dest.to_string_lossy()], false).await;
            let _ = self.executor.run("chmod", &["+x", &bin_dest.to_string_lossy()], false).await;

            state.insert(spec.name.clone(), InstalledPkg {
                owner: clean_name.split('/').next().unwrap_or("").to_string(),
                repo: bin_name.to_string(),
                version: release.tag_name,
                bin_path: pkg_dir.to_string_lossy().to_string(),
            });
        }
        std::fs::write(&self.state_file, serde_json::to_string_pretty(&state)?)?;
        Ok(())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let specs: Vec<_> = p.iter().map(|n| PackageSpec { name: n.clone(), backend: "github".into(), options: HashMap::new() }).collect();
        self.install_with_options(&specs, s).await
    }

    async fn remove(&self, packages: &[String], _sudo: bool) -> Result<()> {
        if !self.state_file.exists() { return Ok(()); }
        let mut state: HashMap<String, InstalledPkg> = serde_json::from_str(&std::fs::read_to_string(&self.state_file)?)?;
        for name in packages {
            if let Some(pkg) = state.remove(name) {
                let bin_dest = dirs::home_dir().unwrap_or_default().join(".local").join("bin").join(&pkg.repo);
                let _ = self.executor.run("rm", &[&bin_dest.to_string_lossy()], false).await;
                let _ = std::fs::remove_dir_all(Path::new(&pkg.bin_path));
                info!("Removed {}", name);
            }
        }
        std::fs::write(&self.state_file, serde_json::to_string_pretty(&state)?)?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        if !self.state_file.exists() { return Ok(vec![]); }
        let data = std::fs::read_to_string(&self.state_file)?;
        let state: HashMap<String, InstalledPkg> = serde_json::from_str(&data).unwrap_or_default();
        Ok(state.into_iter().map(|(n, i)| Package {
            name: n, version: Some(i.version), backend: "github".into(), ..Package::new("", "")
        }).collect())
    }
}