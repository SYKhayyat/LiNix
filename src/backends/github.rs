use crate::core::{
    manager::{Backend, Installable, Queryable},
    security::verify_checksum,
    CommandExecutor, Error, Package, PackageSpec, Result,
};
use crate::utils::archive::extract_archive;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Internal state for a GitHub-managed package.
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
    #[serde(rename = "browser_download_url")]
    url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    #[serde(rename = "tag_name")]
    version: String,
    assets: Vec<GithubAsset>,
}

/// A specialized manager for installing binaries directly from GitHub Releases.
/// Implements Roadmap Phase 2.1 (LockMap compatibility) and Phase 3.2 (Deterministic state).
pub struct GithubManager {
    executor: CommandExecutor,
    client: reqwest::Client,
    install_dir: PathBuf,
    state_file: PathBuf,
    /// Internal lock to prevent concurrent modification of the local github registry file.
    internal_lock: Mutex<()>,
}

impl GithubManager {
    pub fn new(executor: CommandExecutor) -> Self {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("github");
        
        Self {
            executor,
            client: reqwest::Client::new(),
            install_dir: base.clone(),
            state_file: base.join("installed.json"),
            internal_lock: Mutex::new(()),
        }
    }

    /// Helper to identify GitHub package strings.
    pub fn parse_github_url(spec: &str) -> Option<PackageSpec> {
        let clean = spec.trim_start_matches("https://github.com/").trim_start_matches("github:");
        let parts: Vec<&str> = clean.split('/').collect();
        if parts.len() >= 2 {
            Some(PackageSpec {
                name: format!("{}/{}", parts[0], parts[1]),
                backend: "github".into(),
                options: HashMap::new(),
                requires: vec![],
            })
        } else {
            None
        }
    }

    /// Performs a GET request with GitHub Rate-Limit awareness and backoff.
    async fn github_get(&self, url: &str) -> Result<reqwest::Response> {
        let mut attempts = 0;
        loop {
            let res = self.client.get(url)
                .header("User-Agent", "linix-manager")
                .send().await?;
            
            if res.status() == 403 {
                if let Some(reset) = res.headers().get("x-ratelimit-reset") {
                    let reset_time = reset.to_str().unwrap_or("0").parse::<u64>().unwrap_or(0);
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                    if reset_time > now {
                        let wait = reset_time - now + 1;
                        warn!("GitHub Rate Limit reached. Pausing for {}s...", wait);
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                        attempts += 1;
                        if attempts > 3 { return Err(Error::RateLimit); }
                        continue;
                    }
                }
            }
            return Ok(res);
        }
    }

    /// Scores a release asset based on system compatibility (OS and Architecture).
    fn score_asset(&self, name: &str) -> i32 {
        let name = name.to_lowercase();
        let mut score = 0;
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        // OS Matching
        if name.contains(os) { score += 50; }
        else if os == "linux" && name.contains("linux") { score += 40; }
        else if os == "macos" && (name.contains("darwin") || name.contains("apple")) { score += 40; }

        // Architecture Matching
        if name.contains(arch) { score += 50; }
        else if arch == "x86_64" && (name.contains("amd64") || name.contains("x64")) { score += 45; }
        else if arch == "aarch64" && (name.contains("arm64") || name.contains("armv8")) { score += 45; }

        // Preferred formats
        if name.ends_with(".tar.gz") || name.ends_with(".zip") || name.ends_with(".tgz") { score += 10; }
        if name.contains("musl") && os == "linux" { score += 5; }
        
        // Penalize debug or source assets
        if name.contains("src") || name.contains("dev") || name.contains("dbg") { score -= 100; }
        
        score
    }
}

impl Backend for GithubManager {
    fn name(&self) -> &str { "github" }
    fn is_available(&self) -> bool { true }
    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
}

#[async_trait]
impl Installable for GithubManager {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        
        let mut state: HashMap<String, GithubState> = if self.state_file.exists() {
            let data = tokio::fs::read_to_string(&self.state_file).await?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            HashMap::new()
        };

        for spec in specs {
            let url = format!("https://api.github.com/repos/{}/releases/latest", spec.name);
            let res = self.github_get(&url).await?;
            let release: GithubRelease = res.json().await?;

            // 1. Asset Selection
            let filter = spec.options.get("asset_filter");
            let best_asset = release.assets.iter()
                .filter(|a| filter.map_or(true, |f| a.name.contains(f)))
                .max_by_key(|a| self.score_asset(&a.name))
                .ok_or_else(|| Error::PackageNotFound(format!("No compatible asset found for {} (filter: {:?})", spec.name, filter)))?;

            // 2. Version Check (Roadmap 2.2)
            if let Some(existing) = state.get(&spec.name) {
                if existing.version == release.version {
                    debug!("GitHub: {} is already at version {}", spec.name, release.version);
                    continue;
                }
            }

            // 3. Atomic Download & Verification
            info!("Downloading GitHub release: {} ({})", spec.name, release.version);
            let bytes = self.github_get(&best_asset.url).await?.bytes().await?;
            let tmp_dir = tempfile::tempdir()?;
            let dl_path = tmp_dir.path().join(&best_asset.name);
            tokio::fs::write(&dl_path, bytes).await?;

            if let Some(expected_sha) = spec.options.get("sha256") {
                verify_checksum(&dl_path, expected_sha)?;
            }

            // 4. Extraction & Linkage
            let pkg_dir_name = spec.name.replace('/', "_");
            let pkg_dir = self.install_dir.join(&pkg_dir_name);
            let _ = tokio::fs::remove_dir_all(&pkg_dir).await;
            tokio::fs::create_dir_all(&pkg_dir).await?;

            extract_archive(&dl_path, &pkg_dir)?;

            // Binary Discovery Logic
            let repo_name = spec.name.split('/').last().unwrap_or(&spec.name);
            let bin_dest = dirs::home_dir().unwrap().join(".local").join("bin").join(repo_name);
            
            let mut entries = walkdir::WalkDir::new(&pkg_dir).into_iter().filter_map(|e| e.ok());
            let mut found_bin = None;

            while let Some(entry) = entries.next() {
                let fname = entry.file_name().to_string_lossy().to_lowercase();
                // Match exact name or name without extension (common in Go/Rust projects)
                if fname == repo_name.to_lowercase() || (fname.starts_with(repo_name) && !fname.contains('.')) {
                    #[cfg(unix)] {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o755));
                        let _ = tokio::fs::remove_file(&bin_dest).await;
                        let _ = tokio::fs::create_dir_all(bin_dest.parent().unwrap()).await;
                        std::os::unix::fs::symlink(entry.path(), &bin_dest)?;
                        found_bin = Some(bin_dest.to_string_lossy().to_string());
                    }
                    break;
                }
            }

            // 5. State Persistence
            state.insert(spec.name.clone(), GithubState {
                repo: spec.name.clone(),
                version: release.version,
                bin_path: found_bin,
                install_path: pkg_dir.to_string_lossy().to_string(),
            });
        }
        
        let _ = tokio::fs::create_dir_all(self.state_file.parent().unwrap()).await;
        tokio::fs::write(&self.state_file, serde_json::to_string_pretty(&state)?).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        if !self.state_file.exists() { return Ok(()); }
        let mut state: HashMap<String, GithubState> = serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?;
        
        for name in names {
            if let Some(pkg) = state.remove(name) {
                if let Some(bp) = pkg.bin_path { 
                    let _ = tokio::fs::remove_file(bp).await; 
                }
                let _ = tokio::fs::remove_dir_all(pkg.install_path).await;
                info!("Purged GitHub package: {}", name);
            }
        }
        
        tokio::fs::write(&self.state_file, serde_json::to_string_pretty(&state)?).await?;
        Ok(())
    }
}

#[async_trait]
impl Queryable for GithubManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        if !self.state_file.exists() { return Ok(vec![]); }
        let state: HashMap<String, GithubState> = serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?;
        Ok(state.into_iter().map(|(n, s)| {
            Package::with_version(&n, &s.version, "github")
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}