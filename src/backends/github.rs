use crate::core::{
    security::{generate_checksum, verify_checksum},
    verify_against, ArtifactLedger, ArtifactLock, BackendCore, CommandExecutor, Error,
    HealthReport, HealthStatus, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    RateLimiter, Result,
};
use crate::backends::artifact::{
    self, default_formats, ArtifactOptions, Asset as ArtifactAsset, Entry as ArchiveEntry, Format,
    FormatOrder, Platform, Request as SelectRequest,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GithubState {
    repo: String,
    version: String,
    bin_path: Option<String>,
    install_path: String,
    /// The resolved artifact. A record of only the version leaves the file free to change
    /// under a pinned declaration, which is what artifact selection exists to prevent.
    #[serde(default)]
    asset: Option<String>,
    #[serde(default)]
    format: Option<String>,
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

pub struct GithubBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub client: reqwest::Client,
    pub install_dir: PathBuf,
    pub state_file: PathBuf,
    /// `locks/github.toml` — what each declaration resolved to, in the config repo (VIII.2).
    /// Separate from `state_file`, which is LiNix's own bookkeeping and is not in git.
    pub locks_file: PathBuf,
    pub rate_limiter: RateLimiter,
    pub github_token: Option<String>,
    pub internal_lock: Mutex<()>,
}

impl GithubBackendCore {
    pub fn new(
        executor: CommandExecutor,
        install_dir: PathBuf,
        locks_file: PathBuf,
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
            locks_file,
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

/// What this backend can install today. A `.deb` would have to be handed to `dpkg`, which
/// puts it in apt's database and makes apt able to upgrade it out from under LiNix — an
/// ownership question that is recorded and unanswered, so the format is selected against
/// rather than half-installed.
fn installable_here(format: Format) -> bool {
    format.is_archive() || matches!(format, Format::AppImage | Format::Binary)
}

/// Windows has no executable bit, so the name is the only signal there.
#[cfg(unix)]
fn is_executable(entry: &walkdir::DirEntry) -> bool {
    use std::os::unix::fs::PermissionsExt;
    entry
        .metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(entry: &walkdir::DirEntry) -> bool {
    matches!(
        entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .as_deref(),
        Some("exe") | Some("bat") | Some("cmd")
    )
}

#[async_trait]
impl Installable for GithubInstallable {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let mut state = self.core.load_state_internal().await;
        let mut ledger = ArtifactLedger::load(&self.core.locks_file)?;

        for spec in specs {
            let url = format!("https://api.github.com/repos/{}/releases/latest", spec.name);
            let res = self.core.github_get(&url).await?;
            let release: GithubRelease = res.json().await.map_err(Error::from)?;

            let wanted = ArtifactOptions::read(&spec.options).map_err(Error::Validation)?;
            let asked = wanted.resolved_formats(&default_formats());
            let formats = asked.retaining(installable_here);
            if formats.is_empty() {
                let refused = asked.rejected_by(installable_here);
                return Err(Error::Validation(format!(
                    "{}: `github` cannot install {} — it unpacks archives, and a system \
                     package has to be handed to the package manager that owns it. Ask for \
                     one of: {}.",
                    spec.name,
                    refused
                        .iter()
                        .map(|f| f.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    FormatOrder::new(
                        Format::ALL.into_iter().filter(|f| installable_here(*f)).collect()
                    )
                )));
            }
            let offered: Vec<ArtifactAsset> = release
                .assets
                .iter()
                .map(|a| ArtifactAsset::new(&a.name, &a.url))
                .collect();

            let platform = Platform::current();
            let selection = artifact::select(
                &SelectRequest {
                    package: &spec.name,
                    release: &release.version,
                    platform: &platform,
                    formats: &formats,
                    pattern: wanted.asset.as_ref(),
                },
                &offered,
            )
            .map_err(|e| Error::PackageNotFound(e.to_string()))?;

            // A tie-break is a guess, and a guess nobody sees is the one that drifts.
            if selection.was_ambiguous() {
                let passed: Vec<&str> = selection
                    .passed_over
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect();
                info!(
                    "{}: chose {} over {}",
                    spec.name,
                    selection.picks[0].asset.name,
                    passed.join(", ")
                );
            }

            if selection.picks.len() > 1 {
                return Err(Error::Validation(format!(
                    "{}: `@asset=all` selected {} files, and installing several artifacts \
                     under one declaration is not built yet. Narrow it with a pattern, e.g. \
                     @asset=*musl*.",
                    spec.name,
                    selection.picks.len()
                )));
            }

            let chosen = &selection.picks[0];
            let best_asset = &chosen.asset;

            // The version alone is not the identity of what is installed: changing `formats`
            // on a pinned version must still reinstall, or the declaration and the disk part
            // ways with nothing to show it.
            if let Some(existing) = state.get(&spec.name) {
                if existing.version == release.version
                    && existing.asset.as_deref() == Some(best_asset.name.as_str())
                {
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

            // The same asset of the same release, with different bytes than last time. No
            // legitimate republish does that, so it is an alarm rather than an update — and
            // it must not be answered by selecting a different asset, which would turn a
            // supply-chain warning into a silent substitution (VIII.2).
            let downloaded_sha = generate_checksum(&dl_path)?;
            if let Some(locked) = ledger.get(&spec.name) {
                if locked.version.as_deref() == Some(release.version.as_str()) {
                    if let Some(objection) =
                        verify_against(locked, &best_asset.name, Some(&downloaded_sha))
                    {
                        return Err(Error::Validation(format!(
                            "{}: {}",
                            spec.name, objection
                        )));
                    }
                }
            }
            ledger.record(
                spec.name.clone(),
                ArtifactLock {
                    version: Some(release.version.clone()),
                    asset: best_asset.name.clone(),
                    url: best_asset.url.clone(),
                    format: chosen.format.to_string(),
                    sha256: Some(downloaded_sha),
                },
            );

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

            let final_bin_path;
            let core_pkg_dir = pkg_dir.clone();

            let listing: Vec<ArchiveEntry> = tokio::task::spawn_blocking(move || {
                walkdir::WalkDir::new(&core_pkg_dir)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                    .map(|e| {
                        let executable = is_executable(&e);
                        ArchiveEntry::new(e.path().to_path_buf(), executable)
                    })
                    .collect()
            })
            .await
            .map_err(|e| Error::Other(e.to_string()))?;

            let discovered = artifact::find_executable(&listing, &spec.name, wanted.bin.as_deref())
                .map_err(|e| Error::PackageNotFound(e.to_string()))?;

            {
                let src_path = discovered;
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
                    asset: Some(best_asset.name.clone()),
                    format: Some(chosen.format.to_string()),
                },
            );
        }

        self.core.save_state_internal(&state).await?;
        ledger.save(&self.core.locks_file)?;
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        let mut state = self.core.load_state_internal().await;
        let mut ledger = ArtifactLedger::load(&self.core.locks_file)?;
        let mut failures = Vec::new();
        for name in names {
            if let Some(pkg) = state.remove(name) {
                let mut errors = Vec::new();
                if let Some(ref bp) = pkg.bin_path {
                    if let Err(e) = crate::utils::remove_deployed_path(bp).await {
                        errors.push(e);
                    }
                }
                if let Err(e) = crate::utils::remove_deployed_path(&pkg.install_path).await {
                    errors.push(e);
                }
                if errors.is_empty() {
                    // The lock describes what is installed. Leaving the entry behind would
                    // pin a future install to an artifact chosen for a declaration that is
                    // gone.
                    ledger.forget(name);
                    info!("removed {}", name);
                } else {
                    // The binary is still on disk and still on PATH. Dropping it from state
                    // anyway would make it drift no `sync` can see, so put the record back.
                    state.insert(name.clone(), pkg);
                    failures.push(format!("{}: {}", name, errors.join("; ")));
                }
            }
        }
        self.core.save_state_internal(&state).await?;
        ledger.save(&self.core.locks_file)?;
        if !failures.is_empty() {
            return Err(Error::Other(format!(
                "could not remove {} GitHub package(s), still installed: {}",
                failures.len(),
                failures.join(", ")
            )));
        }
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

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let core = Arc::new(GithubBackendCore::new(
        exec.duplicate(),
        cfg.github_dir.clone(),
        cfg.config_root().join("locks").join("github.toml"),
        // A secret is the environment only, never a file (II.1) — `preferences.toml` is
        // committed to the repo it lives in, so a token key there is a token in git.
        std::env::var("LINIX_GITHUB_TOKEN")
            .ok()
            .filter(|t| !t.is_empty()),
    ));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GithubInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GithubQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
