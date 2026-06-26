use crate::core::{
    BackendCore, Installable, Package, PackageSpec, 
    Queryable, Result, Error, CommandExecutor, MetadataProvider
};
use async_trait::async_trait;
use std::path::Path;
use std::fs;
use std::sync::Arc;
use tracing::{info, debug};

/// Core backend implementation for managing BTRFS filesystem resources.
pub struct BtrfsBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl BtrfsBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self { 
            executor,
            name: "btrfs".to_string(),
        }
    }

    /// Helper to ensure quota groups (qgroups) are enabled.
    async fn ensure_qgroups(&self, path: &str, sudo: bool) -> Result<()> {
        debug!("BTRFS: Ensuring qgroups are enabled for {}", path);
        self.executor.run("btrfs", &["quota", "enable", path], sudo).await?;
        Ok(())
    }

    /// Discovers the UUID of the filesystem containing the given path.
    async fn get_fs_uuid(&self, path: &str) -> Result<String> {
        let output = self.executor.run_output("btrfs", &["filesystem", "show", path], false).await?;
        for line in output.lines() {
            if let Some(uuid_part) = line.trim().strip_prefix("uuid:") {
                return Ok(uuid_part.trim().to_string());
            }
        }
        Err(Error::Other(format!("Could not determine BTRFS UUID for {}", path)))
    }

    /// Manages /etc/fstab entries for declarative subvolume mounting.
    fn update_fstab(&self, uuid: &str, subvol_path: &str, mount_point: &str, options: &str) -> Result<()> {
        let fstab_path = Path::new("/etc/fstab");
        if !fstab_path.exists() {
            return Err(Error::Other("/etc/fstab not found".into()));
        }

        // fstab modification is synchronous and requires careful atomic handling
        let content = fs::read_to_string(fstab_path).map_err(Error::from)?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        
        lines.retain(|l| !l.contains(mount_point));

        let new_line = format!(
            "UUID={} {} btrfs defaults,subvol={},{} 0 0",
            uuid, mount_point, subvol_path, options
        );
        lines.push(new_line);

        let final_content = lines.join("\n") + "\n";
        crate::utils::file::atomic_write(fstab_path, &final_content)?;
        
        Ok(())
    }
}

#[async_trait]
impl BackendCore for BtrfsBackendCore {
    fn name(&self) -> &str { &self.name }

    fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && self.executor.command_exists_sync("btrfs")
    }

    fn needs_root(&self) -> bool {
        // Filesystem level modifications (subvolumes, mounts) require root.
        true
    }
}

#[async_trait]
impl MetadataProvider for BtrfsBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        // Subvolumes are standalone filesystem objects and do not have transitive package deps.
        Ok(vec![])
    }
}

pub struct BtrfsInstallable {
    pub core: Arc<BtrfsBackendCore>,
}

#[async_trait]
impl Installable for BtrfsInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let path = &spec.name;
            
            if !Path::new(path).exists() {
                info!("BTRFS: Creating subvolume at {}", path);
                self.core.executor.run("btrfs", &["subvolume", "create", path], sudo).await?;
            }

            if let Some(quota_size) = spec.options.get("quota") {
                let _ = self.core.ensure_qgroups(path, sudo).await;
                self.core.executor.run("btrfs", &["qgroup", "limit", quota_size, path], sudo).await?;
            }

            if let Some(mount_point) = spec.options.get("mount") {
                if !Path::new(mount_point).exists() {
                    self.core.executor.run("mkdir", &["-p", mount_point], sudo).await?;
                }

                let uuid = self.core.get_fs_uuid(path).await?;
                let custom_options = spec.options.get("options").map(|s| s.as_str()).unwrap_or("defaults");
                
                let core_ref = self.core.clone();
                let uuid_str = uuid.clone();
                let path_str = path.clone();
                let mount_str = mount_point.clone();
                let opt_str = custom_options.to_string();

                tokio::task::spawn_blocking(move || {
                    core_ref.update_fstab(&uuid_str, &path_str, &mount_str, &opt_str)
                }).await.map_err(|e| Error::Other(e.to_string()))??;

                self.core.executor.run("mount", &[mount_point], sudo).await?;
            }
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            if Path::new(name).exists() {
                info!("BTRFS: Deleting subvolume {}", name);
                self.core.executor.run("btrfs", &["subvolume", "delete", name], sudo).await?;
            }
        }
        Ok(())
    }
}

pub struct BtrfsQueryable {
    pub core: Arc<BtrfsBackendCore>,
}

#[async_trait]
impl Queryable for BtrfsQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("btrfs", &["subvolume", "list", "-p", "/"], false).await?;
        let mut packages = Vec::new();
        for line in output.lines() {
            if let Some(path_part) = line.split("path ").last() {
                let full_path = if path_part.starts_with('/') { path_part.to_string() } else { format!("/{}", path_part) };
                packages.push(Package::new(full_path, "btrfs"));
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        if !Path::new(name).exists() { return Ok(None); }
        let p = Package::new(name, "btrfs");
        Ok(Some(p))
    }
}

/// Build and register the BTRFS subvolume backend.
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(BtrfsBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(crate::core::BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(BtrfsInstallable { core: core.clone() }))
        .with_queryable(Arc::new(BtrfsQueryable { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}