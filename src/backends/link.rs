// src/backends/link.rs
use crate::core::{CommandExecutor, Package, PackageManager, Result, PackageSpec, Error};
use crate::app::LuaHooks;
use async_trait::async_trait;
use std::sync::Arc;
use std::path::PathBuf;
use tracing::info;

pub struct LinkManager {
    #[allow(dead_code)] executor: CommandExecutor,
    hooks: Arc<LuaHooks>,
}

impl LinkManager {
    pub fn new(executor: CommandExecutor, hooks: Arc<LuaHooks>) -> Self {
        Self { executor, hooks }
    }
}

#[async_trait]
impl PackageManager for LinkManager {
    fn name(&self) -> &str { "link" }
    fn is_available(&self) -> bool { true }

    async fn install_with_options(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            let source = PathBuf::from(&spec.name);
            let target = spec.options.get("target").map(PathBuf::from)
                .ok_or_else(|| Error::Other("Link requires @target path".into()))?;

            if let Some(p) = target.parent() { tokio::fs::create_dir_all(p).await?; }

            if spec.options.get("template") == Some(&"true".to_string()) {
                let content = tokio::fs::read_to_string(&source).await?;
                // FIX: LuaHooks is now Send, allowing this to work in parallel
                let rendered = self.hooks.render_template(&content);
                tokio::fs::write(&target, rendered).await?;
                info!("Template rendered: {:?} -> {:?}", source, target);
            } else {
                if target.exists() || target.is_symlink() {
                    let _ = tokio::fs::remove_file(&target).await;
                }
                #[cfg(unix)] std::os::unix::fs::symlink(&source, &target)?;
                #[cfg(windows)] {
                    if source.is_dir() { std::os::windows::fs::symlink_dir(&source, &target)?; }
                    else { std::os::windows::fs::symlink_file(&source, &target)?; }
                }
                info!("Linked: {:?} -> {:?}", source, target);
            }
        }
        Ok(())
    }
    async fn install(&self, _: &[String], _: bool) -> Result<()> { Ok(()) }
    async fn remove(&self, targets: &[String], _: bool) -> Result<()> {
        for target in targets {
            let path = PathBuf::from(target);
            if path.exists() || path.is_symlink() { tokio::fs::remove_file(path).await?; }
        }
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> { Ok(vec![]) }
}