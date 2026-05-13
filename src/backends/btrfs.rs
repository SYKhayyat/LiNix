use crate::core::{
    Backend, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Error
};
use async_trait::async_trait;
use std::path::Path;
use std::fs;
use tracing::{info, debug, warn};

/// Specialized backend for managing BTRFS filesystem resources.
/// Hardened for Version 3.4.0 to handle subvolume creation, quotas,
/// /etc/fstab persistence, and automated mounting.
/// 
/// Syntax: btrfs:/path/to/subvolume[@quota=20G,@mount=/mnt/data,@options=noatime]
pub struct BtrfsManager {
    executor: CommandExecutor,
}

impl BtrfsManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self { executor }
    }

    /// Helper to ensure quota groups (qgroups) are enabled.
    async fn ensure_qgroups(&self, path: &str) -> Result<()> {
        debug!("BTRFS: Ensuring qgroups are enabled for {}", path);
        self.executor.run("btrfs", &["quota", "enable", path], true).await?;
        Ok(())
    }

    /// Discovers the UUID of the filesystem containing the given path.
    async fn get_fs_uuid(&self, path: &str) -> Result<String> {
        let output = self.executor.run_output("btrfs", &["filesystem", "show", path], false).await?;
        // Format snippet: devid    1 size 465.76GiB used 300.00GiB path /dev/sda2
        // uuid: 12345678-1234-1234-1234-1234567890ab
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

        let content = fs::read_to_string(fstab_path)?;
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        
        // Remove existing entries for this mount point to ensure idempotency
        lines.retain(|l| !l.contains(mount_point));

        // Construct new fstab line
        // UUID=xxx /mount/point btrfs defaults,subvol=/path/to/subvol,options 0 0
        let new_line = format!(
            "UUID={} {} btrfs defaults,subvol={},{} 0 0",
            uuid, mount_point, subvol_path, options
        );
        lines.push(new_line);

        let final_content = lines.join("\n") + "\n";
        crate::utils::file::atomic_write(fstab_path, &final_content)?;
        
        Ok(())
    }

    /// Removes an entry from /etc/fstab.
    fn remove_from_fstab(&self, mount_point: &str) -> Result<()> {
        let fstab_path = Path::new("/etc/fstab");
        if !fstab_path.exists() { return Ok(()); }

        let content = fs::read_to_string(fstab_path)?;
        let lines: Vec<String> = content.lines()
            .filter(|l| !l.contains(mount_point))
            .map(|s| s.to_string())
            .collect();

        let final_content = lines.join("\n") + "\n";
        crate::utils::file::atomic_write(fstab_path, &final_content)
    }

    fn validate_size(&self, size: &str) -> Result<String> {
        if size.chars().all(|c| c.is_alphanumeric()) {
            Ok(size.to_string())
        } else {
            Err(Error::Validation(format!("Invalid BTRFS quota size: {}", size)))
        }
    }
}

impl Backend for BtrfsManager {
    fn name(&self) -> &str { "btrfs" }

    fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && self.executor.command_exists_sync("btrfs")
    }

    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
}

#[async_trait]
impl Installable for BtrfsManager {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let path = &spec.name;
            
            // 1. Ensure Subvolume Exists
            if !Path::new(path).exists() {
                info!("BTRFS: Creating subvolume at {}", path);
                self.executor.run("btrfs", &["subvolume", "create", path], sudo).await?;
            }

            // 2. Apply Quotas
            if let Some(quota_size) = spec.options.get("quota") {
                let validated_size = self.validate_size(quota_size)?;
                let _ = self.ensure_qgroups(path).await;
                self.executor.run("btrfs", &["qgroup", "limit", &validated_size, path], sudo).await?;
            }

            // 3. Automated Mounting & Persistence
            if let Some(mount_point) = spec.options.get("mount") {
                info!("BTRFS: Configuring mount for {} -> {}", path, mount_point);
                
                // Ensure mount point directory exists
                if !Path::new(mount_point).exists() {
                    self.executor.run("mkdir", &["-p", mount_point], sudo).await?;
                }

                let uuid = self.get_fs_uuid(path).await?;
                let custom_options = spec.options.get("options").map(|s| s.as_str()).unwrap_or("defaults");
                
                // Update /etc/fstab for persistence
                self.update_fstab(&uuid, path, mount_point, custom_options)?;

                // Immediate Mount
                debug!("BTRFS: Triggering immediate mount for {}", mount_point);
                self.executor.run("mount", &[mount_point], sudo).await?;
            }
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            // If the subvolume was mounted, unmount it first
            // We search info to see if we have mount metadata
            if let Ok(Some(pkg)) = self.info(name).await {
                if let Some(mp) = pkg.properties.get("mount_point") {
                    info!("BTRFS: Unmounting {}...", mp);
                    let _ = self.executor.run("umount", &[mp], sudo).await;
                    let _ = self.remove_from_fstab(mp);
                }
            }

            if Path::new(name).exists() {
                info!("BTRFS: Deleting subvolume {}", name);
                self.executor.run("btrfs", &["subvolume", "delete", name], sudo).await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Queryable for BtrfsManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.executor.run_output("btrfs", &["subvolume", "list", "-p", "/"], false).await?;
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

        let mut p = Package::new(name, "btrfs");
        
        // Find if currently mounted
        let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
        for line in mounts.lines() {
            if line.contains(&format!("subvol={}", name)) || line.contains(&format!("subvol=/{}", name)) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(mp) = parts.get(1) {
                    p.properties.insert("mount_point".into(), mp.to_string());
                    p.properties.insert("mounted".into(), "true".into());
                }
            }
        }

        if let Ok(show_out) = self.executor.run_output("btrfs", &["subvolume", "show", name], false).await {
            for line in show_out.lines() {
                let line = line.trim();
                if let Some(uuid) = line.strip_prefix("UUID:") {
                    p.properties.insert("uuid".into(), uuid.trim().to_string());
                }
            }
        }

        Ok(Some(p))
    }
}