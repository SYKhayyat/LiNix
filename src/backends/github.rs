// src/backends/github.rs
use crate::core::{CommandExecutor, Package, PackageManager, RateLimiter, Result, Error, PackageSpec};
use crate::utils::archive::extract_archive;
use crate::core::security::verify_checksum;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;
use tracing::info;

pub struct GithubManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    client: reqwest::Client,
    rate_limiter: RateLimiter,
    install_dir: PathBuf,
    state_file: PathBuf,
    settings: Option<HashMap<String, String>>,
    // FIX 4: In-process lock
    internal_lock: Mutex<()>, 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstalledPkg { repo: String, version: String, install_path: String, bin_path: String }
#[derive(Debug, Deserialize)]
struct GithubAsset { name: String, #[serde(rename = "browser_download_url")] url: String }
#[derive(Debug, Deserialize)]
struct GithubRelease { tag_name: String, assets: Vec<GithubAsset> }

impl GithubManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let base_dir = home.join(".local").join("share").join("linix");
        let token = settings.as_ref().and_then(|s| s.get("token")).cloned();
        Self {
            executor,
            available: OnceCell::new(),
            client: reqwest::Client::new(),
            rate_limiter: if token.is_some() { RateLimiter::github_authenticated() } else { RateLimiter::github() },
            install_dir: base_dir.join("github"),
            state_file: base_dir.join("github_installed.json"),
            settings,
            internal_lock: Mutex::new(()),
        }
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
        // FIX 4: Acquire lock to prevent concurrent write corruption
        let _guard = self.internal_lock.lock().await;

        let mut state: HashMap<String, InstalledPkg> = if self.state_file.exists() {
            serde_json::from_str(&fs::read_to_string(&self.state_file)?)?
        } else { HashMap::new() };

        for spec in specs {
            self.rate_limiter.wait().await?;
            let clean_name = spec.name.trim_start_matches("github:").trim_start_matches("https://github.com/");
            let repo_name = clean_name.split('/').nth(1).unwrap_or("");
            let url = format!("https://api.github.com/repos/{}/releases/latest", clean_name);
            
            let mut req = self.client.get(&url).header("User-Agent", "linix");
            if let Some(t) = self.settings.as_ref().and_then(|s| s.get("token")) {
                req = req.header("Authorization", format!("token {}", t));
            }
            
            let resp = req.send().await?;
            let release: GithubRelease = resp.json().await?;
            let asset = self.select_best_asset(&release.assets).ok_or_else(|| Error::Other("No asset".into()))?;

            info!("Installing {} version {}...", spec.name, release.tag_name);
            let tmp = tempfile::tempdir()?;
            let download_path = tmp.path().join(&asset.name);
            fs::write(&download_path, self.client.get(&asset.url).send().await?.bytes().await?)?;
            if let Some(hash) = spec.options.get("sha256") { verify_checksum(&download_path, hash)?; }

            let pkg_dir = self.install_dir.join(clean_name.replace('/', "_"));
            extract_archive(&download_path, &pkg_dir)?;

            let bin_dest = dirs::home_dir().unwrap().join(".local").join("bin").join(repo_name);
            let bin_src = pkg_dir.join(repo_name);

            #[cfg(unix)]
            {
                let _ = self.executor.run("chmod", &["+x", &bin_src.to_string_lossy()], false).await;
                let _ = self.executor.run("ln", &["-sf", &bin_src.to_string_lossy(), &bin_dest.to_string_lossy()], false).await;
            }
            #[cfg(windows)]
            {
                let _ = self.executor.run("cmd", &["/C", "copy", "/Y", &bin_src.to_string_lossy(), &bin_dest.to_string_lossy()], false).await;
            }

            state.insert(spec.name.clone(), InstalledPkg {
                repo: repo_name.to_string(), version: release.tag_name,
                bin_path: bin_dest.to_string_lossy().to_string(), install_path: pkg_dir.to_string_lossy().to_string(),
            });
        }
        let _ = fs::create_dir_all(self.state_file.parent().unwrap());
        fs::write(&self.state_file, serde_json::to_string_pretty(&state)?)?;
        Ok(())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let specs: Vec<_> = p.iter().map(|n| PackageSpec { name: n.clone(), backend: "github".into(), options: HashMap::new() }).collect();
        self.install_with_options(&specs, s).await
    }

    async fn remove(&self, p: &[String], _s: bool) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        if !self.state_file.exists() { return Ok(()); }
        let mut state: HashMap<String, InstalledPkg> = serde_json::from_str(&std::fs::read_to_string(&self.state_file)?)?;
        for name in p {
            if let Some(pkg) = state.remove(name) {
                let _ = self.executor.run(if cfg!(windows) { "del" } else { "rm" }, &[&pkg.bin_path], false).await;
                let _ = fs::remove_dir_all(Path::new(&pkg.install_path));
            }
        }
        fs::write(&self.state_file, serde_json::to_string_pretty(&state)?)?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        if !self.state_file.exists() { return Ok(vec![]); }
        let state: HashMap<String, InstalledPkg> = serde_json::from_str(&std::fs::read_to_string(&self.state_file)?)?;
        Ok(state.into_iter().map(|(n, i)| Package { name: n, version: Some(i.version), backend: "github".into(), ..Package::new("", "") }).collect())
    }
}