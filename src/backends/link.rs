use crate::core::{CommandExecutor, Result, PackageSpec, BackendCore, Installable, Error, MetadataProvider};
use crate::config::Config;
use async_trait::async_trait;
use std::sync::Arc;
use std::path::{Path, PathBuf};
use tracing::{info, warn, debug};
use tera::{Tera, Context};

/// Core backend implementation for filesystem links and configuration templating.
pub struct LinkBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub config: Arc<Config>,
}

impl LinkBackendCore {
    pub fn new(executor: CommandExecutor, config: Arc<Config>) -> Self {
        Self { 
            executor, 
            name: "link".to_string(),
            config 
        }
    }

    /// Renders a configuration file using the Tera engine.
    async fn render_template(&self, source_path: &Path) -> Result<String> {
        let content = self.executor.read_file(source_path).await?;
        
        let mut tera = Tera::default();
        tera.add_raw_template("config", &content)
            .map_err(|e| Error::Other(format!("Tera Parse Error in {:?}: {}", source_path, e)))?;

        let mut context = Context::new();
        context.insert("OS", std::env::consts::OS);
        context.insert("ARCH", std::env::consts::ARCH);
        context.insert("USER", &std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()));
        context.insert("HOSTNAME", &Config::get_hostname());

        context.insert("aliases", &self.config.aliases);
        context.insert("groups", &self.config.groups);

        tera.render("config", &context)
            .map_err(|e| Error::Other(format!("Tera Render Error in {:?}: {}", source_path, e)))
    }

    #[cfg(target_os = "windows")]
    fn is_same_drive(a: &Path, b: &Path) -> bool {
        use std::path::Component;
        let drive_a = a.components().find(|c| matches!(c, Component::Prefix(_)));
        let drive_b = b.components().find(|c| matches!(c, Component::Prefix(_)));
        drive_a == drive_b
    }
}

#[async_trait]
impl BackendCore for LinkBackendCore {
    fn name(&self) -> &str { &self.name }
    fn is_available(&self) -> bool { true }
    fn needs_root(&self) -> bool { false }
}

/// Phase 1.1: MetadataProvider for Link (Filesystem objects have no native transitive deps).
#[async_trait]
impl MetadataProvider for LinkBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct LinkInstallable {
    pub core: Arc<LinkBackendCore>,
}

#[async_trait]
impl Installable for LinkInstallable {
    /// Aligns the target file with the source (Link or Rendered Template).
    /// Hardened for Phase 1.1 Correction: Uses the abstracted executor.symlink() 
    /// to ensure dry-run VFS recording.
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            let source = PathBuf::from(&spec.name);
            let target_str = spec.options.get("target")
                .ok_or_else(|| Error::Other("Link requires @target".into()))?;
            
            let target_path = if target_str.starts_with('~') {
                dirs::home_dir().ok_or_else(|| Error::Other("Could not find home".into()))?
                    .join(&target_str[2..])
            } else {
                PathBuf::from(target_str)
            };

            // 1. Template Rendering Path
            if spec.options.get("template") == Some(&"true".to_string()) {
                let rendered = self.core.render_template(&source).await?;
                
                let needs_write = !matches!(
                    self.core.executor.read_file(&target_path).await,
                    Ok(existing) if existing == rendered
                );

                if needs_write {
                    info!("Link: Rendering template {:?} -> {:?}", source, target_path);
                    self.core.executor.write_atomic(&target_path, &rendered).await?;
                } else {
                    debug!("Link: Template {:?} is already up-to-date at {:?}", source, target_path);
                }
                continue;
            }

            // 2. Standard Symlinking Path
            let exists = tokio::fs::try_exists(&target_path).await.unwrap_or(false);
            let is_symlink = target_path.is_symlink();

            if exists || is_symlink {
                if let Ok(existing_link) = tokio::fs::read_link(&target_path).await {
                    if existing_link == source {
                        debug!("Link: Correct symlink already exists at {:?}", target_path);
                        continue;
                    }
                }

                if self.core.executor.dry_run {
                    info!("[DRY-RUN] Would remove existing file/link at {:?}", target_path);
                } else {
                    let metadata = tokio::fs::symlink_metadata(&target_path).await.map_err(Error::from)?;
                    if metadata.is_dir() && !metadata.is_symlink() {
                        tokio::fs::remove_dir_all(&target_path).await.map_err(Error::from)?;
                    } else {
                        tokio::fs::remove_file(&target_path).await.map_err(Error::from)?;
                    }
                }
            }

            info!("Link: Creating link {:?} -> {:?}", source, target_path);
            
            // Fulfills Phase 1.1 Correction: Delegate to executor to allow VFS recording in tests.
            #[cfg(target_os = "windows")]
            {
                let source_abs = source.canonicalize().unwrap_or_else(|_| source.clone());
                if !self.core.executor.dry_run && !LinkBackendCore::is_same_drive(&source_abs, &target_path) {
                    warn!("Link: Cross-drive fallback to COPY for {:?}", source);
                    tokio::fs::copy(&source, &target_path).await.map_err(Error::from)?;
                } else {
                    // This now handles dry-run automatically via VFS
                    self.core.executor.symlink(&source, &target_path).await?;
                }
            }

            #[cfg(unix)]
            {
                self.core.executor.symlink(&source, &target_path).await?;
            }
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
            let path = Path::new(name);
            let exists = tokio::fs::try_exists(path).await.unwrap_or(false);
            let is_symlink = path.is_symlink();

            if exists || is_symlink {
                info!("Link: Removing link/rendered file {:?}", path);
                if !self.core.executor.dry_run {
                    let metadata = tokio::fs::symlink_metadata(path).await.map_err(Error::from)?;
                    if metadata.is_dir() && !metadata.is_symlink() {
                        tokio::fs::remove_dir_all(path).await.map_err(Error::from)?;
                    } else {
                        tokio::fs::remove_file(path).await.map_err(Error::from)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Build and register the symlink/template backend.
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let core = Arc::new(LinkBackendCore::new(exec.duplicate(), Arc::new(cfg.clone())));
    reg.register(Arc::new(crate::core::BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(LinkInstallable { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}