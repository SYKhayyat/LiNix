use crate::core::{CommandExecutor, Package, PackageManager, Result, PackageSpec, Error};
use crate::core::security::verify_checksum; // FIX 2: Security
use crate::utils::archive::extract_archive;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::Mutex;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebState {
    url: String,
    local_path: String,
    is_program: bool,
    bin_link: Option<String>,
}

pub struct WebManager {
    executor: CommandExecutor,
    install_dir: PathBuf,
    state_file: PathBuf,
    internal_lock: Mutex<()>,
}

impl WebManager {
    pub fn new(executor: CommandExecutor, _settings: Option<HashMap<String, String>>) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let base_dir = home.join(".local").join("share").join("linix");
        Self {
            executor,
            install_dir: base_dir.join("web"),
            state_file: base_dir.join("web_installed.json"),
            internal_lock: Mutex::new(()),
        }
    }

    async fn load_state(&self) -> HashMap<String, WebState> {
        if let Ok(data) = tokio::fs::read_to_string(&self.state_file).await {
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            HashMap::new()
        }
    }
}

#[async_trait]
impl PackageManager for WebManager {
    fn name(&self) -> &str { "web" }
    fn is_available(&self) -> bool { true }

    async fn install_with_options(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        let mut state = self.load_state().await;
        let client = reqwest::Client::new();

        for spec in specs {
            info!("Web downloading: {}", spec.name);
            let response = client.get(&spec.name).send().await.map_err(|e| Error::Other(e.to_string()))?;
            let filename = spec.name.split('/').last().unwrap_or("download.bin");
            
            let tmp_dir = tempfile::tempdir().map_err(|e| Error::Io(e))?;
            let download_path = tmp_dir.path().join(filename);
            tokio::fs::write(&download_path, response.bytes().await.map_err(|e| Error::Other(e.to_string()))?).await?;

            // FIX 2: Checksum Enforcement
            if let Some(expected_sha) = spec.options.get("sha256") {
                verify_checksum(&download_path, expected_sha)?;
                info!("Checksum verified for {}", spec.name);
            }

            let id = {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut s = DefaultHasher::new();
                spec.name.hash(&mut s);
                format!("{:x}", s.finish())
            };

            // FIX 1: Atomicity via unique Store Path
            let dest_dir = self.install_dir.join(id);
            tokio::fs::create_dir_all(&dest_dir).await?;

            let is_program = spec.options.get("type").map(|t| t == "program").unwrap_or(false);
            let mut bin_link = None;

            if is_program {
                extract_archive(&download_path, &dest_dir)?;
                let bin_name = spec.options.get("bin").map(|s| s.as_str()).unwrap_or_else(|| {
                    filename.split('.').next().unwrap_or(filename)
                });
                
                let bin_dest = dirs::home_dir().unwrap_or_default().join(".local").join("bin").join(bin_name);
                let bin_src = dest_dir.join(bin_name);

                #[cfg(unix)] {
                    let _ = self.executor.run("chmod", &["+x", &bin_src.to_string_lossy()], false).await;
                    let _ = self.executor.run("ln", &["-sf", &bin_src.to_string_lossy(), &bin_dest.to_string_lossy()], false).await;
                }
                #[cfg(windows)] {
                    let _ = self.executor.run("cmd", &["/C", "copy", "/Y", &bin_src.to_string_lossy(), &bin_dest.to_string_lossy()], false).await;
                }
                bin_link = Some(bin_dest.to_string_lossy().to_string());
            } else {
                tokio::fs::copy(&download_path, dest_dir.join(filename)).await?;
            }

            state.insert(spec.name.clone(), WebState {
                url: spec.name.clone(),
                local_path: dest_dir.to_string_lossy().to_string(),
                is_program,
                bin_link,
            });
        }

        tokio::fs::write(&self.state_file, serde_json::to_string_pretty(&state)?).await?;
        Ok(())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let specs: Vec<_> = p.iter().map(|u| PackageSpec { name: u.clone(), backend: "web".into(), options: HashMap::new() }).collect();
        self.install_with_options(&specs, s).await
    }

    async fn remove(&self, packages: &[String], _sudo: bool) -> Result<()> {
        let _guard = self.internal_lock.lock().await;
        let mut state = self.load_state().await;
        for url in packages {
            if let Some(entry) = state.remove(url) {
                if let Some(link) = entry.bin_link { let _ = tokio::fs::remove_file(link).await; }
                let _ = tokio::fs::remove_dir_all(entry.local_path).await;
            }
        }
        tokio::fs::write(&self.state_file, serde_json::to_string_pretty(&state)?).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let state = self.load_state().await;
        Ok(state.keys().map(|u| Package::new(u, "web")).collect())
    }
}