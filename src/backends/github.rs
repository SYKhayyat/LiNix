use crate::core::{
    security::{generate_checksum, verify_checksum},
    verify_against, ArtifactLedger, ArtifactLock, BackendCore, CommandExecutor, Error,
    HealthReport, HealthStatus, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    RateLimiter, Result,
};
use crate::backends::artifact::{
    self, default_formats, ArtifactOptions, Asset as ArtifactAsset, AssetPattern,
    Entry as ArchiveEntry, Format, FormatOrder, Platform, Request as SelectRequest,
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

    /// `None` when the release does not exist, which is an answer rather than a failure: a pin
    /// is tried under both tag spellings and one of the two is expected to be absent.
    async fn release_at(&self, url: &str) -> Result<Option<GithubRelease>> {
        let res = self.github_get(url).await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let res = res.error_for_status().map_err(Error::from)?;
        res.json().await.map(Some).map_err(Error::from)
    }

    async fn resolve_release(&self, repo: &str, pin: Option<&str>) -> Result<GithubRelease> {
        let Some(pin) = pin else {
            // `releases/latest` is GitHub's own newest non-draft, non-prerelease release.
            // Filtering the full list here would be a second definition of the same thing,
            // free to drift from theirs.
            let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
            return self.release_at(&url).await?.ok_or_else(|| {
                Error::PackageNotFound(format!("{}: the repo has no published release", repo))
            });
        };

        let [bare, prefixed] = tag_spellings(pin);
        let found_bare = self.release_at(&tag_url(repo, &bare)).await?;
        let found_prefixed = self.release_at(&tag_url(repo, &prefixed)).await?;
        one_release(repo, pin, found_bare, found_prefixed)
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

fn tag_url(repo: &str, tag: &str) -> String {
    format!("https://api.github.com/repos/{}/releases/tags/{}", repo, tag)
}

/// The two tags a pin may name. Roughly half of GitHub tags carry a leading `v`, so `@version=`
/// is spelled one way and tagged the other often enough that trying only what was written finds
/// nothing on half the repos.
fn tag_spellings(pin: &str) -> [String; 2] {
    let bare = pin.strip_prefix('v').unwrap_or(pin);
    [bare.to_string(), format!("v{}", bare)]
}

fn same_tag(pin: &str, tag: &str) -> bool {
    let [bare, _] = tag_spellings(pin);
    let [tag_bare, _] = tag_spellings(tag);
    bare == tag_bare
}

/// A repo carrying both `10.2.0` and `v10.2.0` has two releases answering to one pin, and
/// choosing between them here would install a version the user never named.
fn one_release(
    repo: &str,
    pin: &str,
    bare: Option<GithubRelease>,
    prefixed: Option<GithubRelease>,
) -> Result<GithubRelease> {
    match (bare, prefixed) {
        (Some(b), Some(p)) => Err(Error::Validation(format!(
            "{}: @version={} matches two releases, `{}` and `{}`. Name the tag you mean.",
            repo, pin, b.version, p.version
        ))),
        (Some(r), None) | (None, Some(r)) => Ok(r),
        (None, None) => {
            let [bare, prefixed] = tag_spellings(pin);
            Err(Error::PackageNotFound(format!(
                "{}: no release tagged `{}` or `{}`",
                repo, bare, prefixed
            )))
        }
    }
}

/// What local files already know about a declaration, before anything is asked of GitHub.
struct Known<'a> {
    pin: Option<&'a str>,
    locked: Option<&'a ArtifactLock>,
    installed: Option<&'a GithubState>,
}

/// Whether the lock and the install already answer the declaration, so no API call is owed.
///
/// Only a pinned line can be answered this way: an unpinned one asks for whatever is newest,
/// and only GitHub knows that. The formats and `@asset=` checks are what keep a pinned line
/// honest — changing either asks for a different artifact of the same release, and only a
/// re-selection can find it.
fn answered_locally(known: &Known, formats: &FormatOrder, asset: Option<&AssetPattern>) -> bool {
    let (Some(pin), Some(locked), Some(installed)) = (known.pin, known.locked, known.installed)
    else {
        return false;
    };
    let Some(locked_version) = locked.version.as_deref() else {
        return false;
    };
    if !same_tag(pin, locked_version) || installed.version != locked_version {
        return false;
    }
    if installed.asset.as_deref() != Some(locked.asset.as_str()) {
        return false;
    }
    if Format::parse(&locked.format)
        .ok()
        .and_then(|f| formats.rank(f))
        .is_none()
    {
        return false;
    }
    asset.is_none_or(|pattern| pattern.matches(&locked.asset))
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

            let pin = spec
                .options
                .get("version")
                .map(|v| v.trim())
                .filter(|v| !v.is_empty());

            let known = Known {
                pin,
                locked: ledger.get(&spec.name),
                installed: state.get(&spec.name),
            };
            if answered_locally(&known, &formats, wanted.asset.as_ref()) {
                debug!(
                    "GitHub: {} is locked at {} and installed — no API call",
                    spec.name,
                    pin.unwrap_or_default()
                );
                continue;
            }

            let release = self.core.resolve_release(&spec.name, pin).await?;

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
                #[allow(unused_mut)] // mutated only under cfg(windows)
                let mut bin_dest = bin_dest_base.clone();
                #[cfg(windows)]
                if bin_dest.extension().is_none() {
                    bin_dest.set_extension("exe");
                }

                crate::utils::deploy_executable(
                    &discovered,
                    &bin_dest,
                    &self.core.install_dir,
                    state.get(&spec.name).and_then(|s| s.bin_path.as_deref()),
                )
                .await?;

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
        cfg.layout().lock_file("github"),
        // A secret is the environment only, never a file (II.1) — `preferences.toml` is
        // committed to the repo it lives in, so a token key there is a token in git.
        std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()),
    ));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GithubInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GithubQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> GithubRelease {
        GithubRelease {
            version: tag.to_string(),
            assets: vec![],
        }
    }

    fn lock(version: &str, asset: &str, format: &str) -> ArtifactLock {
        ArtifactLock {
            version: Some(version.to_string()),
            asset: asset.to_string(),
            url: format!("https://example.invalid/{}", asset),
            format: format.to_string(),
            sha256: Some("abc123".into()),
        }
    }

    fn installed(version: &str, asset: &str) -> GithubState {
        GithubState {
            repo: "sharkdp/fd".into(),
            version: version.to_string(),
            bin_path: Some("/home/u/.local/bin/fd".into()),
            install_path: "/opt/linix/sharkdp_fd".into(),
            asset: Some(asset.to_string()),
            format: Some("tarball".into()),
        }
    }

    fn tarballs() -> FormatOrder {
        FormatOrder::new(vec![Format::Tarball, Format::Binary])
    }

    #[test]
    fn a_pinned_version_matches_the_tag_with_or_without_a_v() {
        assert_eq!(tag_spellings("10.2.0"), tag_spellings("v10.2.0"));
        assert!(same_tag("10.2.0", "v10.2.0"));
        assert!(same_tag("v10.2.0", "10.2.0"));
        assert!(!same_tag("10.2.0", "10.2.1"));
    }

    #[test]
    fn a_pin_that_answers_to_both_spellings_is_an_error_naming_both() {
        let err = one_release(
            "sharkdp/fd",
            "10.2.0",
            Some(release("10.2.0")),
            Some(release("v10.2.0")),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("10.2.0"), "{}", err);
        assert!(err.contains("v10.2.0"), "{}", err);
    }

    #[test]
    fn either_spelling_alone_resolves_to_that_release() {
        let from_bare = one_release("sharkdp/fd", "10.2.0", Some(release("10.2.0")), None).unwrap();
        assert_eq!(from_bare.version, "10.2.0");
        let from_prefixed =
            one_release("sharkdp/fd", "10.2.0", None, Some(release("v10.2.0"))).unwrap();
        assert_eq!(from_prefixed.version, "v10.2.0");
    }

    #[test]
    fn a_pin_no_tag_answers_names_both_spellings_it_tried() {
        let err = one_release("sharkdp/fd", "9.9.9", None, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("9.9.9"), "{}", err);
        assert!(err.contains("v9.9.9"), "{}", err);
    }

    #[test]
    fn a_pinned_installed_package_is_answered_without_the_network() {
        let locked = lock("v10.2.0", "fd.tar.gz", "tarball");
        let state = installed("v10.2.0", "fd.tar.gz");
        let known = Known {
            pin: Some("10.2.0"),
            locked: Some(&locked),
            installed: Some(&state),
        };
        assert!(answered_locally(&known, &tarballs(), None));
    }

    #[test]
    fn an_unpinned_line_always_asks_github() {
        let locked = lock("v10.2.0", "fd.tar.gz", "tarball");
        let state = installed("v10.2.0", "fd.tar.gz");
        let known = Known {
            pin: None,
            locked: Some(&locked),
            installed: Some(&state),
        };
        assert!(!answered_locally(&known, &tarballs(), None));
    }

    #[test]
    fn a_pin_that_moved_past_the_lock_asks_github() {
        let locked = lock("v10.2.0", "fd.tar.gz", "tarball");
        let state = installed("v10.2.0", "fd.tar.gz");
        let known = Known {
            pin: Some("10.3.0"),
            locked: Some(&locked),
            installed: Some(&state),
        };
        assert!(!answered_locally(&known, &tarballs(), None));
    }

    #[test]
    fn a_lock_without_an_install_asks_github() {
        let locked = lock("v10.2.0", "fd.tar.gz", "tarball");
        let known = Known {
            pin: Some("10.2.0"),
            locked: Some(&locked),
            installed: None,
        };
        assert!(!answered_locally(&known, &tarballs(), None));
    }

    #[test]
    fn an_install_that_drifted_from_the_lock_asks_github() {
        let locked = lock("v10.2.0", "fd-gnu.tar.gz", "tarball");
        let state = installed("v10.2.0", "fd-musl.tar.gz");
        let known = Known {
            pin: Some("10.2.0"),
            locked: Some(&locked),
            installed: Some(&state),
        };
        assert!(!answered_locally(&known, &tarballs(), None));
    }

    #[test]
    fn changing_formats_under_a_pin_asks_github_again() {
        let locked = lock("v10.2.0", "fd.tar.gz", "tarball");
        let state = installed("v10.2.0", "fd.tar.gz");
        let known = Known {
            pin: Some("10.2.0"),
            locked: Some(&locked),
            installed: Some(&state),
        };
        assert!(!answered_locally(
            &known,
            &FormatOrder::new(vec![Format::Deb]),
            None
        ));
    }

    #[test]
    fn changing_the_asset_pattern_under_a_pin_asks_github_again() {
        let locked = lock("v10.2.0", "fd-gnu.tar.gz", "tarball");
        let state = installed("v10.2.0", "fd-gnu.tar.gz");
        let known = Known {
            pin: Some("10.2.0"),
            locked: Some(&locked),
            installed: Some(&state),
        };
        let musl = AssetPattern::parse("*musl*").unwrap();
        assert!(!answered_locally(&known, &tarballs(), Some(&musl)));
        let gnu = AssetPattern::parse("*gnu*").unwrap();
        assert!(answered_locally(&known, &tarballs(), Some(&gnu)));
    }
}
