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
    pub install_dir: PathBuf,
    pub state_file: PathBuf,
}

impl AppImageBackendCore {
    pub fn new(executor: CommandExecutor, install_dir: PathBuf) -> Self {
        let state = install_dir.join("state.json");

        Self {
            executor,
            install_dir,
            state_file: state,
        }
    }

    async fn ensure_dirs(&self) -> Result<PathBuf> {
        if !tokio::fs::try_exists(&self.install_dir)
            .await
            .unwrap_or(false)
        {
            tokio::fs::create_dir_all(&self.install_dir).await?;
        }
        let bin_dir = dirs::home_dir()
            .ok_or_else(|| Error::Other("Could not locate home directory".into()))?
            .join(".local")
            .join("bin");

        if !tokio::fs::try_exists(&bin_dir).await.unwrap_or(false) {
            tokio::fs::create_dir_all(&bin_dir).await?;
        }
        Ok(bin_dir)
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
        let client = reqwest::Client::builder()
            .user_agent("linix-manager")
            .build()?;

        for spec in specs {
            let url = &spec.name;
            let filename = url.split('/').next_back().unwrap_or("app.AppImage");
            let dest_path = self.core.install_dir.join(filename);

            info!("AppImage: Downloading {}...", url);
            let response = client.get(url).send().await?;
            if !response.status().is_success() {
                return Err(Error::Other(format!(
                    "Download failed for {}: {}",
                    url,
                    response.status()
                )));
            }

            let bytes = response.bytes().await?;
            tokio::fs::write(&dest_path, bytes).await?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let metadata = tokio::fs::metadata(&dest_path).await?;
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                tokio::fs::set_permissions(&dest_path, perms).await?;
            }

            let link_name = filename
                .strip_suffix(".AppImage")
                .or_else(|| filename.strip_suffix(".appimage"))
                .unwrap_or(filename);

            let link_path = bin_dir.join(link_name);

            if tokio::fs::try_exists(&link_path).await.unwrap_or(false) || link_path.is_symlink() {
                let _ = tokio::fs::remove_file(&link_path).await;
            }

            #[cfg(unix)]
            {
                tokio::fs::symlink(&dest_path, &link_path).await?;
            }

            state.insert(
                spec.name.clone(),
                AppImageState {
                    url: url.clone(),
                    local_path: dest_path.to_string_lossy().to_string(),
                    symlink_path: link_path.to_string_lossy().to_string(),
                },
            );
            info!(
                "AppImage: Successfully installed {} to {}",
                link_name,
                link_path.display()
            );
        }

        self.core.save_state(&state).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        let mut state = self.core.load_state().await;

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

        self.core.save_state(&state).await?;
        Ok(())
    }
}

pub struct AppImageQueryable {
    pub core: Arc<AppImageBackendCore>,
}

#[async_trait]
impl Queryable for AppImageQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let state = self.core.load_state().await;
        Ok(state
            .keys()
            .map(|url| {
                let name = url.split('/').next_back().unwrap_or(url);
                Package::new(name, "appimage")
            })
            .collect())
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
    let core = Arc::new(AppImageBackendCore::new(
        exec.duplicate(),
        cfg.appimage_dir.clone(),
    ));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(AppImageInstallable { core: core.clone() }))
            .with_queryable(Arc::new(AppImageQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
