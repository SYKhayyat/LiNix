use crate::core::{
    manager::{Backend, Installable, Queryable},
    security::verify_checksum,
    CommandExecutor, Package, PackageSpec, Result, Error,
};
use crate::utils::archive::extract_archive;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;
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

/// A specialized manager for direct HTTP/HTTPS downloads.
/// Supports fingerprinting (ETags), checksum verification, and binary symlinking.
/// Follows Roadmap Phase 3.2 for deterministic state management.
pub struct WebManager {
    executor: CommandExecutor,
    install_dir: PathBuf,
    state_file: PathBuf,
    /// Internal lock to prevent race conditions on the state registry file.
    internal_lock: Mutex<()>,
}

impl WebManager {
    pub fn new(executor: CommandExecutor) -> Self {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("web");
        Self {
            executor,
            install_dir: base.clone(),
            state_file: base.join("installed.json"),
            internal_lock: Mutex::new(()),
        }
    }

    /// Prepares directories and creates the internal registry if missing.
    async fn init_storage(&self) -> Result<()> {
        if !self.install_dir.exists() {
            tokio::fs::create_dir_all(&self.install_dir).await?;
        }
        Ok(())
    }

    /// Loads the current web-resource registry.
    async fn load_state(&self) -> HashMap<String, WebState> {
        if !self.state_file.exists() {
            return HashMap::new();
        }
        let data = tokio::fs::read_to_string(&self.state_file).await.unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_default()
    }

    /// Persists state atomically using the utility layer.
    async fn save_state(&self, state: &HashMap<String, WebState>) -> Result<()> {
        let data = serde_json::to_string_pretty(state).map_err(|e| Error::Other(e.to_string()))?;
        crate::utils::file::atomic_write(&self.state_file, &data)
    }
}

impl Backend for WebManager {
    fn name(&self) -> &str { "web" }
    fn is_available(&self) -> bool { true }
    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
}

#[async_trait]
impl Installable for WebManager {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        self.init_storage().await?;
        let mut state = self.load_state().await;
        let client = reqwest::Client::builder()
            .user_agent("linix-manager")
            .build()?;

        for spec in specs {
            // 1. Efficiency: Check Fingerprints (ETag/Last-Modified) via HTTP HEAD
            let head_res = client.head(&spec.name).send().await?;
            let remote_etag = head_res.headers().get("etag").and_then(|v| v.to_str().ok().map(|s| s.to_string()));
            let remote_mod = head_res.headers().get("last-modified").and_then(|v| v.to_str().ok().map(|s| s.to_string()));

            if let Some(existing) = state.get(&spec.name) {
                if (remote_etag.is_some() && remote_etag == existing.etag) || 
                   (remote_mod.is_some() && remote_mod == existing.last_modified) {
                    debug!("Web: {} is up to date, skipping download.", spec.name);
                    continue;
                }
            }

            // 2. Download into temporary buffer
            info!("Web: Downloading resource: {}", spec.name);
            let response = client.get(&spec.name).send().await?;
            if !response.status().is_success() {
                return Err(Error::Other(format!("Failed to download {}: {}", spec.name, response.status())));
            }
            let bytes = response.bytes().await?;

            let tmp_dir = tempfile::tempdir()?;
            let dl_path = tmp_dir.path().join("downloaded_file");
            tokio::fs::write(&dl_path, bytes).await?;

            // 3. Security: Checksum Verification
            if let Some(expected_sha) = spec.options.get("sha256") {
                verify_checksum(&dl_path, expected_sha)?;
            } else {
                warn!("Web: No SHA256 provided for {}; installing unverified binary.", spec.name);
            }

            // 4. Content Processing (Extraction vs Direct Copy)
            let id = format!("{:x}", md5::compute(&spec.name));
            let dest_dir = self.install_dir.join(&id);
            let _ = tokio::fs::remove_dir_all(&dest_dir).await;
            tokio::fs::create_dir_all(&dest_dir).await?;

            let filename = spec.name.split('/').last().unwrap_or("resource");
            let is_archive = [".zip", ".gz", ".tar", ".xz", ".bz2", ".tgz"].iter().any(|ext| filename.contains(ext));
            
            if is_archive {
                extract_archive(&dl_path, &dest_dir)?;
            } else {
                tokio::fs::copy(&dl_path, dest_dir.join(filename)).await?;
            }

            // 5. Binary Linkage (Roadmap 4.2)
            let mut bin_link = None;
            if spec.options.get("type").map(|t| t == "program").unwrap_or(true) {
                let bin_name = spec.options.get("bin").map(|s| s.as_str()).unwrap_or_else(|| {
                    filename.split('.').next().unwrap_or(filename)
                });
                
                let bin_dest = dirs::home_dir().unwrap().join(".local").join("bin").join(bin_name);
                
                // Deep scan for the binary inside the extracted folder
                let mut entries = walkdir::WalkDir::new(&dest_dir).into_iter().filter_map(|e| e.ok());
                let bin_src = entries.find(|e| {
                    let fname = e.file_name().to_string_lossy().to_lowercase();
                    fname == bin_name.to_lowercase() || (fname.starts_with(bin_name) && !fname.contains('.'))
                }).map(|e| e.into_path());

                if let Some(src) = bin_src {
                    #[cfg(unix)] {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o755));
                        let _ = tokio::fs::remove_file(&bin_dest).await;
                        let _ = tokio::fs::create_dir_all(bin_dest.parent().unwrap()).await;
                        std::os::unix::fs::symlink(&src, &bin_dest)?;
                        bin_link = Some(bin_dest.to_string_lossy().to_string());
                    }
                }
            }

            // 6. Persistence
            state.insert(spec.name.clone(), WebState {
                url: spec.name.clone(),
                local_path: dest_dir.to_string_lossy().to_string(),
                bin_link,
                etag: remote_etag,
                last_modified: remote_mod,
            });
        }
        
        self.save_state(&state).await?;
        Ok(())
    }

    async fn remove(&self, urls: &[String], _: bool) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        let mut state = self.load_state().await;
        
        for url in urls {
            if let Some(entry) = state.remove(url) {
                if let Some(l) = entry.bin_link {
                    let _ = tokio::fs::remove_file(l).await;
                }
                let _ = tokio::fs::remove_dir_all(entry.local_path).await;
                info!("Web: Removed resource: {}", url);
            }
        }
        
        self.save_state(&state).await?;
        Ok(())
    }
}

#[async_trait]
impl Queryable for WebManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let state = self.load_state().await;
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