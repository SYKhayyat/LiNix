use crate::core::{
    BackendCore, Installable, Queryable,
    security::verify_checksum,
    CommandExecutor, Error, Package, PackageSpec, Result, RateLimiter, HealthReport, HealthStatus
};
use crate::utils::archive::extract_archive;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::Mutex;
use std::sync::Arc;
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
    pub internal_lock: Mutex<()>,
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

    async fn github_get(&self, url: &str) -> Result<reqwest::Response> {
        self.rate_limiter.execute(|| async {
            let mut request_builder = self.client.get(url)
                .header("User-Agent", "linix-manager");
            
            if let Some(token) = &self.github_token {
                request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
            }
            
            let res = request_builder.send().await.map_err(Error::from)?;
            
            if res.status() == 403 {
                if let Some(reset) = res.headers().get("x-ratelimit-reset") {
                    let reset_time = reset.to_str().unwrap_or("0").parse::<u64>().unwrap_or(0);
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                    if reset_time > now {
                        let wait = reset_time - now + 1;
                        warn!("GitHub Rate Limit reached. Pausing for {}s...", wait);
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                    }
                }
            }
            Ok(res)
        }).await
    }

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
    
    async fn load_state_internal(&self) -> HashMap<String, GithubState> {
        let _guard = self.internal_lock.lock().await;
        if !self.state_file.exists() {
            return HashMap::new();
        }
        let data = std::fs::read_to_string(&self.state_file).unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_default()
    }
    
    async fn save_state_internal(&self, state: &HashMap<String, GithubState>) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        let data = serde_json::to_string_pretty(state).map_err(Error::from)?;
        crate::utils::file::atomic_write(&self.state_file, &data)
    }
}

#[async_trait]
impl BackendCore for GithubBackendCore {
    fn name(&self) -> &str { &self.name }
    fn is_available(&self) -> bool { true }
    async fn check_health(&self) -> Result<HealthReport> {
        Ok(HealthReport { status: HealthStatus::Ok, message: None })
    }
}

pub struct GithubInstallable {
    pub core: Arc<GithubBackendCore>,
}

#[async_trait]
impl Installable for GithubInstallable {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let mut state = self.core.load_state_internal().await;

        for spec in specs {
            let url = format!("https://api.github.com/repos/{}/releases/latest", spec.name);
            let res = self.core.github_get(&url).await?;
            let release: GithubRelease = res.json().await.map_err(Error::from)?;

            let best_asset = release.assets.iter()
                .max_by_key(|a| self.core.score_asset(&a.name))
                .ok_or_else(|| Error::PackageNotFound(format!("No asset for {}", spec.name)))?;

            if let Some(existing) = state.get(&spec.name) {
                if existing.version == release.version {
                    debug!("GitHub: {} is already at version {}", spec.name, release.version);
                    continue;
                }
            }

            info!("Downloading GitHub release: {} ({})", spec.name, release.version);
            let bytes = self.core.github_get(&best_asset.url).await?.bytes().await.map_err(Error::from)?;
            let tmp_dir = tempfile::tempdir().map_err(Error::from)?;
            let dl_path = tmp_dir.path().join(&best_asset.name);
            std::fs::write(&dl_path, bytes).map_err(Error::from)?;

            if let Some(expected_sha) = spec.options.get("sha256") {
                verify_checksum(&dl_path, expected_sha)?;
            }

            let pkg_dir_name = spec.name.replace('/', "_");
            let pkg_dir = self.core.install_dir.join(&pkg_dir_name);
            let _ = std::fs::remove_dir_all(&pkg_dir);
            std::fs::create_dir_all(&pkg_dir).map_err(Error::from)?;

            extract_archive(&dl_path, &pkg_dir)?;

            let repo_name = spec.name.split('/').last().unwrap_or(&spec.name);
            let bin_dest_base = dirs::home_dir()
                .ok_or_else(|| Error::Other("Home directory not found".into()))?
                .join(".local").join("bin").join(repo_name);
            
            let mut final_bin_path = None;
            let walker = walkdir::WalkDir::new(&pkg_dir).into_iter().filter_map(|e| e.ok());
            
            for entry in walker {
                let fname = entry.file_name().to_string_lossy().to_lowercase();
                if fname == repo_name.to_lowercase() || 
                   fname == format!("{}.exe", repo_name.to_lowercase()) ||
                   (fname.starts_with(repo_name) && !fname.contains('.')) 
                {
                    let src_path = entry.path();
                    let mut bin_dest = bin_dest_base.clone();
                    
                    #[cfg(windows)] {
                        if bin_dest.extension().is_none() { bin_dest.set_extension("exe"); }
                        std::fs::copy(src_path, &bin_dest).map_err(Error::from)?;
                    }

                    #[cfg(unix)] {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(src_path, std::fs::Permissions::from_mode(0o755));
                        let _ = std::fs::remove_file(&bin_dest);
                        let _ = std::fs::create_dir_all(bin_dest.parent().unwrap());
                        std::os::unix::fs::symlink(src_path, &bin_dest).map_err(Error::from)?;
                    }

                    final_bin_path = Some(bin_dest.to_string_lossy().to_string());
                    break;
                }
            }

            state.insert(spec.name.clone(), GithubState {
                repo: spec.name.clone(),
                version: release.version,
                bin_path: final_bin_path,
                install_path: pkg_dir.to_string_lossy().to_string(),
            });
        }
        
        self.core.save_state_internal(&state).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        let mut state = self.core.load_state_internal().await;
        for name in names {
            if let Some(pkg) = state.remove(name) {
                if let Some(ref bp) = pkg.bin_path {
                    let _ = std::fs::remove_file(bp);
                }
                let _ = std::fs::remove_dir_all(pkg.install_path);
                info!("Purged GitHub package: {}", name);
            }
        }
        self.core.save_state_internal(&state).await?;
        Ok(())
    }
}

pub struct GithubQueryable {
    pub core: Arc<GithubBackendCore>,
}

#[async_trait]
impl Queryable for GithubQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let state = self.core.load_state_internal().await;
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