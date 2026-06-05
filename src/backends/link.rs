use crate::core::{CommandExecutor, Result, PackageSpec, BackendCore, Installable, Error};
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

    #[cfg(windows)]
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
}

pub struct LinkInstallable {
    pub core: Arc<LinkBackendCore>,
}

#[async_trait]
impl Installable for LinkInstallable {
    /// Aligns the target file with the source (Link or Rendered Template).
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
                
                let needs_write = match self.core.executor.read_file(&target_path).await {
                    Ok(existing) if existing == rendered => false,
                    _ => true,
                };

                if needs_write {
                    info!("Link: Rendering template {:?} -> {:?}", source, target_path);
                    self.core.executor.write_atomic(&target_path, &rendered).await?;
                } else {
                    debug!("Link: Template {:?} is already up-to-date at {:?}", source, target_path);
                }
                continue;
            }

            // 2. Standard Symlinking Path
            if target_path.exists() || target_path.is_symlink() {
                if let Ok(existing_link) = std::fs::read_link(&target_path) {
                    if existing_link == source {
                        debug!("Link: Correct symlink already exists at {:?}", target_path);
                        continue;
                    }
                }

                if self.core.executor.dry_run {
                    info!("[DRY-RUN] Would remove existing file/link at {:?}", target_path);
                } else {
                    if target_path.is_dir() && !target_path.is_symlink() {
                        std::fs::remove_dir_all(&target_path)?;
                    } else {
                        std::fs::remove_file(&target_path)?;
                    }
                }
            }

            info!("Link: Creating link {:?} -> {:?}", source, target_path);
            if !self.core.executor.dry_run {
                if let Some(p) = target_path.parent() {
                    std::fs::create_dir_all(p)?;
                }

                #[cfg(unix)] {
                    std::os::unix::fs::symlink(&source, &target_path)?;
                }

                #[cfg(windows)] {
                    let source_abs = source.canonicalize().unwrap_or_else(|_| source.clone());
                    if !LinkBackendCore::is_same_drive(&source_abs, &target_path) {
                        warn!("Link: Cross-drive fallback to COPY for {:?}", source);
                        std::fs::copy(&source, &target_path)?;
                    } else if source.is_dir() {
                        std::os::windows::fs::symlink_dir(&source, &target_path)?;
                    } else {
                        std::os::windows::fs::symlink_file(&source, &target_path)?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
            let path = Path::new(name);
            if path.exists() || path.is_symlink() {
                info!("Link: Removing link/rendered file {:?}", path);
                if !self.core.executor.dry_run {
                    if path.is_dir() && !path.is_symlink() {
                        std::fs::remove_dir_all(path)?;
                    } else {
                        std::fs::remove_file(path)?;
                    }
                }
            }
        }
        Ok(())
    }
}