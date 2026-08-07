use crate::backends::artifact::{system_pkg, Format};
use crate::core::{
    security::verify_checksum, BackendCore, CommandExecutor, Error, Installable, MetadataProvider,
    Package, PackageSpec, Queryable, Result,
};
use crate::utils::archive::extract_archive;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebState {
    url: String,
    local_path: String,
    bin_link: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
    /// The system manager that owns this resource (D5), when the URL pointed at a `.deb`/`.rpm`
    /// that was handed to `dpkg`/`rpm`. `None` is the ordinary web resource LiNix unpacked or put
    /// on PATH itself; when set, removal and dedup route through this manager.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    installed_by: Option<String>,
    /// The name that manager listed it under — what removal and dedup key on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system_package: Option<String>,
}

pub struct WebBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    /// `[guard] confine_bin`: whether an `@bin=` value may name a file outside `bin_dir`
    /// (SEC1). Carried here because the backend is where the value becomes a path.
    pub confine_bin: bool,
    /// K4: also clean the fetched file from the cache locations on removal.
    pub clean_cache_on_remove: bool,
    pub cache_dirs: Vec<PathBuf>,
    pub install_dir: PathBuf,
    /// Where the executable is deployed — `[bin_dir]`, the same directory the shims use and the
    /// one a sandboxed config moves (2026-07-29; it was built from `dirs::home_dir()` here).
    pub bin_dir: PathBuf,
    pub state_file: PathBuf,
    pub internal_lock: Mutex<()>,
}

impl WebBackendCore {
    pub fn new(
        executor: CommandExecutor,
        install_dir: PathBuf,
        bin_dir: PathBuf,
        confine_bin: bool,
        clean_cache_on_remove: bool,
        cache_dirs: Vec<PathBuf>,
    ) -> Self {
        let state_file = install_dir.join("installed.json");
        Self {
            executor,
            name: "web".to_string(),
            confine_bin,
            clean_cache_on_remove,
            cache_dirs,
            install_dir,
            bin_dir,
            state_file,
            internal_lock: Mutex::new(()),
        }
    }

    async fn load_state(&self) -> HashMap<String, WebState> {
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

    async fn save_state(&self, state: &HashMap<String, WebState>) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        let data = serde_json::to_string_pretty(state).map_err(Error::from)?;
        crate::utils::file::persist(&self.state_file, &data).map(|_| ())
    }
}

#[async_trait]
impl BackendCore for WebBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        true
    }
    fn probes(&self) -> Vec<String> {
        Vec::new()
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for WebBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct WebInstallable {
    pub core: Arc<WebBackendCore>,
}

#[async_trait]
impl Installable for WebInstallable {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let mut state = self.core.load_state().await;

        for spec in specs {
            // SEC2: checked before a byte is fetched, so a refusal costs nothing and cannot
            // leave a half-downloaded file behind.
            let allow_http = crate::core::download::allows_http(spec);
            crate::core::download::check_scheme(&spec.name, allow_http, &spec.name)?;
            crate::core::download::check_checksum_declared(spec)?;
            let client = crate::core::download::client(allow_http, "linix-manager")?;

            let head_res = client.head(&spec.name).send().await.map_err(Error::from)?;
            let remote_etag = head_res
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok().map(|s| s.to_string()));
            let remote_mod = head_res
                .headers()
                .get("last-modified")
                .and_then(|v| v.to_str().ok().map(|s| s.to_string()));

            if let Some(existing) = state.get(&spec.name) {
                if (remote_etag.is_some() && remote_etag == existing.etag)
                    || (remote_mod.is_some() && remote_mod == existing.last_modified)
                {
                    debug!("Web: {} is up to date, skipping download.", spec.name);
                    continue;
                }
            }

            // Q37: the PATH name is derived from the URL and from nothing inside the file, so
            // the deploy refusal is answerable before the transfer rather than after it. The
            // `web:` twin of the `github:` ordering that spent 180s on a rejected artifact.
            let url_name = spec.name.split('/').next_back().unwrap_or("resource");
            let deploys = !crate::backends::artifact::ArtifactOptions::read(&spec.options)
                .map(|o| o.download_only)
                .unwrap_or(false);
            if deploys {
                let bin_dest = crate::utils::bin_destination(
                    &self.core.bin_dir,
                    crate::utils::strip_archive_suffixes(url_name),
                    self.core.confine_bin,
                )?;
                crate::utils::ensure_deployable(
                    &bin_dest,
                    &self.core.install_dir,
                    state.get(&spec.name).and_then(|s| s.bin_link.as_deref()),
                )
                .await?;
            }

            info!("Web: Downloading resource: {}", spec.name);
            let response = client.get(&spec.name).send().await.map_err(Error::from)?;
            let bytes = response.bytes().await.map_err(Error::from)?;

            let tmp_dir = tempfile::tempdir().map_err(Error::from)?;
            let dl_path = tmp_dir.path().join("downloaded_file");
            tokio::fs::write(&dl_path, bytes)
                .await
                .map_err(Error::from)?;

            if let Some(expected_sha) = spec.options.get("sha256") {
                verify_checksum(&dl_path, expected_sha).await?;
            }

            // D5: a URL that points at a `.deb`/`.rpm` installs itself into a system database.
            // Hand it to its manager, which then owns it — record only which manager and the
            // name it listed the package under, and skip the unpack/PATH path entirely. On a
            // machine without the manager it falls through and is kept as a plain resource.
            let url_filename = spec.name.split('/').next_back().unwrap_or("");
            let handoff =
                Format::of_filename(url_filename).filter(|f| system_pkg::is_handoff_format(*f));
            if let Some(format) = handoff {
                let detect = system_pkg::detect_command(format).unwrap_or("");
                if self.core.executor.command_exists(detect).await {
                    let installer = system_pkg::installer_for(format).unwrap_or(detect);
                    let query = system_pkg::query_name_argv(format, &dl_path)?;
                    let (qprog, qargs) = query.split_first().expect("a query argv is never empty");
                    let qrefs: Vec<&str> = qargs.iter().map(String::as_str).collect();
                    let system_package = self
                        .core
                        .executor
                        .run_output(qprog, &qrefs, false)
                        .await?
                        .trim()
                        .to_string();

                    let install = system_pkg::install_argv(format, &dl_path)?;
                    let (iprog, iargs) = install
                        .split_first()
                        .expect("an install argv is never empty");
                    let irefs: Vec<&str> = iargs.iter().map(String::as_str).collect();
                    info!(
                        "Web: handing {} to {} — installs as `{}`",
                        url_filename, installer, system_package
                    );
                    self.core.executor.run(iprog, &irefs, true).await?;

                    state.insert(
                        spec.name.clone(),
                        WebState {
                            url: spec.name.clone(),
                            // No local tree LiNix owns: the manager placed the files.
                            local_path: String::new(),
                            bin_link: None,
                            etag: remote_etag,
                            last_modified: remote_mod,
                            installed_by: Some(installer.to_string()),
                            system_package: Some(system_package),
                        },
                    );
                    continue;
                }
            }

            let id = format!("{:x}", md5::compute(&spec.name));
            let dest_dir = self.core.install_dir.join(&id);
            if dest_dir.exists() {
                tokio::fs::remove_dir_all(&dest_dir)
                    .await
                    .map_err(Error::from)?;
            }
            tokio::fs::create_dir_all(&dest_dir)
                .await
                .map_err(Error::from)?;

            let filename = spec.name.split('/').next_back().unwrap_or("resource");
            // The vocabulary, not a fifth hand-written list. This one was matched with
            // `.contains()` rather than `ends_with`, so `notes.gz.txt` was an archive and
            // `report.tar.summary` was one too — and three of its six entries (`.tar`, `.gz`,
            // `.xz`, `.bz2` bare) named things `extract_archive` could not open, which meant a
            // silent `fs::copy` reported as a successful deploy.
            let is_archive = crate::backends::artifact::format::Format::of_filename(filename)
                .is_some_and(|f| f.is_archive());

            if is_archive {
                let dl_path_archive = dl_path.clone();
                let dest_dir_archive = dest_dir.clone();
                tokio::task::spawn_blocking(move || {
                    extract_archive(&dl_path_archive, &dest_dir_archive)
                })
                .await
                .map_err(|e| Error::Other(e.to_string()))??;
            } else {
                crate::utils::file::copy_over(&dl_path, &dest_dir.join(filename)).await?;
            }

            // D3b: `@download_only` fetches the file and stops. And a bare `web:` line that
            // resolves to no runnable program keeps the download rather than failing — the
            // default download-only fallback is simply "no binary was found to deploy" here,
            // because the discovery below records `None` when it finds nothing.
            let download_only = crate::backends::artifact::ArtifactOptions::read(&spec.options)
                .map(|o| o.download_only)
                .unwrap_or(false);

            let mut final_bin_link = None;
            if !download_only {
                // The name comes from the URL, not from an option: `@bin` is refused on
                // `web` (it picks between several files of one release, and a `web:` URL names
                // exactly one). Reading it here was the SEC1 traversal's entry point, and a
                // dead branch besides.
                // Cut at the first `.` and `ripgrep-14.1.0-x86_64.tar.gz` installs a binary
                // called `ripgrep-14`. Only a known archive/package suffix comes off, and
                // repeatedly, so `.tar.gz` goes but a dotted version stays.
                let bin_name = crate::utils::strip_archive_suffixes(filename);

                let bin_dir = self.core.bin_dir.clone();
                let bin_dest =
                    crate::utils::bin_destination(&bin_dir, bin_name, self.core.confine_bin)?;

                let dest_dir_discovery = dest_dir.clone();
                let bin_name_str = bin_name.to_string();

                let bin_src_result: Result<Option<PathBuf>> =
                    tokio::task::spawn_blocking(move || {
                        let mut entries = walkdir::WalkDir::new(&dest_dir_discovery)
                            .into_iter()
                            .filter_map(|e| e.ok());
                        let found = entries
                            .find(|e| {
                                let fname = e.file_name().to_string_lossy().to_lowercase();
                                fname == bin_name_str.to_lowercase()
                                    || fname == format!("{}.exe", bin_name_str.to_lowercase())
                                    || (fname.starts_with(&bin_name_str) && !fname.contains('.'))
                            })
                            .map(|e| e.into_path());
                        Ok(found)
                    })
                    .await
                    .map_err(|e| Error::Other(e.to_string()))?;

                if let Some(src_path) = bin_src_result? {
                    crate::utils::deploy_executable(
                        &src_path,
                        &bin_dest,
                        &self.core.install_dir,
                        state.get(&spec.name).and_then(|s| s.bin_link.as_deref()),
                    )
                    .await?;

                    final_bin_link = Some(bin_dest.to_string_lossy().to_string());
                }
            }

            state.insert(
                spec.name.clone(),
                WebState {
                    url: spec.name.clone(),
                    local_path: dest_dir.to_string_lossy().to_string(),
                    bin_link: final_bin_link,
                    etag: remote_etag,
                    last_modified: remote_mod,
                    installed_by: None,
                    system_package: None,
                },
            );
        }

        self.core.save_state(&state).await?;
        Ok(())
    }

    async fn remove(&self, urls: &[String], _: bool, _reaped: crate::app::sync::guard::Reaped) -> Result<()> {
        let mut state = self.core.load_state().await;
        let mut failures = Vec::new();
        for url in urls {
            if let Some(entry) = state.remove(url) {
                let mut errors = Vec::new();
                // D5: a resource a system manager owns is removed *through* that manager.
                if let (Some(installer), Some(system_package)) = (
                    entry.installed_by.as_deref(),
                    entry.system_package.as_deref(),
                ) {
                    match system_pkg::remove_argv(installer, system_package) {
                        Ok(argv) => {
                            let (prog, args) =
                                argv.split_first().expect("a remove argv is never empty");
                            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                            if let Err(e) = self.core.executor.run(prog, &refs, true).await {
                                errors.push(format!("{} {}: {}", installer, system_package, e));
                            }
                        }
                        Err(e) => errors.push(e.to_string()),
                    }
                }
                if let Some(ref l) = entry.bin_link {
                    if let Err(e) = crate::utils::remove_deployed_path(l).await {
                        errors.push(e);
                    }
                }
                if !entry.local_path.is_empty() {
                    if let Err(e) = crate::utils::remove_deployed_path(&entry.local_path).await {
                        errors.push(e);
                    }
                }
                if errors.is_empty() {
                    info!("Web: Removed resource: {}", url);
                    if self.core.clean_cache_on_remove {
                        let basename = url.split('/').next_back().unwrap_or("");
                        crate::model::cache::clean_cached(basename, &self.core.cache_dirs).await;
                    }
                } else {
                    // The file is still on disk and still on PATH. Dropping it from state
                    // anyway would make it drift no `sync` can see, so put the record back.
                    state.insert(url.clone(), entry);
                    failures.push(format!("{}: {}", url, errors.join("; ")));
                }
            }
        }
        self.core.save_state(&state).await?;
        if !failures.is_empty() {
            return Err(Error::Other(format!(
                "could not remove {} web resource(s), still on disk: {}",
                failures.len(),
                failures.join(", ")
            )));
        }
        Ok(())
    }
}

pub struct WebQueryable {
    pub core: Arc<WebBackendCore>,
}

#[async_trait]
impl Queryable for WebQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let state = self.core.load_state().await;
        Ok(state.keys().map(|u| Package::new(u, "web")).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }

    async fn owned_system_packages(&self) -> Vec<(String, String)> {
        // D5: report the `.deb`/`.rpm` resources this backend handed to a system manager, so the
        // unmanaged crawl defers to it.
        self.core
            .load_state()
            .await
            .values()
            .filter_map(|s| match (&s.installed_by, &s.system_package) {
                (Some(installer), Some(pkg)) => Some((installer.clone(), pkg.clone())),
                _ => None,
            })
            .collect()
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let core = Arc::new(WebBackendCore::new(
        exec.duplicate(),
        cfg.web_dir.clone(),
        cfg.bin_dir.clone(),
        cfg.guard.confine_bin,
        cfg.clean_cache_on_remove,
        cfg.cache_dirs.clone(),
    ));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(WebInstallable { core: core.clone() }))
            .with_queryable(Arc::new(WebQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::sync::guard::{GuardScope, Reaped};
    use crate::core::executor::MockExecutor;
    use crate::core::executor::DryRunOutput;
    use dashmap::DashMap;

    /// A `web:` backend over a temporary tree, with a mock in front of every command.
    ///
    /// `web.rs` had **no tests at all** — a backend that runs `dpkg -i` and `rpm -e` as root
    /// (`run(prog, args, true)`, the `true` being sudo) and whose removal path decides whether a
    /// record stays or goes. The install half needs a live HTTP server and is the real machine's
    /// job; everything below is local and was simply never asked about.
    fn backend(tag: &str) -> (WebInstallable, Arc<MockExecutor>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let vfs: Arc<DashMap<std::path::PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(DashMap::new()),
        );
        let core = Arc::new(WebBackendCore::new(
            exec,
            tmp.path().join(tag),
            tmp.path().join("bin"),
            true,
            false,
            Vec::new(),
        ));
        (WebInstallable { core }, mock, tmp)
    }

    async fn record(core: &WebBackendCore, url: &str, entry: WebState) {
        let mut state = core.load_state().await;
        state.insert(url.to_string(), entry);
        core.save_state(&state).await.expect("writing the state");
    }

    fn handed_to(installer: &str, package: &str, url: &str) -> WebState {
        WebState {
            url: url.to_string(),
            // A handoff owns no tree of LiNix's — the manager placed the files.
            local_path: String::new(),
            bin_link: None,
            etag: None,
            last_modified: None,
            installed_by: Some(installer.to_string()),
            system_package: Some(package.to_string()),
        }
    }

    fn reaped() -> Reaped {
        Reaped::for_reason(
            GuardScope::Remove,
            "a unit test for the effector, which is not a test of the guard",
        )
    }

    /// D5: a `.deb` LiNix handed to `dpkg` is removed **through `dpkg`**, by the package name
    /// read out of the file at install time — not by the URL, and not by deleting a tree LiNix
    /// does not own.
    #[tokio::test]
    async fn a_resource_a_system_manager_owns_is_removed_through_that_manager() {
        let (web, mock, _tmp) = backend("deb");
        let url = "https://example.invalid/fd_10.2.0_amd64.deb";
        record(&web.core, url, handed_to("dpkg", "fd", url)).await;

        mock.set_response("dpkg -r fd", Ok(DryRunOutput::new().into()));
        web.remove(&[url.to_string()], false, reaped())
            .await
            .expect("the removal succeeds");

        let calls = mock.get_calls().await;
        assert!(
            calls.iter().any(|c| c.contains("dpkg -r fd")),
            "the removal did not go through dpkg: {calls:?}"
        );
        assert!(
            !calls.iter().any(|c| c.contains("fd_10.2.0_amd64.deb")),
            "removal named the file rather than the package dpkg lists it under: {calls:?}"
        );
        assert!(
            !web.core.load_state().await.contains_key(url),
            "the record survived a successful removal"
        );
    }

    /// The `rpm` half of the same rule. Written because `install_argv` deliberately differs
    /// between the two (`dpkg -i` vs `rpm -U --replacepkgs`), and a test on only one of them
    /// would pin the pair's shape while leaving the divergent one unasked.
    #[tokio::test]
    async fn the_rpm_twin_removes_by_name_too() {
        let (web, mock, _tmp) = backend("rpm");
        let url = "https://example.invalid/fd-10.2.0.x86_64.rpm";
        record(&web.core, url, handed_to("rpm", "fd", url)).await;

        mock.set_response("rpm -e fd", Ok(DryRunOutput::new().into()));
        web.remove(&[url.to_string()], false, reaped())
            .await
            .expect("the removal succeeds");

        let calls = mock.get_calls().await;
        assert!(
            calls.iter().any(|c| c.contains("rpm -e fd")),
            "the removal did not go through rpm: {calls:?}"
        );
    }

    /// **The invariant that matters most here, and the one nothing was checking.**
    ///
    /// When the system manager refuses, the files are still on disk and still on PATH. Dropping
    /// the record anyway would make the resource drift *no sync can see*: LiNix would have
    /// forgotten it, so nothing would ever try again and nothing would report it. The record
    /// goes back, and the call returns an error naming what is still installed.
    #[tokio::test]
    async fn a_failed_removal_keeps_the_record_rather_than_forgetting_the_resource() {
        let (web, mock, _tmp) = backend("stuck");
        let url = "https://example.invalid/fd_10.2.0_amd64.deb";
        record(&web.core, url, handed_to("dpkg", "fd", url)).await;

        mock.set_response(
            "dpkg -r fd",
            Err(Error::Other("dpkg: dependency problems".into())),
        );
        let err = web
            .remove(&[url.to_string()], false, reaped())
            .await
            .expect_err("a manager that refuses must not read as a removal");
        assert!(
            err.to_string().contains("still on disk"),
            "the error does not say the resource is still installed: {err}"
        );
        assert!(
            web.core.load_state().await.contains_key(url),
            "the record was dropped for a resource that is still installed — the one state no \
             sync can detect"
        );
    }

    /// A recorded installer LiNix has no removal argv for is an error, not a silent skip. The
    /// same shape as the failing-manager case, reached without running anything: `remove_argv`
    /// refuses first.
    #[tokio::test]
    async fn an_installer_with_no_known_removal_is_reported_not_skipped() {
        let (web, _mock, _tmp) = backend("odd");
        let url = "https://example.invalid/fd.pkg";
        record(&web.core, url, handed_to("brew", "fd", url)).await;

        let err = web
            .remove(&[url.to_string()], false, reaped())
            .await
            .expect_err("an unknown installer must not read as a removal");
        assert!(err.to_string().contains("still on disk"), "{err}");
        assert!(web.core.load_state().await.contains_key(url));
    }

    /// A URL that is not in the state file at all is not an error: `remove` is called with what
    /// the plan asked to remove, and a resource already gone is the end state that was wanted.
    #[tokio::test]
    async fn removing_something_that_was_never_recorded_is_not_a_failure() {
        let (web, _mock, _tmp) = backend("absent");
        web.remove(
            &["https://example.invalid/never-installed.tar.gz".to_string()],
            false,
            reaped(),
        )
        .await
        .expect("an already-absent resource is the end state that was asked for");
    }
}
