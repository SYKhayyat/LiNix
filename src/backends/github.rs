use crate::core::{CommandExecutor, Package, PackageManager, Result, Error, PackageSpec, security::verify_checksum};
use crate::utils::archive::extract_archive;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::Mutex;
use tracing::{info, warn, debug};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GithubState {
    repo: String,
    version: String,
    bin_path: Option<String>,
    install_path: String,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

pub struct GithubManager {
    executor: CommandExecutor,
    client: reqwest::Client,
    install_dir: PathBuf,
    state_file: PathBuf,
    internal_lock: Mutex<()>,
}

impl GithubManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
let base = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
Self {
            executor,
            client: reqwest::Client::new(),
            install_dir: base.join("github"),
            state_file: base.join("github_installed.json"),
            internal_lock: Mutex::new(()),
        }
    }

    /// REAL LOGIC: Handles GitHub API Rate Limits by pausing instead of crashing.
    async fn github_get(&self, url: &str) -> Result<reqwest::Response> {
        loop {
            let res = self.client.get(url)
                .header("User-Agent", "linix-manager")
                .send().await?;
            
            if res.status() == reqwest::StatusCode::FORBIDDEN {
                if let Some(reset) = res.headers().get("x-ratelimit-reset") {
                    let reset_time = reset.to_str().unwrap_or("0").parse::<u64>().unwrap_or(0);
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                    if reset_time > now {
                        let sleep_secs = reset_time - now + 2;
                        warn!("GitHub Rate Limit hit. Waiting {}s...", sleep_secs);
                        tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
                        continue;
                    }
                }
            }
            return Ok(res);
        }
    }

    /// REAL LOGIC: Scores assets to find the best match for current OS and Architecture
    fn score_asset(&self, name: &str) -> i32 {
        let name = name.to_lowercase();
        let mut score = 0;
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        // OS matching
        if name.contains(os) { score += 50; }
        else if os == "linux" && name.contains("linux") { score += 40; }
        else if os == "macos" && (name.contains("darwin") || name.contains("apple")) { score += 40; }

        // Architecture matching
        if name.contains(arch) { score += 50; }
        else if arch == "x86_64" && (name.contains("amd64") || name.contains("x64")) { score += 45; }
        else if arch == "aarch64" && (name.contains("arm64") || name.contains("armv8")) { score += 45; }

        // Extension preference
        if name.ends_with(".tar.gz") || name.ends_with(".zip") || name.ends_with(".tgz") { score += 10; }
        if name.contains("musl") && os == "linux" { score += 5; } // Prefer static binaries on Linux

        // Penalty for debug or source symbols
        if name.contains("src") || name.contains("dev") || name.contains("dbg") { score -= 100; }
        
        score
    }
}

#[async_trait]
impl PackageManager for GithubManager {
    fn name(&self) -> &str { "github" }
    fn is_available(&self) -> bool { true }

    async fn install_with_options(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        let mut state: HashMap<String, GithubState> = if self.state_file.exists() {
            serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?
        } else { HashMap::new() };

        for spec in specs {
            let repo_full = spec.name.trim_start_matches("github:").trim_start_matches("https://github.com/");
            let url = format!("https://api.github.com/repos/{}/releases/latest", repo_full);
            
            let res = self.github_get(&url).await?;
            let release: GithubRelease = res.json().await?;
            
            // Find the best asset based on the scoring algorithm
            let asset = release.assets.iter()
                .max_by_key(|a| self.score_asset(&a.name))
                .ok_or_else(|| Error::Other(format!("No compatible asset found for {}", spec.name)))?;

            if let Some(existing) = state.get(&spec.name) {
                if existing.version == release.tag_name { continue; }
            }

            info!("GitHub downloading {} {}...", spec.name, release.tag_name);
            let bytes = self.github_get(&asset.browser_download_url).await?.bytes().await?;
            let tmp = tempfile::tempdir()?;
            let download_path = tmp.path().join(&asset.name);
            tokio::fs::write(&download_path, bytes).await?;

            if let Some(h) = spec.options.get("sha256") { verify_checksum(&download_path, h)?; }

            let pkg_dir = self.install_dir.join(repo_full.replace('/', "_"));
            let _ = tokio::fs::remove_dir_all(&pkg_dir).await;
            extract_archive(&download_path, &pkg_dir)?;

            // Symlink logic
            let repo_name = repo_full.split('/').last().unwrap();
            let bin_dest = dirs::home_dir().unwrap().join(".local/bin").join(repo_name);
            
            // Search extracted files for the executable
            let mut entries = tokio::fs::read_dir(&pkg_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let fname = entry.file_name().to_string_lossy().to_lowercase();
                if fname == repo_name.to_lowercase() || (fname.starts_with(repo_name) && !fname.contains('.')) {
                    #[cfg(unix)] {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o755));
                        let _ = tokio::fs::remove_file(&bin_dest).await;
                        let _ = tokio::fs::create_dir_all(bin_dest.parent().unwrap()).await;
                        std::os::unix::fs::symlink(entry.path(), &bin_dest)?;
                    }
                    break;
                }
            }

            state.insert(spec.name.clone(), GithubState {
                repo: repo_full.to_string(), version: release.tag_name,
                bin_path: Some(bin_dest.to_string_lossy().to_string()),
                install_path: pkg_dir.to_string_lossy().to_string(),
            });
        }
        tokio::fs::write(&self.state_file, serde_json::to_string_pretty(&state)?).await?;
        Ok(())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let specs: Vec<_> = p.iter().map(|n| PackageSpec { name: n.clone(), backend: "github".into(), options: HashMap::new() }).collect();
        self.install_with_options(&specs, s).await
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        if !self.state_file.exists() { return Ok(()); }
        let mut state: HashMap<String, GithubState> = serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?;
        for name in p {
            if let Some(pkg) = state.remove(name) {
                if let Some(bp) = pkg.bin_path { let _ = tokio::fs::remove_file(bp).await; }
                let _ = tokio::fs::remove_dir_all(pkg.install_path).await;
            }
        }
        tokio::fs::write(&self.state_file, serde_json::to_string_pretty(&state)?).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        if !self.state_file.exists() { return Ok(vec![]); }
        let state: HashMap<String, GithubState> = serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?;
        Ok(state.into_iter().map(|(n, s)| Package { 
            name: n, version: Some(s.version), backend: "github".into(), ..Package::new("", "") 
        }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let url = format!("https://api.github.com/search/repositories?q={}&per_page=15", query);
        let res = self.github_get(&url).await?;
        let json: serde_json::Value = res.json().await?;
        
        if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
            return Ok(items.iter().filter_map(|i| {
                let name = i.get("full_name")?.as_str()?;
                let mut p = Package::new(name, "github");
                p.description = i.get("description").and_then(|s| s.as_str()).map(|s| s.to_string());
                Some(p)
            }).collect());
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let url = format!("https://api.github.com/repos/{}", package.trim_start_matches("github:"));
        let res = self.github_get(&url).await?;
        if !res.status().is_success() { return Ok(None); }
        
        let json: serde_json::Value = res.json().await?;
        Ok(Some(Package {
            name: package.to_string(),
            description: json.get("description").and_then(|s| s.as_str()).map(|s| s.to_string()),
            repository: json.get("html_url").and_then(|s| s.as_str()).map(|s| s.to_string()),
            backend: "github".into(),
            ..Package::new("", "")
        }))
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        let installed = self.list_installed().await?;
        let names: Vec<String> = installed.into_iter().map(|p| p.name).collect();
        self.install(&names, s).await
    }
}