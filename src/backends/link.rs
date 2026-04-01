use crate::core::{CommandExecutor, Package, PackageManager, Result, PackageSpec, Error};
use crate::app::LuaHooks;
use async_trait::async_trait;
use std::sync::Arc;
use std::path::PathBuf;
use tracing::{info, warn};

pub struct LinkManager {
    executor: CommandExecutor,
    hooks: Arc<LuaHooks>,
}

impl LinkManager {
    pub fn new(executor: CommandExecutor, hooks: Arc<LuaHooks>) -> Self {
        Self { executor, hooks }
    }

    /// Helper: Determines if two paths are on the same physical drive (Windows specific)
    #[cfg(windows)]
    fn is_same_drive(a: &std::path::Path, b: &std::path::Path) -> bool {
        use std::path::Component;
        let drive_a = a.components().find(|c| matches!(c, Component::Prefix(_)));
        let drive_b = b.components().find(|c| matches!(c, Component::Prefix(_)));
        drive_a == drive_b
    }
}

#[async_trait]
impl PackageManager for LinkManager {
    fn name(&self) -> &str { "link" }
    fn is_available(&self) -> bool { true }

    async fn install_with_options(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            let source = PathBuf::from(&spec.name);
            let target_str = spec.options.get("target").ok_or_else(|| Error::Other("Link requires @target".into()))?;
            let target = PathBuf::from(target_str);

            // Ensure parent directory exists
            if let Some(p) = target.parent() { tokio::fs::create_dir_all(p).await?; }

            // 1. Handle Template Rendering
            if spec.options.get("template") == Some(&"true".to_string()) {
                info!("Rendering template: {:?} -> {:?}", source, target);
                let content = tokio::fs::read_to_string(&source).await?;
                let rendered = self.hooks.render_template(&content);
                tokio::fs::write(&target, rendered).await?;
                continue;
            }

            // 2. Clear existing target if it exists
            if target.exists() || target.is_symlink() {
                let _ = tokio::fs::remove_file(&target).await;
                let _ = tokio::fs::remove_dir_all(&target).await;
            }

            // 3. Perform Linking (Platform Specific)
            #[cfg(unix)] {
                std::os::unix::fs::symlink(&source, &target)?;
            }

            #[cfg(windows)] {
                // REAL LOGIC: Cross-drive symlinks are unreliable on Windows.
                if !Self::is_same_drive(&source, &target) {
                    warn!("Cross-drive link detected. Falling back to COPY for {:?}", source);
                    if source.is_dir() {
                        self.executor.run("cmd", &["/C", "xcopy", "/E", "/I", "/Y", &source.to_string_lossy(), &target.to_string_lossy()], false).await?;
                    } else {
                        tokio::fs::copy(&source, &target).await?;
                    }
                } else if source.is_dir() {
                    // Try Symlink, fallback to Junction (Junctions don't require Admin/Dev Mode)
                    if std::os::windows::fs::symlink_dir(&source, &target).is_err() {
                        debug!("Symlink restricted. Using Directory Junction for {:?}", source);
                        self.executor.run("cmd", &["/C", "mklink", "/J", &target.to_string_lossy(), &source.to_string_lossy()], false).await?;
                    }
                } else {
                    // Try Symlink, fallback to Hard Link (Hard Links don't require Admin)
                    if std::os::windows::fs::symlink_file(&source, &target).is_err() {
                        debug!("Symlink restricted. Using Hard Link for {:?}", source);
                        tokio::fs::hard_link(&source, &target).await?;
                    }
                }
            }
            info!("Linked: {:?} -> {:?}", source, target);
        }
        Ok(())
    }

    async fn install(&self, _: &[String], _: bool) -> Result<()> { Ok(()) }

    async fn remove(&self, targets: &[String], _: bool) -> Result<()> {
        for t in targets {
            let p = PathBuf::from(t);
            if p.exists() || p.is_symlink() {
                if p.is_dir() { tokio::fs::remove_dir_all(p).await?; }
                else { tokio::fs::remove_file(p).await?; }
            }
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // Link manager is state-driven; it doesn't "list" globally.
        // Returning empty allows the SyncEngine to handle state comparison.
        Ok(vec![])
    }
}