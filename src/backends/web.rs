use crate::backends::artifact::teardown::{still_installed, tear_down, Deployed};
use crate::backends::artifact::{system_pkg, Format};
use crate::core::{
    security::verify_checksum, BackendCore, CommandExecutor, Error, Installable, MetadataProvider,
    Package, PackageSpec, Queryable, Result,
};
use crate::utils::archive::extract_archive;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    /// that was handed to `dpkg`/`rpm`. `None` is the ordinary web resource Shall unpacked or put
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
    /// The deployment records, in memory, behind the lock that guards the file.
    ///
    /// **The read and the modify used to be two separate critical sections**, 220 lines and a
    /// download apart in `install`. Two `web:` packages in one wave both loaded `{}` and
    /// whichever saved last wrote a map holding only its own record; the one that lost reads
    /// as unmanaged, so the next sync cannot see what it deployed and teardown has nothing to
    /// remove. Identical shape and identical fix in `github.rs` — the family is these two, and
    /// no other backend keeps a private state file.
    state: Mutex<Option<HashMap<String, WebState>>>,
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
            state: Mutex::new(None),
        }
    }

    /// The records as they stand, for reading. A copy: a caller holding a borrow would hold
    /// the lock across its whole install, and the download in the middle of that is the
    /// reason this backend is concurrent at all.
    async fn load_state(&self) -> Result<HashMap<String, WebState>> {
        let mut guard = self.state.lock().await;
        Ok(Self::loaded(&mut guard, &self.state_file).await?.clone())
    }

    /// Read the file into the memo the first time, and hand back the map either way.
    ///
    /// Through `ledger::load_json_records`, which is where the absent-versus-unparseable rule
    /// lives. Reading a corrupt file as an empty map is not a read failure that recovers — the
    /// emptiness is merged and written back, and the record of every deployed artifact is gone.
    async fn loaded<'a>(
        guard: &'a mut Option<HashMap<String, WebState>>,
        state_file: &Path,
    ) -> Result<&'a mut HashMap<String, WebState>> {
        if guard.is_none() {
            *guard = Some(crate::core::ledger::load_json_records(state_file).await?);
        }
        Ok(guard.as_mut().expect("just filled"))
    }

    /// Apply one task's changes to the shared records and write them, under one lock.
    ///
    /// A caller hands over what it changed, not the map it believes the file should hold, so
    /// a concurrent installer's records are merged rather than overwritten. The write goes to
    /// the blocking pool: `persist` ends in `sync_all`, a physical flush, and this runs inside
    /// the install wave where a parked worker costs the whole wave (II.52).
    ///
    /// Compact, not pretty, for the reason `core::state` gives about the registry beside it:
    /// this is machine-read bookkeeping nobody opens, and pretty printing doubles the bytes.
    async fn commit_state(
        &self,
        installed: Vec<(String, WebState)>,
        removed: Vec<String>,
    ) -> Result<()> {
        let mut guard = self.state.lock().await;
        let map = Self::loaded(&mut guard, &self.state_file).await?;
        // **A preview changes the memo as little as it changes the disk.** The merge happens on
        // a copy, and the copy is only adopted for a run that acts. `persist` already refuses
        // the write under `--dry-run` and says "would write"; before this map existed, each
        // call re-read the file, so a preview's changes could not survive to the next question.
        // With a memo they would — and the next `list_installed` in the same run would report a
        // package the preview only said it would install.
        let mut merged = map.clone();
        for url in &removed {
            merged.remove(url);
        }
        merged.extend(installed);
        if !crate::core::dry_run::active() {
            *map = merged.clone();
        }
        let data = serde_json::to_string(&merged).map_err(Error::from)?;
        let state_file = self.state_file.clone();
        crate::core::off_the_runtime(move || {
            crate::utils::file::persist(&state_file, &data).map(|_| ())
        })
        .await?
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
        let mut state = self.core.load_state().await?;
        // What this call changed, kept apart from the copy it reads.
        let mut installed_records: Vec<(String, WebState)> = Vec::new();

        for spec in specs {
            // SEC2: checked before a byte is fetched, so a refusal costs nothing and cannot
            // leave a half-downloaded file behind.
            let allow_http = crate::core::download::allows_http(spec);
            crate::core::download::check_scheme(&spec.name, allow_http, &spec.name)?;
            crate::core::download::check_checksum_declared(spec)?;
            let client = crate::core::download::client(allow_http, "shall-manager")?;

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
            let url_name = crate::utils::file::url_filename(&spec.name)?;
            let deploys = !crate::backends::artifact::ArtifactOptions::read(&spec.options)
                .map(|o| o.download_only)
                .unwrap_or(false);
            if deploys {
                let bin_dest = crate::utils::bin_destination(
                    &self.core.bin_dir,
                    crate::utils::strip_archive_suffixes(&url_name),
                    self.core.confine_bin,
                )?;
                crate::utils::ensure_deployable(
                    &bin_dest,
                    &self.core.install_dir,
                    state.get(&spec.name).and_then(|s| s.bin_link.as_deref()),
                )
                .await?;
            }

            // As in `appimage:`: the deploy refusal above is the half of this a preview can
            // actually answer, and it is answered before this line.
            if crate::core::dry_run::active() {
                crate::would!("download and install {}", spec.name);
                continue;
            }

            info!("Web: Downloading resource: {}", spec.name);
            let response = client.get(&spec.name).send().await.map_err(Error::from)?;

            let tmp_dir = tempfile::tempdir().map_err(Error::from)?;
            let dl_path = tmp_dir.path().join("downloaded_file");
            crate::core::download::write_capped(response, &dl_path, &spec.name).await?;

            if let Some(expected_sha) = spec.options.one("sha256") {
                verify_checksum(&dl_path, expected_sha).await?;
            }

            // D5: a URL that points at a `.deb`/`.rpm` installs itself into a system database.
            // Hand it to its manager, which then owns it — record only which manager and the
            // name it listed the package under, and skip the unpack/PATH path entirely. On a
            // machine without the manager it falls through and is kept as a plain resource.
            let handoff =
                Format::of_filename(&url_name).filter(|f| system_pkg::is_handoff_format(*f));
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
                        url_name, installer, system_package
                    );
                    self.core.executor.run(iprog, &irefs, true).await?;

                    let record = WebState {
                        url: spec.name.clone(),
                        // No local tree Shall owns: the manager placed the files.
                        local_path: String::new(),
                        bin_link: None,
                        etag: remote_etag,
                        last_modified: remote_mod,
                        installed_by: Some(installer.to_string()),
                        system_package: Some(system_package),
                    };
                    installed_records.push((spec.name.clone(), record.clone()));
                    state.insert(spec.name.clone(), record);
                    continue;
                }
            }

            // A directory name derived from the URL, so two downloads cannot collide. `sha2`
            // rather than `md5`: the crate was already a dependency for checksum verification,
            // and carrying a second hash implementation for one cache key is a supply-chain
            // line item to explain in a tool whose pitch is being careful about those. Truncated
            // to 32 hex characters — this names a folder, it does not verify anything.
            let id = {
                use sha2::{Digest, Sha256};
                // `hex::encode`, not `format!("{:x}")`: the `LowerHex` impl belongs to the
                // array type the digest returns, and the array type changes under `sha2`.
                let digest = Sha256::digest(spec.name.as_bytes());
                hex::encode(digest)[..32].to_string()
            };
            let dest_dir = self.core.install_dir.join(&id);
            if dest_dir.exists() {
                tokio::fs::remove_dir_all(&dest_dir)
                    .await
                    .map_err(Error::from)?;
            }
            crate::utils::file::ensure_dir_async(&dest_dir).await?;

            let filename = crate::utils::file::url_filename(&spec.name)?;
            // The vocabulary, not a fifth hand-written list. This one was matched with
            // `.contains()` rather than `ends_with`, so `notes.gz.txt` was an archive and
            // `report.tar.summary` was one too — and three of its six entries (`.tar`, `.gz`,
            // `.xz`, `.bz2` bare) named things `extract_archive` could not open, which meant a
            // silent `fs::copy` reported as a successful deploy.
            let is_archive = crate::backends::artifact::format::Format::of_filename(&filename)
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
                crate::utils::file::copy_over(&dl_path, &dest_dir.join(&filename)).await?;
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
                let bin_name = crate::utils::strip_archive_suffixes(&filename);

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

            let record = WebState {
                url: spec.name.clone(),
                local_path: dest_dir.to_string_lossy().to_string(),
                bin_link: final_bin_link,
                etag: remote_etag,
                last_modified: remote_mod,
                installed_by: None,
                system_package: None,
            };
            installed_records.push((spec.name.clone(), record.clone()));
            state.insert(spec.name.clone(), record);
        }

        self.core
            .commit_state(installed_records, Vec::new())
            .await?;
        Ok(())
    }

    async fn remove(
        &self,
        urls: &[String],
        _: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        let mut state = self.core.load_state().await?;
        let mut failures = Vec::new();
        // What this call changed, committed rather than the whole map — see `commit_state`.
        let mut removed_urls: Vec<String> = Vec::new();
        for url in urls {
            if let Some(entry) = state.remove(url) {
                let deployed = Deployed::default()
                    .owned(
                        entry.installed_by.as_deref(),
                        entry.system_package.as_deref(),
                    )
                    .maybe_path(entry.bin_link.as_ref())
                    .path(&entry.local_path)
                    .cached_url(url);
                let errors = tear_down(
                    &deployed,
                    &self.core.executor,
                    self.core.clean_cache_on_remove,
                    &self.core.cache_dirs,
                )
                .await;
                if errors.is_empty() {
                    removed_urls.push(url.clone());
                    info!("Web: Removed resource: {}", url);
                } else {
                    // The file is still on disk and still on PATH. Dropping it from state
                    // anyway would make it drift no `sync` can see — so the url never joins
                    // `removed_urls`, and the shared record it was taken from stands.
                    let _ = entry;
                    failures.push(format!("{}: {}", url, errors.join("; ")));
                }
            }
        }
        self.core.commit_state(Vec::new(), removed_urls).await?;
        if !failures.is_empty() {
            return Err(still_installed("web resource", &failures));
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
        let state = self.core.load_state().await?;
        Ok(state.keys().map(|u| Package::new(u, "web")).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.installed_listing().await?;
        Ok(all.iter().find(|p| p.name == name).cloned())
    }

    async fn owned_system_packages(&self) -> Result<Vec<(String, String)>> {
        // D5: report the `.deb`/`.rpm` resources this backend handed to a system manager, so the
        // unmanaged crawl defers to it.
        Ok(self
            .core
            .load_state()
            .await?
            .values()
            .filter_map(|s| match (&s.installed_by, &s.system_package) {
                (Some(installer), Some(pkg)) => Some((installer.clone(), pkg.clone())),
                _ => None,
            })
            .collect())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let core = Arc::new(WebBackendCore::new(
        exec.clone(),
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
    use crate::core::executor::DryRunOutput;
    use crate::core::executor::MockExecutor;
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
        let exec =
            CommandExecutor::with_layer(false, false, mock.clone(), vfs, Arc::new(DashMap::new()));
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
        core.commit_state(vec![(url.to_string(), entry)], Vec::new())
            .await
            .expect("writing the state");
    }

    /// **Two concurrent installs both keep their record.**
    ///
    /// The lost-update race this closes: `load_state` took the internal lock, read the whole
    /// file and released it; `save_state` took it again and wrote the whole file. In `install`
    /// those two calls are 220 lines and a download apart, so two `web:` packages in one wave
    /// both read `{}` and whichever saved last wrote a map holding only its own record. The
    /// record that lost describes a deployed file and its `bin_link`: the package then reads as
    /// unmanaged, the next sync cannot see what it deployed, and teardown has nothing to remove.
    ///
    /// **What makes this reproduce, where nothing in the suite did.** `MockExecutor` has had a
    /// `delays` map since it was written — *"how long a command takes, for tests about
    /// concurrency rather than about output"* — and it was used on exactly two commands, both
    /// *reads*, in one file about graph ordering. No mutating command anywhere was ever given a
    /// non-zero duration, which is precisely why R1, R2 and R4 could all be real and never fire:
    /// mocked, the "download" returns in microseconds and the window is essentially closed. The
    /// yields below are that missing duration, at the one point that matters — between reading
    /// the state and committing to it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_concurrent_installs_do_not_lose_each_other() {
        let (web, _mock, _tmp) = backend("race");
        let core = web.core.clone();

        // Both tasks read first, both then wait, and only then does either commit — which is
        // the interleaving `install` produces when two packages download at once. Under the old
        // read-modify-write this is the exact sequence that loses A.
        let a = {
            let core = core.clone();
            tokio::spawn(async move {
                let _seen = core.load_state().await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                core.commit_state(
                    vec![(
                        "https://example.test/a".into(),
                        handed_to("dpkg", "a", "https://example.test/a"),
                    )],
                    Vec::new(),
                )
                .await
                .expect("A commits");
            })
        };
        let b = {
            let core = core.clone();
            tokio::spawn(async move {
                let _seen = core.load_state().await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                core.commit_state(
                    vec![(
                        "https://example.test/b".into(),
                        handed_to("dpkg", "b", "https://example.test/b"),
                    )],
                    Vec::new(),
                )
                .await
                .expect("B commits");
            })
        };
        a.await.expect("A");
        b.await.expect("B");

        let state = core.load_state().await.unwrap();
        assert!(
            state.contains_key("https://example.test/a"),
            "A's record was overwritten by B's whole-map write — the read-modify-write is back. \
             The state must be merged under one lock, not rewritten from what each task read \
             before its download."
        );
        assert!(
            state.contains_key("https://example.test/b"),
            "B's record is missing, which is the same defect with the tasks the other way round"
        );

        // And on disk, not only in the memo: a map that is right in memory and wrong in the
        // file is the same bug one process later.
        let on_disk = std::fs::read_to_string(&core.state_file).expect("the state file");
        assert!(
            on_disk.contains("/a") && on_disk.contains("/b"),
            "the file holds only one of the two records:\n{on_disk}"
        );
        assert!(
            !on_disk.contains("\n  "),
            "the state file is pretty-printed again. `core::state` ruled this for the registry \
             beside it — machine-read bookkeeping nobody opens, and pretty printing roughly \
             doubles the bytes."
        );
    }

    /// **A removal that fails leaves the record alone, even while another task is committing.**
    ///
    /// The other half of the same merge. `remove` used to put the entry back into its *local*
    /// copy and then write that copy whole; now it simply never names the url as removed, and
    /// the shared record stands. A merge that dropped it anyway would make a still-installed
    /// package invisible to the next sync — drift nothing can see, which is worse than the
    /// failure it came from.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_commit_beside_a_removal_keeps_both_answers() {
        let (web, _mock, _tmp) = backend("race2");
        let core = web.core.clone();
        record(
            &core,
            "https://example.test/kept",
            handed_to("dpkg", "kept", "https://example.test/kept"),
        )
        .await;
        record(
            &core,
            "https://example.test/gone",
            handed_to("dpkg", "gone", "https://example.test/gone"),
        )
        .await;

        let remover = {
            let core = core.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                core.commit_state(Vec::new(), vec!["https://example.test/gone".into()])
                    .await
                    .expect("the removal commits");
            })
        };
        let installer = {
            let core = core.clone();
            tokio::spawn(async move {
                core.commit_state(
                    vec![(
                        "https://example.test/new".into(),
                        handed_to("dpkg", "new", "https://example.test/new"),
                    )],
                    Vec::new(),
                )
                .await
                .expect("the install commits");
            })
        };
        remover.await.expect("remover");
        installer.await.expect("installer");

        let state = core.load_state().await.unwrap();
        assert!(
            state.contains_key("https://example.test/kept"),
            "an untouched record vanished"
        );
        assert!(
            state.contains_key("https://example.test/new"),
            "the concurrent install was lost"
        );
        assert!(
            !state.contains_key("https://example.test/gone"),
            "the removal was undone by the concurrent install's write — which is the lost update \
             in the other direction, and it leaves a record for a file that is gone"
        );
    }

    fn handed_to(installer: &str, package: &str, url: &str) -> WebState {
        WebState {
            url: url.to_string(),
            // A handoff owns no tree of Shall's — the manager placed the files.
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

    /// A system manager's removal, spelled the way this machine will actually launch it.
    ///
    /// Removing a `.deb` or an `.rpm` asks for privilege, and whether that becomes a `sudo`
    /// depends on the platform and the euid — `CommandExecutor::escalates` is the one place that
    /// knows. Asked rather than restated, because the three registrations below were hard-coded
    /// without it and were wrong on every Linux machine that is not root.
    fn removal_argv(line: &str) -> String {
        let mut parts = line.split(' ');
        let cmd = parts.next().unwrap_or_default();
        let args: Vec<&str> = parts.collect();
        CommandExecutor::as_launched(cmd, &args, true)
    }

    /// D5: a `.deb` Shall handed to `dpkg` is removed **through `dpkg`**, by the package name
    /// read out of the file at install time — not by the URL, and not by deleting a tree Shall
    /// does not own.
    #[tokio::test]
    async fn a_resource_a_system_manager_owns_is_removed_through_that_manager() {
        let (web, mock, _tmp) = backend("deb");
        let url = "https://example.invalid/fd_10.2.0_amd64.deb";
        record(&web.core, url, handed_to("dpkg", "fd", url)).await;

        // Registered as the product will really launch it. A system manager's removal asks
        // for privilege, so on a Linux runner it arrives as `sudo dpkg -r fd` and on Windows
        // as `dpkg -r fd` — three environments, two answers, and `escalates` is the one place
        // that knows which. Hard-coded here, it passed on Windows and had never once run on
        // Linux, because the build matrix was producing one target out of four.
        mock.set_response(&removal_argv("dpkg -r fd"), Ok(DryRunOutput::new().into()));
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
            !web.core.load_state().await.unwrap().contains_key(url),
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

        mock.set_response(&removal_argv("rpm -e fd"), Ok(DryRunOutput::new().into()));
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
    /// the record anyway would make the resource drift *no sync can see*: Shall would have
    /// forgotten it, so nothing would ever try again and nothing would report it. The record
    /// goes back, and the call returns an error naming what is still installed.
    #[tokio::test]
    async fn a_failed_removal_keeps_the_record_rather_than_forgetting_the_resource() {
        let (web, mock, _tmp) = backend("stuck");
        let url = "https://example.invalid/fd_10.2.0_amd64.deb";
        record(&web.core, url, handed_to("dpkg", "fd", url)).await;

        mock.set_response(
            &removal_argv("dpkg -r fd"),
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
            web.core.load_state().await.unwrap().contains_key(url),
            "the record was dropped for a resource that is still installed — the one state no \
             sync can detect"
        );
    }

    /// A recorded installer Shall has no removal argv for is an error, not a silent skip. The
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
        assert!(web.core.load_state().await.unwrap().contains_key(url));
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
