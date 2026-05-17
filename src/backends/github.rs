use crate::core::{
    manager::{BackendCore, Installable, Queryable},
    security::verify_checksum,
    CommandExecutor, Error, Package, PackageSpec, Result, RateLimiter,
};
use crate::utils::archive::extract_archive;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
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

/// Core backend implementation for GitHub.
pub struct GithubBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub client: reqwest::Client,
    pub install_dir: PathBuf,
    pub state_file: PathBuf,
    pub rate_limiter: RateLimiter,
    pub github_token: Option<String>,
    internal_lock: Mutex<()>,
}

impl GithubBackendCore {
    pub fn new(executor: CommandExecutor, github_token: Option<String>) -> Self {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("github");
        
        let rate_limiter = if github_token.is_some() {
            RateLimiter::github_authenticated()
        } else {
            RateLimiter::github()
        };
        
        Self {
            executor,
            name: "github".to_string(),
            client: reqwest::Client::new(),
            install_dir: base.clone(),
            state_file: base.join("installed.json"),
            rate_limiter,
            github_token,
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
        self.rate_limiter.execute(|| async {
            let mut attempts = 0;
            let mut request_builder = self.client.get(url)
                .header("User-Agent", "linix-manager");
            
            if let Some(token) = &self.github_token {
                request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
            }
            
            loop {
                let res = request_builder.try_clone()
                    .ok_or_else(|| Error::Other("Failed to clone request".into()))?
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
        }).await
    }

    /// Scores a release asset based on system compatibility (OS and Architecture).
    fn score_asset(&self, name: &str) -> i32 {
        let name = name.to_lowercase();
        let mut score = 0;
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        if name.contains(os) { score += 50; }
        else if os == "linux" && name.contains("linux") { score += 40; }
        else if os == "macos" && (name.contains("darwin") || name.contains("apple")) { score += 40; }

        if name.contains(arch) { score += 50; }
        else if arch == "x86_64" && (name.contains("amd64") || name.contains("x64")) { score += 45; }
        else if arch == "aarch64" && (name.contains("arm64") || name.contains("armv8")) { score += 45; }

        if name.ends_with(".tar.gz") || name.ends_with(".zip") || name.ends_with(".tgz") { score += 10; }
        if name.contains("musl") && os == "linux" { score += 5; }
        
        if name.contains("src") || name.contains("dev") || name.contains("dbg") { score -= 100; }
        
        score
    }
    
    async fn load_state(&self) -> HashMap<String, GithubState> {
        let _guard = self.internal_lock.lock().await;
        if !self.state_file.exists() {
            return HashMap::new();
        }
        let data = tokio::fs::read_to_string(&self.state_file).await.unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_default()
    }
    
    async fn save_state(&self, state: &HashMap<String, GithubState>) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        let data = serde_json::to_string_pretty(state).map_err(|e| Error::Other(e.to_string()))?;
        crate::utils::file::atomic_write(&self.state_file, &data)
    }
}

impl BackendCore for GithubBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// Installable capability for GitHub backend.
pub struct GithubInstallable {
    pub core: Arc<GithubBackendCore>,
}

#[async_trait]
impl Installable for GithubInstallable {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let mut state = self.core.load_state().await;

        for spec in specs {
            let url = format!("https://api.github.com/repos/{}/releases/latest", spec.name);
            let res = self.core.github_get(&url).await?;
            let release: GithubRelease = res.json().await?;

            let filter = spec.options.get("asset_filter");
            let best_asset = release.assets.iter()
                .filter(|a| filter.map_or(true, |f| a.name.contains(f)))
                .max_by_key(|a| self.core.score_asset(&a.name))
                .ok_or_else(|| Error::PackageNotFound(format!("No compatible asset found for {}", spec.name)))?;

            if let Some(existing) = state.get(&spec.name) {
                if existing.version == release.version {
                    debug!("GitHub: {} is already at version {}", spec.name, release.version);
                    continue;
                }
            }

            info!("Downloading GitHub release: {} ({})", spec.name, release.version);
            let bytes = self.core.github_get(&best_asset.url).await?.bytes().await?;
            let tmp_dir = tempfile::tempdir()?;
            let dl_path = tmp_dir.path().join(&best_asset.name);
            tokio::fs::write(&dl_path, bytes).await?;

            if let Some(expected_sha) = spec.options.get("sha256") {
                verify_checksum(&dl_path, expected_sha)?;
            }

            let pkg_dir_name = spec.name.replace('/', "_");
            let pkg_dir = self.core.install_dir.join(&pkg_dir_name);
            let _ = tokio::fs::remove_dir_all(&pkg_dir).await;
            tokio::fs::create_dir_all(&pkg_dir).await?;

            extract_archive(&dl_path, &pkg_dir)?;

            let repo_name = spec.name.split('/').last().unwrap_or(&spec.name);
            let bin_dest = dirs::home_dir().unwrap().join(".local").join("bin").join(repo_name);
            
            let mut entries = walkdir::WalkDir::new(&pkg_dir).into_iter().filter_map(|e| e.ok());
            let mut found_bin = None;

            while let Some(entry) = entries.next() {
                let fname = entry.file_name().to_string_lossy().to_lowercase();
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

            state.insert(spec.name.clone(), GithubState {
                repo: spec.name.clone(),
                version: release.version,
                bin_path: found_bin,
                install_path: pkg_dir.to_string_lossy().to_string(),
            });
        }
        
        let _ = tokio::fs::create_dir_all(self.core.state_file.parent().unwrap()).await;
        self.core.save_state(&state).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        let mut state = self.core.load_state().await;
        
        for name in names {
            if let Some(pkg) = state.remove(name) {
                if let Some(bp) = pkg.bin_path { 
                    let _ = tokio::fs::remove_file(bp).await; 
                }
                let _ = tokio::fs::remove_dir_all(pkg.install_path).await;
                info!("Purged GitHub package: {}", name);
            }
        }
        
        self.core.save_state(&state).await?;
        Ok(())
    }
}

/// Queryable capability for GitHub backend.
pub struct GithubQueryable {
    pub core: Arc<GithubBackendCore>,
}

#[async_trait]
impl Queryable for GithubQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let state = self.core.load_state().await;
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