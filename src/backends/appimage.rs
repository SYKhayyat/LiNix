use crate::backends::artifact::teardown::{still_installed, tear_down, Deployed};
use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppImageState {
    url: String,
    local_path: String,
    symlink_path: String,
}

pub struct AppImageBackendCore {
    pub executor: CommandExecutor,
    /// `[guard] confine_bin`: whether a link name may reach outside `~/.local/bin` (SEC1).
    pub confine_bin: bool,
    /// K4: also clean the fetched AppImage from the cache locations on removal.
    pub clean_cache_on_remove: bool,
    pub cache_dirs: Vec<PathBuf>,
    pub install_dir: PathBuf,
    /// Where the `.AppImage` is linked from — `[bin_dir]`, the same directory the shims use and
    /// the one a sandboxed config moves (2026-07-29; it was built from `dirs::home_dir()` here).
    pub bin_dir: PathBuf,
    pub state_file: PathBuf,
}

impl AppImageBackendCore {
    pub fn new(
        executor: CommandExecutor,
        install_dir: PathBuf,
        bin_dir: PathBuf,
        confine_bin: bool,
        clean_cache_on_remove: bool,
        cache_dirs: Vec<PathBuf>,
    ) -> Self {
        let state = install_dir.join("state.json");

        Self {
            executor,
            confine_bin,
            clean_cache_on_remove,
            cache_dirs,
            install_dir,
            bin_dir,
            state_file: state,
        }
    }

    async fn ensure_dirs(&self) -> Result<PathBuf> {
        if !tokio::fs::try_exists(&self.install_dir)
            .await
            .unwrap_or(false)
        {
            crate::utils::file::ensure_dir_async(&self.install_dir).await?;
        }
        if !tokio::fs::try_exists(&self.bin_dir).await.unwrap_or(false) {
            crate::utils::file::ensure_dir_async(&self.bin_dir).await?;
        }
        Ok(self.bin_dir.clone())
    }

    async fn load_state(&self) -> HashMap<String, AppImageState> {
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

    async fn save_state(&self, state: &HashMap<String, AppImageState>) -> Result<()> {
        let data = serde_json::to_string_pretty(state).map_err(Error::from)?;
        self.executor.write_atomic(&self.state_file, &data).await
    }
}

#[async_trait]
impl BackendCore for AppImageBackendCore {
    fn name(&self) -> &str {
        "appimage"
    }

    fn is_available(&self) -> bool {
        cfg!(target_os = "linux")
    }
    fn probes(&self) -> Vec<String> {
        Vec::new()
    }

    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for AppImageBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct AppImageInstallable {
    pub core: Arc<AppImageBackendCore>,
}

#[async_trait]
impl Installable for AppImageInstallable {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let bin_dir = self.core.ensure_dirs().await?;
        let mut state = self.core.load_state().await;

        for spec in specs {
            let url = &spec.name;
            // SEC2. This backend had no checksum option at all: it fetched any URL, chmod
            // 0755'd the result and put it on PATH, so `appimage:http://…` was one line
            // between a stranger and your shell.
            let allow_http = crate::core::download::allows_http(spec);
            crate::core::download::check_scheme(url, allow_http, url)?;
            crate::core::download::check_checksum_declared(spec)?;
            let client = crate::core::download::client(allow_http, "shall-manager")?;
            let filename = url.split('/').next_back().unwrap_or("app.AppImage");
            let dest_path = self.core.install_dir.join(filename);

            // Q37: the PATH name comes from the URL, so the refusal below can be asked now —
            // before the network — instead of after a download that was always going to be
            // thrown away. Same question, same message, no bytes.
            let link_name = filename
                .strip_suffix(".AppImage")
                .or_else(|| filename.strip_suffix(".appimage"))
                .unwrap_or(filename);
            let download_only = crate::backends::artifact::ArtifactOptions::read(&spec.options)
                .map(|o| o.download_only)
                .unwrap_or(false);
            let link_path = if download_only {
                None
            } else {
                let link_path =
                    crate::utils::bin_destination(&bin_dir, link_name, self.core.confine_bin)?;
                crate::utils::ensure_deployable(
                    &link_path,
                    &self.core.install_dir,
                    state.get(&spec.name).map(|s| s.symlink_path.as_str()),
                )
                .await?;
                Some(link_path)
            };

            info!("AppImage: Downloading {}...", url);
            let response = client.get(url).send().await?;
            if !response.status().is_success() {
                return Err(Error::Other(format!(
                    "Download failed for {}: {}",
                    url,
                    response.status()
                )));
            }

            crate::core::download::write_capped(response, &dest_path, url).await?;

            // Before the chmod below, never after: an unverified file must never exist as an
            // executable, even briefly.
            if let Some(expected) = spec.options.one("sha256") {
                if let Err(e) = crate::core::verify_checksum(&dest_path, expected).await {
                    let _ = tokio::fs::remove_file(&dest_path).await;
                    return Err(e);
                }
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let metadata = tokio::fs::metadata(&dest_path).await?;
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                tokio::fs::set_permissions(&dest_path, perms).await?;
            }

            // D3b: `@download_only` keeps the fetched AppImage on disk but never links it onto
            // PATH. It is still declared, so a later removal still deletes the file.
            let symlink_path = match &link_path {
                None => String::new(),
                Some(link_path) => {
                    crate::utils::deploy_executable(
                        &dest_path,
                        link_path,
                        &self.core.install_dir,
                        state.get(&spec.name).map(|s| s.symlink_path.as_str()),
                    )
                    .await?;
                    info!(
                        "AppImage: Successfully installed {} to {}",
                        link_name,
                        link_path.display()
                    );
                    link_path.to_string_lossy().to_string()
                }
            };
            if download_only {
                info!(
                    "AppImage: fetched {} (download-only, not on PATH)",
                    filename
                );
            }

            state.insert(
                spec.name.clone(),
                AppImageState {
                    url: url.clone(),
                    local_path: dest_path.to_string_lossy().to_string(),
                    symlink_path,
                },
            );
        }

        self.core.save_state(&state).await?;
        Ok(())
    }

    async fn remove(
        &self,
        names: &[String],
        _: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        let mut state = self.core.load_state().await;

        let mut failures = Vec::new();
        for name in names {
            if let Some(info) = state.remove(name) {
                debug!("AppImage: Removing local files for {}", name);
                // A download-only AppImage was never linked, so `symlink_path` is empty and
                // there is no PATH entry to drop — which `Deployed::path` already knows.
                let deployed = Deployed::default()
                    .path(&info.local_path)
                    .path(&info.symlink_path)
                    .cached_url(&info.url);
                let errors = tear_down(
                    &deployed,
                    &self.core.executor,
                    self.core.clean_cache_on_remove,
                    &self.core.cache_dirs,
                )
                .await;
                if errors.is_empty() {
                    info!("AppImage: Removed {}", name);
                } else {
                    state.insert(name.clone(), info);
                    failures.push(format!("{}: {}", name, errors.join("; ")));
                }
            } else {
                warn!("AppImage: No record found for {}, skipping removal.", name);
            }
        }

        self.core.save_state(&state).await?;
        if !failures.is_empty() {
            return Err(still_installed("AppImage", &failures));
        }
        Ok(())
    }
}

pub struct AppImageQueryable {
    pub core: Arc<AppImageBackendCore>,
}

#[async_trait]
impl Queryable for AppImageQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), "appimage")
    }

    /// **The name is the URL, because the URL is what `install` was handed and what the state
    /// is keyed by.** Reporting the basename instead meant `info(url)` never matched its own
    /// state file: every declared AppImage read as absent, so `sync` re-downloaded all of them
    /// on every run, for ever, and a removal could never find the row it was meant to delete.
    /// Same shape as `btrfs:`, fixed 2026-07-30, and `web:`, which never had it.
    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let state = self.core.load_state().await;
        Ok(state
            .keys()
            .map(|url| Package::new(url, "appimage"))
            .collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.installed_listing().await?;
        Ok(all.iter().find(|p| p.name == name).cloned())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let core = Arc::new(AppImageBackendCore::new(
        exec.clone(),
        cfg.appimage_dir.clone(),
        cfg.bin_dir.clone(),
        cfg.guard.confine_bin,
        cfg.clean_cache_on_remove,
        cfg.cache_dirs.clone(),
    ));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(AppImageInstallable { core: core.clone() }))
            .with_queryable(Arc::new(AppImageQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::sync::guard::{GuardScope, Reaped};
    use crate::core::Installable;

    /// `appimage.rs` had **no tests at all**, and its removal is `web.rs`'s removal with the D5
    /// handoff taken out — same state file, same two deployed paths, same re-insert-on-failure
    /// rule. Testing one of a pair and not the other is how the pair drifts, so both have the
    /// same four questions asked of them now.
    fn backend(tag: &str) -> (AppImageInstallable, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let core = Arc::new(AppImageBackendCore::new(
            CommandExecutor::new(false, false),
            tmp.path().join(tag),
            tmp.path().join("bin"),
            true,
            false,
            Vec::new(),
        ));
        (AppImageInstallable { core }, tmp)
    }

    async fn record(core: &AppImageBackendCore, name: &str, entry: AppImageState) {
        tokio::fs::create_dir_all(&core.install_dir)
            .await
            .expect("the install dir");
        let mut state = core.load_state().await;
        state.insert(name.to_string(), entry);
        core.save_state(&state).await.expect("writing the state");
    }

    fn reaped() -> Reaped {
        Reaped::for_reason(
            GuardScope::Remove,
            "a unit test for the effector, which is not a test of the guard",
        )
    }

    async fn touch(path: &std::path::Path) {
        tokio::fs::create_dir_all(path.parent().expect("a parent"))
            .await
            .expect("the parent dir");
        tokio::fs::write(path, b"ELF").await.expect("the file");
    }

    /// **The name `list` returns is the name `install` was handed.**
    ///
    /// `fetch_installed` reported `url.split('/').next_back()` — the basename — while `install`
    /// keys the state by the whole URL, which is also what the declaration says. So `info(url)`
    /// never matched, every declared AppImage read as absent, and `sync` re-downloaded all of
    /// them on every run for ever. `btrfs:` had the same bug, diagnosed in those words and fixed
    /// on 2026-07-30; `web:`, this backend's twin, never had it.
    #[tokio::test]
    async fn an_appimage_is_listed_by_the_url_install_was_given() {
        let (app, tmp) = backend("img");
        let url = "https://example.invalid/dl/v2.1/fd-v2.1-x86_64.AppImage";
        record(
            &app.core,
            url,
            AppImageState {
                url: url.into(),
                local_path: tmp
                    .path()
                    .join("img")
                    .join("fd-v2.1-x86_64.AppImage")
                    .to_string_lossy()
                    .into(),
                symlink_path: String::new(),
            },
        )
        .await;
        let q = AppImageQueryable {
            core: app.core.clone(),
        };
        let names: Vec<String> = q
            .list_installed()
            .await
            .expect("the listing")
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            names,
            vec![url.to_string()],
            "`install` was handed the URL, so `list` has to say the same string"
        );
        // The consequence, asserted where it bit: the planner asks `info` with the declared name.
        assert!(
            q.info(url).await.expect("the query").is_some(),
            "a declared AppImage read as absent — this is the re-download-for-ever bug"
        );
        assert!(
            q.info("fd-v2.1-x86_64.AppImage")
                .await
                .expect("the query")
                .is_none(),
            "the basename is not a name anything declares, so it must not answer either"
        );
    }

    /// Both deployed paths go, and the record with them.    /// Both deployed paths go, and the record with them.
    #[tokio::test]
    async fn removing_an_appimage_takes_the_file_and_the_path_entry() {
        let (app, tmp) = backend("img");
        let local = tmp.path().join("img").join("fd.AppImage");
        let link = tmp.path().join("bin").join("fd");
        touch(&local).await;
        touch(&link).await;
        record(
            &app.core,
            "fd",
            AppImageState {
                url: "https://example.invalid/fd.AppImage".into(),
                local_path: local.to_string_lossy().into(),
                symlink_path: link.to_string_lossy().into(),
            },
        )
        .await;

        app.remove(&["fd".to_string()], false, reaped())
            .await
            .expect("the removal succeeds");

        assert!(!local.exists(), "the AppImage is still on disk");
        assert!(!link.exists(), "the PATH entry survived the removal");
        assert!(!app.core.load_state().await.contains_key("fd"));
    }

    /// `@download_only` never linked anything, so `symlink_path` is empty — and an empty path is
    /// not a path to delete. The branch exists precisely because treating `""` as a path would
    /// make every download-only removal fail.
    #[tokio::test]
    async fn a_download_only_appimage_has_no_path_entry_to_drop() {
        let (app, tmp) = backend("dl");
        let local = tmp.path().join("dl").join("tool.AppImage");
        touch(&local).await;
        record(
            &app.core,
            "tool",
            AppImageState {
                url: "https://example.invalid/tool.AppImage".into(),
                local_path: local.to_string_lossy().into(),
                symlink_path: String::new(),
            },
        )
        .await;

        app.remove(&["tool".to_string()], false, reaped())
            .await
            .expect("a download-only AppImage removes without a link");
        assert!(!local.exists());
        assert!(app.core.load_state().await.is_empty());
    }

    /// The twin of `web.rs`'s. A path the OS refuses leaves the AppImage installed, so the record
    /// must go back: dropping it would make the resource drift **no sync can see**, because Shall
    /// would have forgotten the only thing that knows it is there.
    ///
    /// The undeletable path is a NUL byte, which every platform's path API refuses with
    /// `InvalidInput` rather than `NotFound` — synthetic, deliberately, because the invariant
    /// under test is what happens *after* a removal fails and not which removals fail.
    #[tokio::test]
    async fn a_failed_removal_keeps_the_record_rather_than_forgetting_the_appimage() {
        let (app, _tmp) = backend("stuck");
        record(
            &app.core,
            "wedged",
            AppImageState {
                url: "https://example.invalid/wedged.AppImage".into(),
                local_path: "no\0such".into(),
                symlink_path: String::new(),
            },
        )
        .await;

        let err = app
            .remove(&["wedged".to_string()], false, reaped())
            .await
            .expect_err("a path that cannot be removed must not read as a removal");
        assert!(
            err.to_string().contains("still installed"),
            "the error does not say the AppImage is still there: {err}"
        );
        assert!(
            app.core.load_state().await.contains_key("wedged"),
            "the record was dropped for an AppImage that is still installed"
        );
    }

    /// A name with no record is a skip, not a failure: `remove` is handed what the plan asked to
    /// remove, and something already gone is the end state that was wanted.
    #[tokio::test]
    async fn removing_something_that_was_never_recorded_is_not_a_failure() {
        let (app, _tmp) = backend("absent");
        app.remove(&["never-installed".to_string()], false, reaped())
            .await
            .expect("an already-absent AppImage is the end state that was asked for");
    }
}
