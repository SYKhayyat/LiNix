// src/backends/github.rs

use crate::core::{
    security::verify_checksum, BackendCore, CommandExecutor, Error, HealthReport, HealthStatus,
    Installable, MetadataProvider, Package, PackageSpec, Queryable, RateLimiter, Result,
};
use crate::utils::archive::extract_archive;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

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

/// Core backend implementation for GitHub releases.
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
    pub fn new(
        executor: CommandExecutor,
        install_dir: PathBuf,
        github_token: Option<String>,
    ) -> Self {
        let rate_limiter = if github_token.is_some() {
            RateLimiter::github_authenticated()
        } else {
            RateLimiter::github()
        };

        let state_file = install_dir.join("installed.json");

        Self {
            executor,
            name: "github".to_string(),
            client: reqwest::Client::new(),
            install_dir,
            state_file,
            rate_limiter,
            github_token,
            internal_lock: Mutex::new(()),
        }
    }

    async fn github_get(&self, url: &str) -> Result<reqwest::Response> {
        self.rate_limiter
            .execute(|| async {
                let mut request_builder =
                    self.client.get(url).header("User-Agent", "linix-manager");

                if let Some(token) = &self.github_token {
                    request_builder =
                        request_builder.header("Authorization", format!("Bearer {}", token));
                }

                let res = request_builder.send().await.map_err(Error::from)?;

                if res.status() == 403 {
                    if let Some(reset) = res.headers().get("x-ratelimit-reset") {
                        let reset_time = reset.to_str().unwrap_or("0").parse::<u64>().unwrap_or(0);
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs();
                        if reset_time > now {
                            let wait = reset_time - now + 1;
                            warn!("GitHub Rate Limit reached. Pausing for {}s...", wait);
                            tokio::time::sleep(Duration::from_secs(wait)).await;
                        }
                    }
                }
                Ok(res)
            })
            .await
    }

    fn score_asset(&self, name: &str) -> i32 {
        let name = name.to_lowercase();
        let mut score = 0;
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        if name.contains(os) {
            score += 50;
        } else if (os == "linux" && name.contains("linux"))
            || (os == "macos" && (name.contains("darwin") || name.contains("apple")))
        {
            score += 40;
        }

        if name.contains(arch) {
            score += 50;
        } else if (arch == "x86_64" && (name.contains("amd64") || name.contains("x64")))
            || (arch == "aarch64" && (name.contains("arm64") || name.contains("armv8")))
        {
            score += 45;
        }

        if name.ends_with(".tar.gz") || name.ends_with(".zip") || name.ends_with(".tgz") {
            score += 10;
        }
        if name.contains("musl") && os == "linux" {
            score += 5;
        }
        if name.contains("src") || name.contains("dev") || name.contains("dbg") {
            score -= 100;
        }

        score
    }

    async fn load_state_internal(&self) -> HashMap<String, GithubState> {
        let _guard = self.internal_lock.lock().await;
        if !tokio::fs::try_exists(&self.state_file)
            .await
            .unwrap_or(false)
        {
            return HashMap::new();
        }
        let data = tokio::fs::read_to_string(&self.state_file)
            .await
            .unwrap_or_default();
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
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        true
    }
    fn needs_root(&self) -> bool {
        false
    }
    async fn check_health(&self) -> Result<HealthReport> {
        Ok(HealthReport {
            status: HealthStatus::Ok,
            message: None,
        })
    }
}

#[async_trait]
impl MetadataProvider for GithubBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
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

            let best_asset = release
                .assets
                .iter()
                .max_by_key(|a| self.core.score_asset(&a.name))
                .ok_or_else(|| Error::PackageNotFound(format!("No asset for {}", spec.name)))?;

            if let Some(existing) = state.get(&spec.name) {
                if existing.version == release.version {
                    debug!(
                        "GitHub: {} is already at version {}",
                        spec.name, release.version
                    );
                    continue;
                }
            }

            info!(
                "Downloading GitHub release: {} ({})",
                spec.name, release.version
            );
            let bytes = self
                .core
                .github_get(&best_asset.url)
                .await?
                .bytes()
                .await
                .map_err(Error::from)?;
            let tmp_dir = tempfile::tempdir().map_err(Error::from)?;
            let dl_path = tmp_dir.path().join(&best_asset.name);
            tokio::fs::write(&dl_path, bytes)
                .await
                .map_err(Error::from)?;

            if let Some(expected_sha) = spec.options.get("sha256") {
                verify_checksum(&dl_path, expected_sha)?;
            }

            let pkg_dir_name = spec.name.replace('/', "_");
            let pkg_dir = self.core.install_dir.join(&pkg_dir_name);
            if tokio::fs::try_exists(&pkg_dir).await.unwrap_or(false) {
                tokio::fs::remove_dir_all(&pkg_dir)
                    .await
                    .map_err(Error::from)?;
            }
            tokio::fs::create_dir_all(&pkg_dir)
                .await
                .map_err(Error::from)?;

            let dl_path_archive = dl_path.clone();
            let pkg_dir_archive = pkg_dir.clone();
            tokio::task::spawn_blocking(move || {
                extract_archive(&dl_path_archive, &pkg_dir_archive)
            })
            .await
            .map_err(|e| Error::Other(e.to_string()))??;

            let repo_name = spec.name.split('/').next_back().unwrap_or(&spec.name);
            let bin_dest_base = dirs::home_dir()
                .ok_or_else(|| Error::Other("Home directory not found".into()))?
                .join(".local")
                .join("bin")
                .join(repo_name);

            let mut final_bin_path = None;
            let core_pkg_dir = pkg_dir.clone();
            let repo_name_str = repo_name.to_string();

            let discovery_result: Result<Option<PathBuf>> =
                tokio::task::spawn_blocking(move || {
                    let walker = walkdir::WalkDir::new(&core_pkg_dir)
                        .into_iter()
                        .filter_map(|e| e.ok());
                    for entry in walker {
                        let fname = entry.file_name().to_string_lossy().to_lowercase();
                        if fname == repo_name_str.to_lowercase()
                            || fname == format!("{}.exe", repo_name_str.to_lowercase())
                            || (fname.starts_with(&repo_name_str) && !fname.contains('.'))
                        {
                            return Ok(Some(entry.path().to_path_buf()));
                        }
                    }
                    Ok(None)
                })
                .await
                .map_err(|e| Error::Other(e.to_string()))?;

            if let Some(src_path) = discovery_result? {
                #[allow(unused_mut)] // mutated only under cfg(windows)
                let mut bin_dest = bin_dest_base.clone();

                #[cfg(windows)]
                {
                    if bin_dest.extension().is_none() {
                        bin_dest.set_extension("exe");
                    }
                    tokio::fs::copy(&src_path, &bin_dest)
                        .await
                        .map_err(Error::from)?;
                }

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let metadata = tokio::fs::metadata(&src_path).await?;
                    let mut perms = metadata.permissions();
                    perms.set_mode(0o755);
                    tokio::fs::set_permissions(&src_path, perms)
                        .await
                        .map_err(Error::from)?;

                    if tokio::fs::try_exists(&bin_dest).await.unwrap_or(false)
                        || bin_dest.is_symlink()
                    {
                        tokio::fs::remove_file(&bin_dest)
                            .await
                            .map_err(Error::from)?;
                    }
                    if let Some(parent) = bin_dest.parent() {
                        tokio::fs::create_dir_all(parent)
                            .await
                            .map_err(Error::from)?;
                    }
                    // FIX: Use tokio::fs::symlink (correct async symlink API)
                    tokio::fs::symlink(&src_path, &bin_dest)
                        .await
                        .map_err(Error::from)?;
                }

                final_bin_path = Some(bin_dest.to_string_lossy().to_string());
            }

            state.insert(
                spec.name.clone(),
                GithubState {
                    repo: spec.name.clone(),
                    version: release.version,
                    bin_path: final_bin_path,
                    install_path: pkg_dir.to_string_lossy().to_string(),
                },
            );
        }

        self.core.save_state_internal(&state).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        let mut state = self.core.load_state_internal().await;
        for name in names {
            if let Some(pkg) = state.remove(name) {
                if let Some(ref bp) = pkg.bin_path {
                    let _ = tokio::fs::remove_file(bp).await;
                }
                let _ = tokio::fs::remove_dir_all(pkg.install_path).await;
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
        Ok(state
            .into_iter()
            .map(|(n, s)| Package::with_version(&n, &s.version, "github"))
            .collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

/// Build and register the GitHub Releases backend.
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let core = Arc::new(GithubBackendCore::new(
        exec.duplicate(),
        cfg.github_dir.clone(),
        cfg.github_token.clone(),
    ));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GithubInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GithubQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
