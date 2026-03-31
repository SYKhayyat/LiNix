use crate::core::{CommandExecutor, Package, PackageManager, Result, PackageSpec};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::info;

pub struct EmacsManager {
    executor: CommandExecutor,
}

impl EmacsManager {
    pub fn new(executor: CommandExecutor, _settings: Option<HashMap<String, String>>) -> Self {
        Self { executor }
    }

    fn get_emacs_dir() -> PathBuf {
        if cfg!(target_os = "windows") {
            dirs::home_dir().unwrap_or_default().join("AppData").join("Roaming").join(".emacs.d")
        } else {
            dirs::home_dir().unwrap_or_default().join(".emacs.d")
        }
    }
}

#[async_trait]
impl PackageManager for EmacsManager {
    fn name(&self) -> &str { "emacs" }
    fn is_available(&self) -> bool { true }

    async fn install_with_options(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            let source_config = PathBuf::from(&spec.name);
            let target_dir = Self::get_emacs_dir();
            tokio::fs::create_dir_all(&target_dir).await?;

            let target_init = target_dir.join("init.el");
            if target_init.exists() || target_init.is_symlink() {
                let _ = tokio::fs::remove_file(&target_init).await;
            }

            #[cfg(unix)]
            std::os::unix::fs::symlink(&source_config, &target_init)?;
            #[cfg(windows)]
            std::os::windows::fs::symlink_file(&source_config, &target_init)?;

            info!("Emacs init linked: {:?}", target_init);

            if spec.options.get("bootstrap") == Some(&"true".to_string()) {
                info!("Bootstrapping Emacs packages (this may take a minute)...");
                let _ = self.executor.run("emacs", &[
                    "--batch", 
                    "--load", &target_init.to_string_lossy(),
                    "--eval", "(message \"LiNix Sync Complete\")",
                    "--kill"
                ], false).await?;
            }
        }
        Ok(())
    }

    async fn install(&self, _p: &[String], _s: bool) -> Result<()> { Ok(()) }
    async fn remove(&self, _p: &[String], _s: bool) -> Result<()> { 
        let dir = Self::get_emacs_dir();
        if dir.exists() { tokio::fs::remove_dir_all(dir).await?; }
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> { Ok(vec![]) }
}