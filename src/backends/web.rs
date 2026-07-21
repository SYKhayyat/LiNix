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
}

pub struct WebBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    /// `[guard] confine_bin`: whether an `@bin=` value may name a file outside `bin_dir`
    /// (SEC1). Carried here because the backend is where the value becomes a path.
    pub confine_bin: bool,
    pub install_dir: PathBuf,
    pub state_file: PathBuf,
    pub internal_lock: Mutex<()>,
}

impl WebBackendCore {
    pub fn new(executor: CommandExecutor, install_dir: PathBuf, confine_bin: bool) -> Self {
        let state_file = install_dir.join("installed.json");
        Self {
            executor,
            name: "web".to_string(),
            confine_bin,
            install_dir,
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
        crate::utils::file::atomic_write(&self.state_file, &data)
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

            info!("Web: Downloading resource: {}", spec.name);
            let response = client.get(&spec.name).send().await.map_err(Error::from)?;
            let bytes = response.bytes().await.map_err(Error::from)?;

            let tmp_dir = tempfile::tempdir().map_err(Error::from)?;
            let dl_path = tmp_dir.path().join("downloaded_file");
            tokio::fs::write(&dl_path, bytes)
                .await
                .map_err(Error::from)?;

            if let Some(expected_sha) = spec.options.get("sha256") {
                verify_checksum(&dl_path, expected_sha)?;
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
            let is_archive = [".zip", ".gz", ".tar", ".xz", ".bz2", ".tgz"]
                .iter()
                .any(|ext| filename.contains(ext));

            if is_archive {
                let dl_path_archive = dl_path.clone();
                let dest_dir_archive = dest_dir.clone();
                tokio::task::spawn_blocking(move || {
                    extract_archive(&dl_path_archive, &dest_dir_archive)
                })
                .await
                .map_err(|e| Error::Other(e.to_string()))??;
            } else {
                tokio::fs::copy(&dl_path, dest_dir.join(filename))
                    .await
                    .map_err(Error::from)?;
            }

            let mut final_bin_link = None;
            if spec
                .options
                .get("type")
                .map(|t| t == "program")
                .unwrap_or(true)
            {
                // The name comes from the URL, not from an option: `@bin` is refused on
                // `web` (it picks between several files of one release, and a `web:` URL names
                // exactly one). Reading it here was the SEC1 traversal's entry point, and a
                // dead branch besides.
                let bin_name = filename.split('.').next().unwrap_or(filename);

                let bin_dir = dirs::home_dir()
                    .ok_or_else(|| Error::Other("Home directory not found".into()))?
                    .join(".local")
                    .join("bin");
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
                },
            );
        }

        self.core.save_state(&state).await?;
        Ok(())
    }

    async fn remove(&self, urls: &[String], _: bool) -> Result<()> {
        let mut state = self.core.load_state().await;
        let mut failures = Vec::new();
        for url in urls {
            if let Some(entry) = state.remove(url) {
                let mut errors = Vec::new();
                if let Some(ref l) = entry.bin_link {
                    if let Err(e) = crate::utils::remove_deployed_path(l).await {
                        errors.push(e);
                    }
                }
                if let Err(e) = crate::utils::remove_deployed_path(&entry.local_path).await {
                    errors.push(e);
                }
                if errors.is_empty() {
                    info!("Web: Removed resource: {}", url);
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
    async fn list_installed(&self) -> Result<Vec<Package>> {
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
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let core = Arc::new(WebBackendCore::new(
        exec.duplicate(),
        cfg.web_dir.clone(),
        cfg.guard.confine_bin,
    ));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(WebInstallable { core: core.clone() }))
            .with_queryable(Arc::new(WebQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
