use crate::core::{
    BackendCore, Installable, Queryable, 
    security::verify_checksum,
    CommandExecutor, Package, PackageSpec, Result, Error,
};
use crate::utils::archive::extract_archive;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::Mutex;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Internal state metadata for resources managed via the 'web' backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebState {
    url: String,
    local_path: String,
    bin_link: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
}

/// Core backend implementation for direct HTTP/HTTPS downloads.
pub struct WebBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub install_dir: PathBuf,
    pub state_file: PathBuf,
    pub internal_lock: Mutex<()>,
}

impl WebBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("web");
        Self {
            executor,
            name: "web".to_string(),
            install_dir: base.clone(),
            state_file: base.join("installed.json"),
            internal_lock: Mutex::new(()),
        }
    }

    async fn load_state(&self) -> HashMap<String, WebState> {
        let _guard = self.internal_lock.lock().await;
        if !self.state_file.exists() {
            return HashMap::new();
        }
        let data = tokio::fs::read_to_string(&self.state_file).await.unwrap_or_default();
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
    fn name(&self) -> &str { &self.name }
    fn is_available(&self) -> bool { true }
}

pub struct WebInstallable {
    pub core: Arc<WebBackendCore>,
}

#[async_trait]
impl Installable for WebInstallable {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let mut state = self.core.load_state().await;
        let client = reqwest::Client::builder()
            .user_agent("linix-manager")
            .build()
            .map_err(Error::from)?;

        for spec in specs {
            let head_res = client.head(&spec.name).send().await.map_err(Error::from)?;
            let remote_etag = head_res.headers().get("etag").and_then(|v| v.to_str().ok().map(|s| s.to_string()));
            let remote_mod = head_res.headers().get("last-modified").and_then(|v| v.to_str().ok().map(|s| s.to_string()));

            if let Some(existing) = state.get(&spec.name) {
                if (remote_etag.is_some() && remote_etag == existing.etag) || 
                   (remote_mod.is_some() && remote_mod == existing.last_modified) {
                    debug!("Web: {} is up to date, skipping download.", spec.name);
                    continue;
                }
            }

            info!("Web: Downloading resource: {}", spec.name);
            let response = client.get(&spec.name).send().await.map_err(Error::from)?;
            let bytes = response.bytes().await.map_err(Error::from)?;

            let tmp_dir = tempfile::tempdir().map_err(Error::from)?;
            let dl_path = tmp_dir.path().join("downloaded_file");
            tokio::fs::write(&dl_path, bytes).await.map_err(Error::from)?;

            if let Some(expected_sha) = spec.options.get("sha256") {
                verify_checksum(&dl_path, expected_sha)?;
            }

            let id = format!("{:x}", md5::compute(&spec.name));
            let dest_dir = self.core.install_dir.join(&id);
            let _ = tokio::fs::remove_dir_all(&dest_dir).await;
            tokio::fs::create_dir_all(&dest_dir).await.map_err(Error::from)?;

            let filename = spec.name.split('/').last().unwrap_or("resource");
            let is_archive = [".zip", ".gz", ".tar", ".xz", ".bz2", ".tgz"].iter().any(|ext| filename.contains(ext));
            
            if is_archive {
                extract_archive(&dl_path, &dest_dir)?;
            } else {
                tokio::fs::copy(&dl_path, dest_dir.join(filename)).await.map_err(Error::from)?;
            }

            // WIRING: Cross-platform binary discovery and linkage
            let mut final_bin_link = None;
            if spec.options.get("type").map(|t| t == "program").unwrap_or(true) {
                let bin_name = spec.options.get("bin").map(|s| s.as_str()).unwrap_or_else(|| {
                    filename.split('.').next().unwrap_or(filename)
                });
                
                let bin_dest_base = dirs::home_dir()
                    .ok_or_else(|| Error::Other("Home directory not found".into()))?
                    .join(".local").join("bin").join(bin_name);
                
                let mut entries = walkdir::WalkDir::new(&dest_dir).into_iter().filter_map(|e| e.ok());
                let bin_src = entries.find(|e| {
                    let fname = e.file_name().to_string_lossy().to_lowercase();
                    fname == bin_name.to_lowercase() || 
                    fname == format!("{}.exe", bin_name.to_lowercase()) ||
                    (fname.starts_with(bin_name) && !fname.contains('.'))
                }).map(|e| e.into_path());

                if let Some(src_path) = bin_src {
                    let bin_dest = bin_dest_base.clone();
                    let _ = std::fs::create_dir_all(bin_dest.parent().unwrap());
                    let _ = std::fs::remove_file(&bin_dest);

                    #[cfg(unix)] {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&src_path, std::fs::Permissions::from_mode(0o755));
                        std::os::unix::fs::symlink(&src_path, &bin_dest).map_err(Error::from)?;
                    }

                    #[cfg(windows)] {
                        let mut win_bin_dest = bin_dest.clone();
                        if win_bin_dest.extension().is_none() { win_bin_dest.set_extension("exe"); }
                        std::fs::copy(&src_path, &win_bin_dest).map_err(Error::from)?;
                    }

                    final_bin_link = Some(bin_dest.to_string_lossy().to_string());
                }
            }

            state.insert(spec.name.clone(), WebState {
                url: spec.name.clone(),
                local_path: dest_dir.to_string_lossy().to_string(),
                bin_link: final_bin_link,
                etag: remote_etag,
                last_modified: remote_mod,
            });
        }
        
        self.core.save_state(&state).await?;
        Ok(())
    }

    async fn remove(&self, urls: &[String], _: bool) -> Result<()> {
        let mut state = self.core.load_state().await;
        for url in urls {
            if let Some(entry) = state.remove(url) {
                if let Some(ref l) = entry.bin_link {
                    let _ = std::fs::remove_file(l);
                }
                let _ = tokio::fs::remove_dir_all(entry.local_path).await;
                info!("Web: Removed resource: {}", url);
            }
        }
        self.core.save_state(&state).await?;
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