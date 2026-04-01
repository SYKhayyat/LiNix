use crate::core::{CommandExecutor, Package, PackageManager, Result, Error};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppImageState { url: String, local_path: String }

pub struct AppImageManager {
    executor: CommandExecutor,
    install_dir: PathBuf,
    state_file: PathBuf,
}

impl AppImageManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        let base = dirs::data_dir().unwrap_or_default().join("linix").join("appimages");
        let state = base.join("state.json");
        Self { executor, install_dir: base, state_file: state }
    }
}

#[async_trait]
impl PackageManager for AppImageManager {
    fn name(&self) -> &str { "appimage" }
    fn is_available(&self) -> bool { cfg!(target_os = "linux") }

    async fn install(&self, urls: &[String], _: bool) -> Result<()> {
        let mut state: HashMap<String, AppImageState> = if self.state_file.exists() {
            serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?
        } else { HashMap::new() };

        tokio::fs::create_dir_all(&self.install_dir).await?;
        let bin_dir = dirs::home_dir().unwrap().join(".local/bin");

        for url in urls {
            let filename = url.split('/').last().unwrap_or("app.AppImage");
            let dest = self.install_dir.join(filename);
            
            let client = reqwest::Client::new();
            let res = client.get(url).send().await?;
            tokio::fs::write(&dest, res.bytes().await?).await?;

            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
                let link_name = filename.split('.').next().unwrap_or(filename);
                let _ = tokio::fs::remove_file(bin_dir.join(link_name)).await;
                std::os::unix::fs::symlink(&dest, bin_dir.join(link_name))?;
            }
            state.insert(filename.to_string(), AppImageState { url: url.clone(), local_path: dest.to_string_lossy().to_string() });
        }
        tokio::fs::write(&self.state_file, serde_json::to_string(&state)?).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        if !self.state_file.exists() { return Ok(()); }
        let mut state: HashMap<String, AppImageState> = serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?;
        for name in names {
            if let Some(info) = state.remove(name) {
                let _ = tokio::fs::remove_file(info.local_path).await;
                let link_name = name.split('.').next().unwrap_or(name);
                let _ = tokio::fs::remove_file(dirs::home_dir().unwrap().join(".local/bin").join(link_name)).await;
            }
        }
        tokio::fs::write(&self.state_file, serde_json::to_string(&state)?).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        if !self.state_file.exists() { return Ok(vec![]); }
        let state: HashMap<String, AppImageState> = serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?;
        Ok(state.keys().map(|k| Package::new(k, "appimage")).collect())
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        let installed = self.list_installed().await?;
        let state: HashMap<String, AppImageState> = serde_json::from_str(&tokio::fs::read_to_string(&self.state_file).await?)?;
        let urls: Vec<String> = installed.iter().filter_map(|p| state.get(&p.name).map(|s| s.url.clone())).collect();
        self.install(&urls, s).await
    }
}