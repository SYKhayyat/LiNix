use crate::backends::artifact::{
    self, default_formats, system_pkg, ArtifactOptions, Asset as ArtifactAsset, AssetPattern,
    Entry as ArchiveEntry, Format, FormatOrder, Platform, Request as SelectRequest,
};
use crate::core::{
    security::{generate_checksum, verify_checksum},
    verify_set, ArtifactLedger, ArtifactLock, BackendCore, CommandExecutor, Error, HealthReport,
    HealthStatus, Installable, MetadataProvider, Package, PackageSpec, Queryable, RateLimiter,
    Result,
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
    /// The declaration's directory. Every artifact it installed unpacks under here, so a
    /// removal has one tree to delete however many files the line resolved to.
    install_path: String,
    /// The resolved artifacts, in selection order. A record of only the version leaves the
    /// file free to change under a pinned declaration, which is what artifact selection exists
    /// to prevent; `@asset=all` is the only way there is more than one.
    artifacts: Vec<InstalledArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstalledArtifact {
    asset: String,
    format: String,
    bin_path: Option<String>,
    /// The system manager that owns this artifact (D5), when the file installed itself into a
    /// package database (`dpkg`/`rpm`) rather than being deployed to PATH by LiNix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    installed_by: Option<String>,
    /// The name that manager knows it as — what removal and dedup use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system_package: Option<String>,
}

impl GithubState {
    fn assets(&self) -> Vec<&str> {
        self.artifacts.iter().map(|a| a.asset.as_str()).collect()
    }
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
    /// `[guard] confine_bin`: whether the deployed name may reach outside `~/.local/bin` (SEC1).
    pub confine_bin: bool,
    /// K4: also clean each fetched asset from the cache locations on removal.
    pub clean_cache_on_remove: bool,
    pub cache_dirs: Vec<PathBuf>,
    pub github_token: Option<String>,
    /// `rate_limit_max_wait_secs`: the ceiling on waiting out a 403 (S26).
    pub rate_limit_max_wait: Duration,
    pub internal_lock: Mutex<()>,
}

/// What to do with a response that may be a rate limit (S26).
#[derive(Debug, PartialEq, Eq)]
enum RateLimit {
    /// Not a rate limit: a success, or a 403 for a reason waiting cannot change (a bad token,
    /// a private repo, a limit that has already reset).
    NotLimited,
    /// Wait this many seconds, then retry once.
    WaitThenRetry(u64),
    /// The reset is further out than the ceiling. Say so; do not sleep on a held lock.
    TooLong(u64),
}

/// Decide from a 403's `x-ratelimit-reset` header. Pure — the clock and the ceiling are
/// arguments — because the decision is the part that has to be right, and an integration test
/// against api.github.com cannot make a rate limit happen on demand.
fn rate_limit_action(status: u16, reset_header: Option<&str>, now: u64, cap: u64) -> RateLimit {
    if status != 403 {
        return RateLimit::NotLimited;
    }
    let Some(reset) = reset_header.and_then(|h| h.trim().parse::<u64>().ok()) else {
        return RateLimit::NotLimited;
    };
    if reset <= now {
        return RateLimit::NotLimited;
    }
    let wait = reset - now + 1;
    if wait > cap {
        RateLimit::TooLong(wait)
    } else {
        RateLimit::WaitThenRetry(wait)
    }
}

fn rate_limit_of(res: &reqwest::Response, cap: u64) -> RateLimit {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let header = res
        .headers()
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok());
    rate_limit_action(res.status().as_u16(), header, now, cap)
}

impl GithubBackendCore {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        executor: CommandExecutor,
        install_dir: PathBuf,
        locks_file: PathBuf,
        confine_bin: bool,
        clean_cache_on_remove: bool,
        cache_dirs: Vec<PathBuf>,
        github_token: Option<String>,
        rate_limit_max_wait: Duration,
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
            // No downgrade across redirects (SEC2). GitHub asset URLs redirect to a CDN,
            // and the hop is where a promised HTTPS download can stop being one.
            client: crate::core::download::client(false, "linix-manager")
                .unwrap_or_else(|_| reqwest::Client::new()),
            install_dir,
            state_file,
            locks_file,
            rate_limiter,
            confine_bin,
            clean_cache_on_remove,
            cache_dirs,
            github_token,
            rate_limit_max_wait,
            internal_lock: Mutex::new(()),
        }
    }

    /// SEC2: every URL this backend fetches must be HTTPS, including an asset URL that a
    /// release points at — the API is HTTPS, but `browser_download_url` is whatever the
    /// release published, and a redirect can leave the scheme the API promised.
    async fn github_get(&self, url: &str) -> Result<reqwest::Response> {
        crate::core::download::check_scheme(url, false, url)?;
        self.rate_limiter
            .execute(|| async {
                let cap = self.rate_limit_max_wait.as_secs();
                let res = self.send(url).await?;
                match rate_limit_of(&res, cap) {
                    RateLimit::NotLimited => Ok(res),
                    RateLimit::TooLong(wait) => Err(Error::RateLimit(format!(
                        "api.github.com is rate limiting this machine and does not reset for \
                         {}s, past the {}s ceiling. Raise `rate_limit_max_wait_secs` in \
                         preferences.toml to wait it out, or set GITHUB_TOKEN for a far \
                         larger allowance.",
                        wait, cap
                    ))),
                    RateLimit::WaitThenRetry(wait) => {
                        warn!(
                            "api.github.com rate limit reached; waiting {}s for it to reset, \
                             then retrying once.",
                            wait
                        );
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                        // One retry. The old code slept and returned the same 403, so the
                        // wait bought nothing at all — up to an hour of a held data lock
                        // followed by the identical error.
                        self.send(url).await
                    }
                }
            })
            .await
    }

    async fn send(&self, url: &str) -> Result<reqwest::Response> {
        let mut request_builder = self.client.get(url).header("User-Agent", "linix-manager");
        if let Some(token) = &self.github_token {
            request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
        }
        request_builder.send().await.map_err(Error::from)
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

/// What this backend can install, given which system installers this machine has (D5). A `.deb`
/// is installable only where `dpkg` exists and an `.rpm` only where `rpm` does — the file is
/// handed to that manager, which then *owns* it (removal, upgrade and dedup route back through
/// the recorded installer). On a machine without the manager the format is not installable, so a
/// line that offers only a `.deb` there falls through to download-only rather than failing.
fn installable_here(format: Format, has_dpkg: bool, has_rpm: bool) -> bool {
    if format.is_archive() || matches!(format, Format::AppImage | Format::Binary) {
        return true;
    }
    match system_pkg::installer_for(format) {
        Some("dpkg") => has_dpkg,
        Some("rpm") => has_rpm,
        _ => false,
    }
}

fn tag_url(repo: &str, tag: &str) -> String {
    format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        repo, tag
    )
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

/// Two asset lists naming the same files, whatever order the release offered them in.
fn same_set(a: &[&str], b: &[&str]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

/// The subdirectory one artifact unpacks into, from its filename. Nothing here reaches the
/// user: it exists so two archives under one declaration cannot overwrite each other.
fn artifact_dir_name(asset: &str) -> String {
    asset
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// What local files already know about a declaration, before anything is asked of GitHub.
struct Known<'a> {
    pin: Option<&'a str>,
    locked: &'a [ArtifactLock],
    installed: Option<&'a GithubState>,
}

/// Whether the lock and the install already answer the declaration, so no API call is owed.
///
/// Only a pinned line can be answered this way: an unpinned one asks for whatever is newest,
/// and only GitHub knows that. The formats and `@asset=` checks are what keep a pinned line
/// honest — changing either asks for a different artifact of the same release, and only a
/// re-selection can find it.
fn answered_locally(known: &Known, formats: &FormatOrder, asset: Option<&AssetPattern>) -> bool {
    let (Some(pin), Some(installed)) = (known.pin, known.installed) else {
        return false;
    };
    if known.locked.is_empty() {
        return false;
    }
    let Some(locked_version) = known.locked[0].version.as_deref() else {
        return false;
    };
    if !same_tag(pin, locked_version) || installed.version != locked_version {
        return false;
    }
    let mut locked_assets: Vec<&str> = known.locked.iter().map(|l| l.asset.as_str()).collect();
    let mut on_disk = installed.assets();
    locked_assets.sort_unstable();
    on_disk.sort_unstable();
    if locked_assets != on_disk {
        return false;
    }
    known.locked.iter().all(|l| {
        Format::parse(&l.format)
            .ok()
            .and_then(|f| formats.rank(f))
            .is_some()
            && asset.is_none_or(|pattern| pattern.matches(&l.asset))
    })
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

        // Which system installers this machine has, computed once: it gates whether a `.deb`/
        // `.rpm` is installable here at all (D5), and a missing one turns such a line into a
        // download-only keep rather than a failure.
        let has_dpkg = self.core.executor.command_exists("dpkg").await;
        let has_rpm = self.core.executor.command_exists("rpm").await;

        for spec in specs {
            let wanted = ArtifactOptions::read(&spec.options).map_err(Error::Validation)?;
            let asked = wanted.resolved_formats(&default_formats());
            let installable = asked.retaining(|f| installable_here(f, has_dpkg, has_rpm));
            // D3b: `@download_only` fetches without installing, and github does the same by
            // default when nothing it was asked for is installable (e.g. only a `.deb` on
            // offer and no `.deb` handoff yet). Rather than failing, it keeps the file — still
            // declared, still removed when the line goes, just never unpacked or put on PATH.
            let download_only = wanted.download_only || installable.is_empty();
            let formats = if download_only {
                asked.clone()
            } else {
                installable
            };

            let pin = spec
                .options
                .get("version")
                .map(|v| v.trim())
                .filter(|v| !v.is_empty());

            let known = Known {
                pin,
                locked: ledger.locked(&spec.name),
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

            // The version alone is not the identity of what is installed: changing `formats`
            // on a pinned version must still reinstall, or the declaration and the disk part
            // ways with nothing to show it.
            let chosen_assets: Vec<&str> = selection
                .picks
                .iter()
                .map(|p| p.asset.name.as_str())
                .collect();
            if let Some(existing) = state.get(&spec.name) {
                if existing.version == release.version
                    && same_set(&existing.assets(), &chosen_assets)
                {
                    debug!(
                        "GitHub: {} is already at version {}",
                        spec.name, release.version
                    );
                    continue;
                }
            }

            // Everything is downloaded and hashed before anything is unpacked or put on PATH:
            // with several artifacts under one declaration, a supply-chain objection to the
            // third must not arrive with the first two already deployed.
            info!(
                "Downloading GitHub release: {} ({}), {} artifact(s)",
                spec.name,
                release.version,
                selection.picks.len()
            );
            let tmp_dir = tempfile::tempdir().map_err(Error::from)?;
            let mut downloaded: Vec<(&artifact::Pick, PathBuf, String)> = Vec::new();
            for pick in &selection.picks {
                let bytes = self
                    .core
                    .github_get(&pick.asset.url)
                    .await?
                    .bytes()
                    .await
                    .map_err(Error::from)?;
                let dl_path = tmp_dir.path().join(&pick.asset.name);
                tokio::fs::write(&dl_path, bytes)
                    .await
                    .map_err(Error::from)?;

                // `@sha256` is legal only on a line that resolves to exactly one file
                // (VIII.2/D6), so it needs no per-artifact story here.
                if let Some(expected_sha) = spec.options.get("sha256") {
                    verify_checksum(&dl_path, expected_sha)?;
                }
                let sha = generate_checksum(&dl_path)?;
                downloaded.push((pick, dl_path, sha));
            }

            // The same asset of the same release, with different bytes than last time. No
            // legitimate republish does that, so it is an alarm rather than an update — and
            // it must not be answered by selecting a different asset, which would turn a
            // supply-chain warning into a silent substitution (VIII.2).
            let locked = ledger.locked(&spec.name);
            if locked.first().and_then(|l| l.version.as_deref()) == Some(release.version.as_str()) {
                let resolved: Vec<(String, Option<String>)> = downloaded
                    .iter()
                    .map(|(p, _, sha)| (p.asset.name.clone(), Some(sha.clone())))
                    .collect();
                if let Some(objection) = verify_set(locked, &resolved) {
                    return Err(Error::Validation(format!("{}: {}", spec.name, objection)));
                }
            }

            // D3b: download-only. Keep each fetched file under the declaration's directory;
            // never unpack it, discover an executable, or put anything on PATH. Still recorded
            // and locked, so a removal still deletes it and a re-download that differs is caught.
            if download_only {
                let pkg_dir = self.core.install_dir.join(spec.name.replace('/', "_"));
                if tokio::fs::try_exists(&pkg_dir).await.unwrap_or(false) {
                    tokio::fs::remove_dir_all(&pkg_dir)
                        .await
                        .map_err(Error::from)?;
                }
                tokio::fs::create_dir_all(&pkg_dir)
                    .await
                    .map_err(Error::from)?;

                let reason =
                    artifact::selection_reason(wanted.asset.is_some(), formats.is_user_specified())
                        .to_string();
                let previous: Vec<String> = state
                    .get(&spec.name)
                    .map(|s| {
                        s.artifacts
                            .iter()
                            .filter_map(|a| a.bin_path.clone())
                            .collect()
                    })
                    .unwrap_or_default();

                let mut installed_artifacts: Vec<InstalledArtifact> = Vec::new();
                let mut locks: Vec<ArtifactLock> = Vec::new();
                for (pick, dl_path, sha) in &downloaded {
                    let dest = pkg_dir.join(&pick.asset.name);
                    crate::utils::file::copy_over(dl_path, &dest).await?;
                    installed_artifacts.push(InstalledArtifact {
                        asset: pick.asset.name.clone(),
                        format: pick.format.to_string(),
                        bin_path: None,
                        installed_by: None,
                        system_package: None,
                    });
                    locks.push(ArtifactLock {
                        version: Some(release.version.clone()),
                        asset: pick.asset.name.clone(),
                        url: pick.asset.url.clone(),
                        format: pick.format.to_string(),
                        selected_by: Some(reason.clone()),
                        sha256: Some((*sha).clone()),
                        ..Default::default()
                    });
                }
                // A line that used to deploy a binary and is now download-only drops the old
                // PATH entry, or it becomes drift nothing declares.
                for stale in &previous {
                    if let Err(e) = crate::utils::remove_deployed_path(stale).await {
                        warn!("{}: could not remove the old `{}`: {}", spec.name, stale, e);
                    }
                }
                info!(
                    "{}: fetched {} artifact(s) for {} — download-only, not on PATH",
                    spec.name,
                    installed_artifacts.len(),
                    release.version
                );
                ledger.record(spec.name.clone(), locks);
                state.insert(
                    spec.name.clone(),
                    GithubState {
                        repo: spec.name.clone(),
                        version: release.version,
                        install_path: pkg_dir.to_string_lossy().to_string(),
                        artifacts: installed_artifacts,
                    },
                );
                continue;
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

            let repo_name = spec.name.split('/').next_back().unwrap_or(&spec.name);
            let bin_dir = dirs::home_dir()
                .ok_or_else(|| Error::Other("Home directory not found".into()))?
                .join(".local")
                .join("bin");
            let previous: Vec<String> = state
                .get(&spec.name)
                .map(|s| {
                    s.artifacts
                        .iter()
                        .filter_map(|a| a.bin_path.clone())
                        .collect()
                })
                .unwrap_or_default();

            // D5: a `.deb`/`.rpm` is handed to its system manager, never unpacked or put on
            // PATH; the rest are LiNix's to deploy. Split them so the deploy-naming rule counts
            // only the artifacts that actually take a PATH name.
            let handoff_idx: Vec<usize> = downloaded
                .iter()
                .enumerate()
                .filter(|(_, (pick, _, _))| system_pkg::is_handoff_format(pick.format))
                .map(|(i, _)| i)
                .collect();
            let regular_idx: Vec<usize> = downloaded
                .iter()
                .enumerate()
                .filter(|(_, (pick, _, _))| !system_pkg::is_handoff_format(pick.format))
                .map(|(i, _)| i)
                .collect();
            let one_artifact = regular_idx.len() == 1;

            // Unpack and find each program first, deploy nothing yet: the name two artifacts
            // fight over is only knowable once both archives are open, and a refusal that
            // arrives after the first is already on PATH has half-installed the line it
            // refused. Handoffs to a system manager (below) run only after this pass is clean,
            // for the same reason: a `dpkg -i` that ran before a sibling clash was found would
            // leave the line half-installed.
            let mut resolved: Vec<(&artifact::Pick, &String, PathBuf, PathBuf)> = Vec::new();
            for &i in &regular_idx {
                let (pick, dl_path, sha) = &downloaded[i];
                // One subdirectory per artifact: two archives under one declaration can both
                // contain `bin/`, and unpacking them over each other loses one of them.
                let unpack_dir = pkg_dir.join(artifact_dir_name(&pick.asset.name));
                tokio::fs::create_dir_all(&unpack_dir)
                    .await
                    .map_err(Error::from)?;

                let dl_path_archive = dl_path.clone();
                let unpack_archive = unpack_dir.clone();
                tokio::task::spawn_blocking(move || {
                    extract_archive(&dl_path_archive, &unpack_archive)
                })
                .await
                .map_err(|e| Error::Other(e.to_string()))??;

                let walk_dir = unpack_dir.clone();
                let listing: Vec<ArchiveEntry> = tokio::task::spawn_blocking(move || {
                    walkdir::WalkDir::new(&walk_dir)
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

                let discovered =
                    artifact::find_executable(&listing, &spec.name, wanted.bin.as_deref())
                        .map_err(|e| Error::PackageNotFound(e.to_string()))?;

                // One artifact is deployed under the repo's name, as it always has been.
                // Several cannot be — so each keeps the name of the program inside it, and
                // two that would land on the same name is an error rather than one silently
                // overwriting the other (owner ruling, 2026-07-21).
                let deploy_name = if one_artifact {
                    repo_name.to_string()
                } else {
                    discovered
                        .file_name()
                        .and_then(|n| n.to_str())
                        .ok_or_else(|| {
                            Error::Other(format!(
                                "{}: the executable inside `{}` has no usable filename",
                                spec.name, pick.asset.name
                            ))
                        })?
                        .to_string()
                };
                let bin_dest =
                    crate::utils::bin_destination(&bin_dir, &deploy_name, self.core.confine_bin)?;
                if let Some((clash, _, _, _)) =
                    resolved.iter().find(|(_, _, _, dest)| dest == &bin_dest)
                {
                    return Err(Error::Validation(format!(
                        "{}: both `{}` and `{}` install a program called `{}`. Narrow \
                         `@asset=all` with a pattern, e.g. @asset=*musl*, so one file answers \
                         the line.",
                        spec.name, clash.asset.name, pick.asset.name, deploy_name
                    )));
                }
                resolved.push((pick, sha, discovered, bin_dest));
            }

            // D14: the same rule chose every pick of this declaration (they share the line's
            // formats and pattern), so the reason is computed once and recorded on each lock.
            let reason =
                artifact::selection_reason(wanted.asset.is_some(), formats.is_user_specified())
                    .to_string();

            let mut installed_artifacts: Vec<InstalledArtifact> = Vec::new();
            let mut locks: Vec<ArtifactLock> = Vec::new();

            // D5: hand each `.deb`/`.rpm` to its manager. The manager now owns the files — LiNix
            // records only *which* manager and the name it listed the package under, and removal,
            // upgrade and dedup route through that record. The name is read from the file before
            // the install, because after the install the file is gone and the name is the only
            // handle removal has.
            for &i in &handoff_idx {
                let (pick, dl_path, sha) = &downloaded[i];
                let installer = system_pkg::installer_for(pick.format).ok_or_else(|| {
                    Error::Other(format!(
                        "{}: no system installer for {}",
                        spec.name, pick.format
                    ))
                })?;
                let query = system_pkg::query_name_argv(pick.format, dl_path)?;
                let (qprog, qargs) = query.split_first().expect("a query argv is never empty");
                let qrefs: Vec<&str> = qargs.iter().map(String::as_str).collect();
                let system_package = self
                    .core
                    .executor
                    .run_output(qprog, &qrefs, false)
                    .await?
                    .trim()
                    .to_string();

                let install = system_pkg::install_argv(pick.format, dl_path)?;
                let (iprog, iargs) = install
                    .split_first()
                    .expect("an install argv is never empty");
                let irefs: Vec<&str> = iargs.iter().map(String::as_str).collect();
                info!(
                    "{}: handing {} to {} — installs as `{}`",
                    spec.name, pick.asset.name, installer, system_package
                );
                self.core.executor.run(iprog, &irefs, true).await?;

                installed_artifacts.push(InstalledArtifact {
                    asset: pick.asset.name.clone(),
                    format: pick.format.to_string(),
                    bin_path: None,
                    installed_by: Some(installer.to_string()),
                    system_package: Some(system_package.clone()),
                });
                locks.push(ArtifactLock {
                    version: Some(release.version.clone()),
                    asset: pick.asset.name.clone(),
                    url: pick.asset.url.clone(),
                    format: pick.format.to_string(),
                    selected_by: Some(reason.clone()),
                    sha256: Some((*sha).clone()),
                    installed_by: Some(installer.to_string()),
                    system_package: Some(system_package),
                });
            }

            for (pick, sha, discovered, bin_dest) in &resolved {
                crate::utils::deploy_executable(
                    discovered,
                    bin_dest,
                    &self.core.install_dir,
                    previous
                        .iter()
                        .find(|p| *p == &bin_dest.to_string_lossy())
                        .map(|s| s.as_str()),
                )
                .await?;

                installed_artifacts.push(InstalledArtifact {
                    asset: pick.asset.name.clone(),
                    format: pick.format.to_string(),
                    bin_path: Some(bin_dest.to_string_lossy().to_string()),
                    installed_by: None,
                    system_package: None,
                });
                locks.push(ArtifactLock {
                    version: Some(release.version.clone()),
                    asset: pick.asset.name.clone(),
                    url: pick.asset.url.clone(),
                    format: pick.format.to_string(),
                    selected_by: Some(reason.clone()),
                    sha256: Some((*sha).clone()),
                    ..Default::default()
                });
            }

            // A declaration that used to deploy a name it no longer deploys leaves that file
            // on PATH, where nothing declares it and no `sync` can see it.
            for stale in previous.iter().filter(|p| {
                !installed_artifacts
                    .iter()
                    .any(|a| a.bin_path.as_ref() == Some(*p))
            }) {
                if let Err(e) = crate::utils::remove_deployed_path(stale).await {
                    warn!("{}: could not remove the old `{}`: {}", spec.name, stale, e);
                }
            }

            ledger.record(spec.name.clone(), locks);
            state.insert(
                spec.name.clone(),
                GithubState {
                    repo: spec.name.clone(),
                    version: release.version,
                    install_path: pkg_dir.to_string_lossy().to_string(),
                    artifacts: installed_artifacts,
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
                // D5: an artifact a system manager owns is removed *through* that manager, by the
                // name it was recorded under — a file delete would leave the package in apt/rpm's
                // database, drift no `sync` could see.
                for art in &pkg.artifacts {
                    if let (Some(installer), Some(system_package)) =
                        (art.installed_by.as_deref(), art.system_package.as_deref())
                    {
                        match system_pkg::remove_argv(installer, system_package) {
                            Ok(argv) => {
                                let (prog, args) =
                                    argv.split_first().expect("a remove argv is never empty");
                                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                                if let Err(e) = self.core.executor.run(prog, &refs, true).await {
                                    errors.push(format!(
                                        "{} -e {}: {}",
                                        installer, system_package, e
                                    ));
                                }
                            }
                            Err(e) => errors.push(e.to_string()),
                        }
                    }
                }
                for bp in pkg.artifacts.iter().filter_map(|a| a.bin_path.as_ref()) {
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
                    if self.core.clean_cache_on_remove {
                        for asset in pkg.artifacts.iter().map(|a| a.asset.as_str()) {
                            crate::model::cache::clean_cached(asset, &self.core.cache_dirs).await;
                        }
                    }
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

    async fn owned_system_packages(&self) -> Vec<(String, String)> {
        // D5: read the ledger LiNix wrote, not the network — a `.deb` this backend handed to
        // dpkg is recorded there as `installed_by`/`system_package`.
        ArtifactLedger::load(&self.core.locks_file)
            .map(|l| l.system_packages())
            .unwrap_or_default()
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
        cfg.guard.confine_bin,
        cfg.clean_cache_on_remove,
        cfg.cache_dirs.clone(),
        // A secret is the environment only, never a file (II.1) — `preferences.toml` is
        // committed to the repo it lives in, so a token key there is a token in git.
        std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()),
        Duration::from_secs(cfg.rate_limit_max_wait_secs),
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

    // S26. The old handler slept until the reset — up to an hour, holding the data lock —
    // and then returned the same 403 it had slept on, so the wait bought nothing.
    #[test]
    fn a_reset_inside_the_ceiling_is_waited_out_and_retried() {
        assert_eq!(
            rate_limit_action(403, Some("1000"), 980, 30),
            RateLimit::WaitThenRetry(21)
        );
    }

    #[test]
    fn a_reset_past_the_ceiling_is_refused_with_the_wait_it_would_have_taken() {
        assert_eq!(
            rate_limit_action(403, Some("4600"), 1000, 30),
            RateLimit::TooLong(3601)
        );
    }

    #[test]
    fn the_ceiling_is_the_only_thing_that_decides_between_them() {
        // The same response, one machine willing to wait an hour and one not.
        assert_eq!(
            rate_limit_action(403, Some("4600"), 1000, 7200),
            RateLimit::WaitThenRetry(3601)
        );
    }

    #[test]
    fn a_403_that_is_not_a_rate_limit_is_never_waited_on() {
        // A bad token or a private repo: no header, and no amount of waiting changes it.
        assert_eq!(
            rate_limit_action(403, None, 1000, 30),
            RateLimit::NotLimited
        );
        // A header that is not a number is not a reset time either.
        assert_eq!(
            rate_limit_action(403, Some("soon"), 1000, 30),
            RateLimit::NotLimited
        );
        // And a limit that has already reset is over, whatever the header says.
        assert_eq!(
            rate_limit_action(403, Some("999"), 1000, 30),
            RateLimit::NotLimited
        );
    }

    #[test]
    fn a_response_that_is_not_a_403_is_left_alone() {
        for status in [200, 404, 429, 500] {
            assert_eq!(
                rate_limit_action(status, Some("99999"), 1000, 30),
                RateLimit::NotLimited,
                "status {} was treated as a rate limit",
                status
            );
        }
    }

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
            selected_by: None,
            sha256: Some("abc123".into()),
            ..Default::default()
        }
    }

    fn installed(version: &str, assets: &[&str]) -> GithubState {
        GithubState {
            repo: "sharkdp/fd".into(),
            version: version.to_string(),
            install_path: "/opt/linix/sharkdp_fd".into(),
            artifacts: assets
                .iter()
                .map(|a| InstalledArtifact {
                    asset: (*a).to_string(),
                    format: "tarball".into(),
                    bin_path: Some(format!("/home/u/.local/bin/{}", a)),
                    installed_by: None,
                    system_package: None,
                })
                .collect(),
        }
    }

    fn tarballs() -> FormatOrder {
        FormatOrder::new(vec![Format::Tarball, Format::Binary])
    }

    #[test]
    fn a_deb_is_installable_only_where_dpkg_exists() {
        // D5: on a machine with the manager the file is a handoff install; without it, the
        // format is not installable here, so a line offering only a `.deb` becomes download-only
        // rather than failing.
        assert!(installable_here(Format::Deb, true, false));
        assert!(!installable_here(Format::Deb, false, false));
        assert!(installable_here(Format::Rpm, false, true));
        assert!(!installable_here(Format::Rpm, false, false));
        // Archives never depend on a system installer.
        assert!(installable_here(Format::Tarball, false, false));
        assert!(installable_here(Format::Binary, false, false));
        // A macOS/Windows database shape LiNix does not hand off is not installable via this path.
        assert!(!installable_here(Format::Msi, true, true));
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
        let locked = [lock("v10.2.0", "fd.tar.gz", "tarball")];
        let state = installed("v10.2.0", &["fd.tar.gz"]);
        let known = Known {
            pin: Some("10.2.0"),
            locked: &locked,
            installed: Some(&state),
        };
        assert!(answered_locally(&known, &tarballs(), None));
    }

    #[test]
    fn an_unpinned_line_always_asks_github() {
        let locked = [lock("v10.2.0", "fd.tar.gz", "tarball")];
        let state = installed("v10.2.0", &["fd.tar.gz"]);
        let known = Known {
            pin: None,
            locked: &locked,
            installed: Some(&state),
        };
        assert!(!answered_locally(&known, &tarballs(), None));
    }

    #[test]
    fn a_pin_that_moved_past_the_lock_asks_github() {
        let locked = [lock("v10.2.0", "fd.tar.gz", "tarball")];
        let state = installed("v10.2.0", &["fd.tar.gz"]);
        let known = Known {
            pin: Some("10.3.0"),
            locked: &locked,
            installed: Some(&state),
        };
        assert!(!answered_locally(&known, &tarballs(), None));
    }

    #[test]
    fn a_lock_without_an_install_asks_github() {
        let locked = [lock("v10.2.0", "fd.tar.gz", "tarball")];
        let known = Known {
            pin: Some("10.2.0"),
            locked: &locked,
            installed: None,
        };
        assert!(!answered_locally(&known, &tarballs(), None));
    }

    #[test]
    fn an_install_that_drifted_from_the_lock_asks_github() {
        let locked = [lock("v10.2.0", "fd-gnu.tar.gz", "tarball")];
        let state = installed("v10.2.0", &["fd-musl.tar.gz"]);
        let known = Known {
            pin: Some("10.2.0"),
            locked: &locked,
            installed: Some(&state),
        };
        assert!(!answered_locally(&known, &tarballs(), None));
    }

    #[test]
    fn changing_formats_under_a_pin_asks_github_again() {
        let locked = [lock("v10.2.0", "fd.tar.gz", "tarball")];
        let state = installed("v10.2.0", &["fd.tar.gz"]);
        let known = Known {
            pin: Some("10.2.0"),
            locked: &locked,
            installed: Some(&state),
        };
        assert!(!answered_locally(
            &known,
            &FormatOrder::new(vec![Format::Deb]),
            None
        ));
    }

    #[test]
    fn a_pinned_set_of_several_is_answered_without_the_network() {
        // `@asset=all` locks every file it installed; all of them present is the answer.
        let locked = [
            lock("v10.2.0", "fd.tar.gz", "tarball"),
            lock("v10.2.0", "fd-server.tar.gz", "tarball"),
        ];
        let state = installed("v10.2.0", &["fd-server.tar.gz", "fd.tar.gz"]);
        let known = Known {
            pin: Some("10.2.0"),
            locked: &locked,
            installed: Some(&state),
        };
        assert!(answered_locally(
            &known,
            &tarballs(),
            Some(&AssetPattern::parse("all").unwrap())
        ));
    }

    #[test]
    fn one_of_a_locked_set_missing_from_disk_asks_github_again() {
        let locked = [
            lock("v10.2.0", "fd.tar.gz", "tarball"),
            lock("v10.2.0", "fd-server.tar.gz", "tarball"),
        ];
        let state = installed("v10.2.0", &["fd.tar.gz"]);
        let known = Known {
            pin: Some("10.2.0"),
            locked: &locked,
            installed: Some(&state),
        };
        assert!(!answered_locally(
            &known,
            &tarballs(),
            Some(&AssetPattern::parse("all").unwrap())
        ));
    }

    #[test]
    fn two_assets_unpack_into_two_directories() {
        // Both archives can contain `bin/`, and one tree would lose one of them.
        assert_ne!(
            artifact_dir_name("fd-x86_64-musl.tar.gz"),
            artifact_dir_name("fd-x86_64-gnu.tar.gz")
        );
        assert!(!artifact_dir_name("../escape.tar.gz").contains('/'));
        assert!(!artifact_dir_name("..\\escape.tar.gz").contains('\\'));
    }

    #[test]
    fn changing_the_asset_pattern_under_a_pin_asks_github_again() {
        let locked = [lock("v10.2.0", "fd-gnu.tar.gz", "tarball")];
        let state = installed("v10.2.0", &["fd-gnu.tar.gz"]);
        let known = Known {
            pin: Some("10.2.0"),
            locked: &locked,
            installed: Some(&state),
        };
        let musl = AssetPattern::parse("*musl*").unwrap();
        assert!(!answered_locally(&known, &tarballs(), Some(&musl)));
        let gnu = AssetPattern::parse("*gnu*").unwrap();
        assert!(answered_locally(&known, &tarballs(), Some(&gnu)));
    }
}
