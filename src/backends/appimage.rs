use crate::core::{
    Backend, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Error
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, debug, warn};

/// Internal state metadata for a managed AppImage.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppImageState {
    url: String,
    local_path: String,
    symlink_path: String,
}

/// A specialized manager for standalone Linux AppImages.
/// Handles downloads, executable permissions, and ~/.local/bin symlinking.
pub struct AppImageManager {
    executor: CommandExecutor,
    install_dir: PathBuf,
    state_file: PathBuf,
}

impl AppImageManager {
    pub fn new(executor: CommandExecutor) -> Self {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("appimages");
        
        let state = base.join("state.json");
        
        Self { 
            executor, 
            install_dir: base, 
            state_file: state 
        }
    }

    /// Prepares the filesystem structure for AppImage storage and binary exposure.
    async fn ensure_dirs(&self) -> Result<PathBuf> {
        if !self.install_dir.exists() {
            tokio::fs::create_dir_all(&self.install_dir).await?;
        }
        let bin_dir = dirs::home_dir()
            .ok_or_else(|| Error::Other("Could not locate home directory".into()))?
            .join(".local")
            .join("bin");
        
        if !bin_dir.exists() {
            tokio::fs::create_dir_all(&bin_dir).await?;
        }
        Ok(bin_dir)
    }

    /// Loads the current AppImage registry from disk.
    async fn load_state(&self) -> HashMap<String, AppImageState> {
        if !self.state_file.exists() {
            return HashMap::new();
        }
        let data = tokio::fs::read_to_string(&self.state_file).await.unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_default()
    }

    /// Saves the registry state atomically.
    async fn save_state(&self, state: &HashMap<String, AppImageState>) -> Result<()> {
        let data = serde_json::to_string_pretty(state).map_err(|e| Error::Other(e.to_string()))?;
        crate::utils::file::atomic_write(&self.state_file, &data)
    }
}

impl Backend for AppImageManager {
    fn name(&self) -> &str { "appimage" }

    fn is_available(&self) -> bool {
        // AppImages are a Linux-specific distribution format.
        cfg!(target_os = "linux")
    }

    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
}

#[async_trait]
impl Installable for AppImageManager {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        let bin_dir = self.ensure_dirs().await?;
        let mut state = self.load_state().await;
        let client = reqwest::Client::builder()
            .user_agent("linix-manager")
            .build()?;

        for spec in specs {
            let url = &spec.name;
            // Extract a clean filename from the URL.
            let filename = url.split('/').last().unwrap_or("app.AppImage");
            let dest_path = self.install_dir.join(filename);
            
            info!("AppImage: Downloading {}...", url);
            let response = client.get(url).send().await?;
            if !response.status().is_success() {
                return Err(Error::Other(format!("Download failed for {}: {}", url, response.status())));
            }
            
            let bytes = response.bytes().await?;
            tokio::fs::write(&dest_path, bytes).await?;

            // 1. Set Executable Permissions (0755)
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = tokio::fs::metadata(&dest_path).await?.permissions();
                perms.set_mode(0o755);
                tokio::fs::set_permissions(&dest_path, perms).await?;
            }

            // 2. Create Symlink in user PATH
            // Use the filename without extension as the command name.
            let link_name = filename.strip_suffix(".AppImage")
                .or_else(|| filename.strip_suffix(".appimage"))
                .unwrap_or(filename);
            
            let link_path = bin_dir.join(link_name);
            
            if link_path.exists() || link_path.is_symlink() {
                debug!("AppImage: Replacing existing symlink at {:?}", link_path);
                let _ = tokio::fs::remove_file(&link_path).await;
            }

            #[cfg(unix)] {
                std::os::unix::fs::symlink(&dest_path, &link_path)?;
            }

            // 3. Update internal registry
            state.insert(spec.name.clone(), AppImageState {
                url: url.clone(),
                local_path: dest_path.to_string_lossy().to_string(),
                symlink_path: link_path.to_string_lossy().to_string(),
            });
            info!("AppImage: Successfully installed {} to {}", link_name, link_path.display());
        }

        self.save_state(&state).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        let mut state = self.load_state().await;
        
        for name in names {
            if let Some(info) = state.remove(name) {
                debug!("AppImage: Removing local files for {}", name);
                let _ = tokio::fs::remove_file(&info.local_path).await;
                let _ = tokio::fs::remove_file(&info.symlink_path).await;
                info!("AppImage: Removed {}", name);
            } else {
                warn!("AppImage: No record found for {}, skipping removal.", name);
            }
        }
        
        self.save_state(&state).await?;
        Ok(())
    }
}

#[async_trait]
impl Queryable for AppImageManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let state = self.load_state().await;
        Ok(state.keys().map(|url| {
            let name = url.split('/').last().unwrap_or(url);
            Package::new(name, "appimage")
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // AppImages are always manually managed by LiNix.
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}