use crate::core::{CommandExecutor, Package, PackageManager, Result, PackageSpec, Error, security::verify_checksum};
use crate::utils::{archive::extract_archive, file::atomic_write};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::sync::Mutex;
use tracing::{info, warn, debug};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebState {
    url: String,
    local_path: String,
    bin_link: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
}

pub struct WebManager {
    executor: CommandExecutor,
    install_dir: PathBuf,
    state_file: PathBuf,
    internal_lock: Mutex<()>,
}

impl WebManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
let base = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
Self {
            executor,
            install_dir: base.join("web"),
            state_file: base.join("web_installed.json"),
            internal_lock: Mutex::new(()),
        }
    }

    /// REAL LOGIC: Removes folders and symlinks that are no longer in the state registry
    async fn garbage_collect(&self, active_ids: &HashSet<String>) -> Result<()> {
        if !self.install_dir.exists() { return Ok(()); }
        let mut entries = tokio::fs::read_dir(&self.install_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let id = entry.file_name().to_string_lossy().to_string();
            if !active_ids.contains(&id) {
                info!("Garbage Collecting orphaned web package: {}", id);
                let _ = tokio::fs::remove_dir_all(entry.path()).await;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl PackageManager for WebManager {
    fn name(&self) -> &str { "web" }
    fn is_available(&self) -> bool { true }

    async fn install_with_options(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        let mut state: HashMap<String, WebState> = if self.state_file.exists() {
            serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?
        } else { HashMap::new() };

        let client = reqwest::Client::new();

        for spec in specs {
            // 1. SMART UPDATE CHECK: Use HTTP HEAD to check ETag or Last-Modified
            let head_res = client.head(&spec.name).header("User-Agent", "linix-manager").send().await?;
            let remote_etag = head_res.headers().get("etag").and_then(|v| v.to_str().ok().map(|s| s.to_string()));
            let remote_mod = head_res.headers().get("last-modified").and_then(|v| v.to_str().ok().map(|s| s.to_string()));

            if let Some(existing) = state.get(&spec.name) {
                // Skip if the fingerprint (ETag) matches our local database
                if remote_etag.is_some() && remote_etag == existing.etag {
                    debug!("Skipping {}; ETag matches.", spec.name);
                    continue;
                }
                // Fallback to Last-Modified timestamp
                if remote_mod.is_some() && remote_mod == existing.last_modified {
                    debug!("Skipping {}; Last-Modified matches.", spec.name);
                    continue;
                }
            }

            // 2. ATOMIC DOWNLOAD
            info!("Downloading web resource: {}", spec.name);
            let bytes = client.get(&spec.name).header("User-Agent", "linix-manager").send().await?.bytes().await?;
            let tmp = tempfile::tempdir()?;
            let dl_path = tmp.path().join("downloaded_file");
            tokio::fs::write(&dl_path, bytes).await?;

            // 3. SECURITY: Verify SHA256 if provided in config string (e.g. url@sha256=xxx)
            if let Some(h) = spec.options.get("sha256") { verify_checksum(&dl_path, h)?; }

            let id = format!("{:x}", md5::compute(&spec.name));
            let dest_dir = self.install_dir.join(&id);
            let _ = tokio::fs::remove_dir_all(&dest_dir).await;
            tokio::fs::create_dir_all(&dest_dir).await?;

            // 4. SMART EXTRACTION
            let filename = spec.name.split('/').last().unwrap_or("app");
            let is_archive = [".zip", ".gz", ".tar", ".xz", ".bz2"].iter().any(|ext| filename.contains(ext));
            
            if is_archive {
                extract_archive(&dl_path, &dest_dir)?;
            } else {
                tokio::fs::copy(&dl_path, dest_dir.join(filename)).await?;
            }

            // 5. BINARY LINKING
            let mut bin_link = None;
            if spec.options.get("type").map(|t| t == "program").unwrap_or(true) {
                let bin_name = spec.options.get("bin").map(|s| s.as_str()).unwrap_or_else(|| filename.split('.').next().unwrap_or(filename));
                let bin_dest = dirs::home_dir().unwrap().join(".local/bin").join(bin_name);
                
                // Search for the binary (it might be inside a subfolder in the archive)
                let mut bin_src = dest_dir.join(bin_name);
                if !bin_src.exists() {
                    // Walk the folder to find it
                    if let Ok(mut it) = tokio::fs::read_dir(&dest_dir).await {
                        while let Some(e) = it.next_entry().await? {
                            if e.file_name().to_string_lossy().to_lowercase() == bin_name.to_lowercase() {
                                bin_src = e.path();
                                break;
                            }
                        }
                    }
                }

                if bin_src.exists() {
                    #[cfg(unix)] {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&bin_src, std::fs::Permissions::from_mode(0o755));
                        let _ = tokio::fs::remove_file(&bin_dest).await;
                        let _ = tokio::fs::create_dir_all(bin_dest.parent().unwrap()).await;
                        std::os::unix::fs::symlink(&bin_src, &bin_dest)?;
                        bin_link = Some(bin_dest.to_string_lossy().to_string());
                    }
                }
            }

            state.insert(spec.name.clone(), WebState {
                url: spec.name.clone(),
                local_path: dest_dir.to_string_lossy().to_string(),
                bin_link,
                etag: remote_etag,
                last_modified: remote_mod,
            });
        }
        
        // 6. PERSISTENCE: Save state atomically to avoid corruption on crash
        let active_ids: HashSet<String> = state.keys().map(|k| format!("{:x}", md5::compute(k))).collect();
        let _ = self.garbage_collect(&active_ids).await;
        atomic_write(&self.state_file, &serde_json::to_string_pretty(&state)?)?;
        Ok(())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let specs: Vec<_> = p.iter().map(|u| PackageSpec { name: u.clone(), backend: "web".into(), options: HashMap::new() }).collect();
        self.install_with_options(&specs, s).await
    }

    async fn remove(&self, packages: &[String], _: bool) -> Result<()> {
        if !self.state_file.exists() { return Ok(()); }
        let mut state: HashMap<String, WebState> = serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?;
        for url in packages {
            if let Some(entry) = state.remove(url) {
                if let Some(l) = entry.bin_link { let _ = tokio::fs::remove_file(l).await; }
                let _ = tokio::fs::remove_dir_all(entry.local_path).await;
            }
        }
        atomic_write(&self.state_file, &serde_json::to_string_pretty(&state)?)?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        if !self.state_file.exists() { return Ok(vec![]); }
        let state: HashMap<String, WebState> = serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?;
        Ok(state.keys().map(|u| Package::new(u, "web")).collect())
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        // REAL LOGIC: Triggering upgrade re-runs the install loop.
        // Because of the ETag checking logic above, this will only download files that actually changed.
        let installed = self.list_installed().await?;
        let names: Vec<String> = installed.into_iter().map(|p| p.name).collect();
        self.install(&names, s).await
    }
}