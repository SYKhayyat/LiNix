//! Declared storage objects: ZFS datasets and LVM logical volumes, one family (U30).
//!
//! `btrfs:` already declares a btrfs subvolume as an object; a ZFS dataset (`zfs create`) and an
//! LVM logical volume (`lvcreate`) are the same idea — a declared, sized, mounted storage object
//! — so they are one family rather than three unrelated backends. They are Rust rather than a
//! `ManagerConfig` because they are not argv-with-`{name}={version}`: a volume has a size and a
//! mountpoint, not a version.
//!
//! **The safety edge U30 turns on: a `remove` here destroys a filesystem and everything on it.**
//! `zfs destroy` and `lvremove` are not "uninstall a package"; they are "erase a disk". Both are
//! ordinary backends, so their removals run through the **normal** sync guard with no special
//! escalation — which is exactly the point: a declared volume is protectable like a package
//! (`[guard] protected_packages` matches `zfs:tank/data`), it counts against `max_removals`, and
//! deleting the line previews the destruction before the guard lets it proceed. A storage backend
//! that ran its own removal outside the guard would be the teleport bug (V-lesson 2026-07-17) with
//! a filesystem on the end of it.

use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result,
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

// --- ZFS datasets -----------------------------------------------------------------------------

/// The argv to create a ZFS dataset. Pure, so the command is unit-testable without a pool.
fn zfs_create(name: &str) -> Vec<String> {
    vec!["create".into(), name.into()]
}

/// `zfs set quota=<size> <name>` — a property, set after create.
fn zfs_set(property: &str, value: &str, name: &str) -> Vec<String> {
    vec!["set".into(), format!("{}={}", property, value), name.into()]
}

/// `zfs list -H -o name,mountpoint`, as datasets and where each is mounted.
///
/// `-H` separates columns with a tab and nothing else, so the split is on `\t`: a mountpoint is
/// a path and may contain spaces.
///
/// A dataset reporting `none`, `legacy` or `-` has **no mountpoint ZFS is managing**, and that is
/// recorded as no property at all rather than as those literal words — a declaration asking for
/// `@mount=/srv` against a dataset ZFS is not mounting is unsatisfied, and the planner has to be
/// able to see that. Reporting `legacy` as if it were a path would make the comparison false and
/// re-run `zfs set mountpoint=` on every sync.
fn parse_zfs_list(output: &str) -> Vec<Package> {
    output
        .lines()
        .filter_map(|line| {
            let mut cols = line.split('\t');
            let name = cols.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let mut p = Package::new(name, "zfs");
            if let Some(m) = cols
                .next()
                .map(str::trim)
                .filter(|m| !matches!(*m, "" | "-" | "none" | "legacy"))
            {
                p.properties.insert("mount".into(), m.to_string());
            }
            Some(p)
        })
        .collect()
}

/// `zfs destroy -r <name>` — destroys the dataset and its children. The dangerous verb; it runs
/// only after the sync guard has cleared the removal.
fn zfs_destroy(name: &str) -> Vec<String> {
    vec!["destroy".into(), "-r".into(), name.into()]
}

pub struct ZfsBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl ZfsBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "zfs".to_string(),
        }
    }
    async fn run(&self, args: &[String], sudo: bool) -> Result<()> {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.executor.run("zfs", &refs, sudo).await.map(|_| ())
    }
}

#[async_trait]
impl BackendCore for ZfsBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("zfs")
    }
    fn probes(&self) -> Vec<String> {
        vec!["zfs".into()]
    }
    fn needs_root(&self) -> bool {
        true
    }
}

#[async_trait]
impl MetadataProvider for ZfsBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct ZfsInstallable {
    pub core: Arc<ZfsBackendCore>,
}

#[async_trait]
impl Installable for ZfsInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let name = &spec.name;
            // Create only if absent, so a sync is idempotent — `zfs list` answers existence.
            let exists = self
                .core
                .executor
                .run_output("zfs", &["list", "-H", "-o", "name", name], false)
                .await
                .map(|o| !o.trim().is_empty())
                .unwrap_or(false);
            if !exists {
                info!("ZFS: creating dataset {}", name);
                self.core.run(&zfs_create(name), sudo).await?;
            }
            if let Some(quota) = spec.options.get("quota") {
                self.core.run(&zfs_set("quota", quota, name), sudo).await?;
            }
            if let Some(mount) = spec.options.get("mount") {
                self.core
                    .run(&zfs_set("mountpoint", mount, name), sudo)
                    .await?;
            }
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            info!("ZFS: destroying dataset {}", name);
            self.core.run(&zfs_destroy(name), sudo).await?;
        }
        Ok(())
    }
}

pub struct ZfsQueryable {
    pub core: Arc<ZfsBackendCore>,
}

#[async_trait]
impl Queryable for ZfsQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        // `mountpoint` rides along on the listing that already runs, so `sync` can tell a
        // declared `@mount=` that took effect from one that did not.
        let out = self
            .core
            .executor
            .run_output("zfs", &["list", "-H", "-o", "name,mountpoint"], false)
            .await?;
        Ok(parse_zfs_list(&out))
    }
    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }
    async fn info(&self, name: &str) -> Result<Option<Package>> {
        Ok(self
            .list_installed()
            .await?
            .into_iter()
            .find(|p| p.name == name))
    }
}

// --- LVM logical volumes ----------------------------------------------------------------------

/// Split `vg/lv` into its volume group and logical volume. An LVM object is named `group/volume`,
/// the one spelling both `lvcreate` and `lvremove` accept via their own conventions.
fn split_lvm(name: &str) -> Result<(&str, &str)> {
    name.split_once('/')
        .filter(|(vg, lv)| !vg.is_empty() && !lv.is_empty())
        .ok_or_else(|| {
            Error::Validation(format!(
                "`lvm:{}` is not a volume — name it `group/volume`, e.g. `lvm:vg0/data`.",
                name
            ))
        })
}

/// `lvcreate -n <lv> -L <size> <vg>` — a logical volume needs a size; without one there is
/// nothing to create, which is a refusal, not a guess.
fn lvm_create(vg: &str, lv: &str, size: &str) -> Vec<String> {
    vec!["-n".into(), lv.into(), "-L".into(), size.into(), vg.into()]
}

/// `lvremove -y <vg>/<lv>` — destroys the volume. The dangerous verb, run only past the guard.
fn lvm_remove(vg: &str, lv: &str) -> Vec<String> {
    vec!["-y".into(), format!("{}/{}", vg, lv)]
}

pub struct LvmBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl LvmBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "lvm".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for LvmBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("lvs")
    }
    /// `lvs`, not `lvm`. The default message said "Binary for lvm not found in PATH" and named
    /// a program this backend never looks for.
    fn probes(&self) -> Vec<String> {
        vec!["lvs".into()]
    }
    fn needs_root(&self) -> bool {
        true
    }
}

#[async_trait]
impl MetadataProvider for LvmBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct LvmInstallable {
    pub core: Arc<LvmBackendCore>,
}

#[async_trait]
impl Installable for LvmInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let (vg, lv) = split_lvm(&spec.name)?;
            let exists = self
                .core
                .executor
                .run_output("lvs", &["--noheadings", "-o", "lv_name", vg], false)
                .await
                .map(|o| o.lines().any(|l| l.trim() == lv))
                .unwrap_or(false);
            if exists {
                continue;
            }
            let Some(size) = spec.options.get("size") else {
                return Err(Error::Validation(format!(
                    "`lvm:{}` has no `size` — a logical volume needs one to be created, e.g. \
                     `lvm:{}@size=10G`.",
                    spec.name, spec.name
                )));
            };
            info!("LVM: creating logical volume {}/{} ({})", vg, lv, size);
            let args = lvm_create(vg, lv, size);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core.executor.run("lvcreate", &refs, sudo).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            let (vg, lv) = split_lvm(name)?;
            info!("LVM: removing logical volume {}/{}", vg, lv);
            let args = lvm_remove(vg, lv);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core.executor.run("lvremove", &refs, sudo).await?;
        }
        Ok(())
    }
}

pub struct LvmQueryable {
    pub core: Arc<LvmBackendCore>,
}

#[async_trait]
impl Queryable for LvmQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self
            .core
            .executor
            .run_output("lvs", &["--noheadings", "-o", "vg_name,lv_name"], false)
            .await?;
        let mut pkgs = Vec::new();
        for line in out.lines() {
            let mut cols = line.split_whitespace();
            if let (Some(vg), Some(lv)) = (cols.next(), cols.next()) {
                pkgs.push(Package::new(format!("{}/{}", vg, lv), "lvm"));
            }
        }
        Ok(pkgs)
    }
    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }
    async fn info(&self, name: &str) -> Result<Option<Package>> {
        Ok(self
            .list_installed()
            .await?
            .into_iter()
            .find(|p| p.name == name))
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let zfs = Arc::new(ZfsBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(zfs.clone())
            .with_installable(Arc::new(ZfsInstallable { core: zfs.clone() }))
            .with_queryable(Arc::new(ZfsQueryable { core: zfs.clone() }))
            .with_metadata_provider(zfs.clone())
            .build(),
    ));

    let lvm = Arc::new(LvmBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(lvm.clone())
            .with_installable(Arc::new(LvmInstallable { core: lvm.clone() }))
            .with_queryable(Arc::new(LvmQueryable { core: lvm.clone() }))
            .with_metadata_provider(lvm.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zfs_creates_and_destroys_by_name() {
        assert_eq!(zfs_create("tank/data"), vec!["create", "tank/data"]);
        // Destroy is recursive: a dataset with children is one object, and the guard cleared the
        // whole of it.
        assert_eq!(zfs_destroy("tank/data"), vec!["destroy", "-r", "tank/data"]);
    }

    #[test]
    fn zfs_properties_become_set_commands() {
        assert_eq!(
            zfs_set("quota", "10G", "tank/data"),
            vec!["set", "quota=10G", "tank/data"]
        );
        assert_eq!(
            zfs_set("mountpoint", "/mnt/data", "tank/data"),
            vec!["set", "mountpoint=/mnt/data", "tank/data"]
        );
    }

    /// The listing carries where each dataset is mounted, so `sync` can tell a `@mount=` that
    /// took effect from one that did not. `legacy`, `none` and `-` are ZFS saying it mounts this
    /// nowhere, and none of them is a path — reporting one as if it were would leave a declared
    /// mountpoint looking satisfied by a word.
    #[test]
    fn the_dataset_listing_reads_mountpoints_and_knows_when_there_is_none() {
        let pkgs = parse_zfs_list(
            "tank\t/tank\n\
             tank/data\t/mnt/data\n\
             tank/legacy\tlegacy\n\
             tank/hidden\tnone\n\
             tank/blank\t-\n\
             tank/spaced\t/mnt/my data\n\
             \n",
        );
        let at = |n: &str| {
            pkgs.iter()
                .find(|p| p.name == n)
                .unwrap_or_else(|| panic!("{} was not listed", n))
                .properties
                .get("mount")
                .map(String::as_str)
        };
        assert_eq!(pkgs.len(), 6, "a blank line is not a dataset");
        assert_eq!(at("tank/data"), Some("/mnt/data"));
        assert_eq!(
            at("tank/spaced"),
            Some("/mnt/my data"),
            "tab-separated, not whitespace"
        );
        // Three spellings of "ZFS is not mounting this". None is a path, so none is reported as
        // one — a declared `@mount=` against any of them is unsatisfied, which is what the
        // planner needs to see.
        assert_eq!(at("tank/legacy"), None);
        assert_eq!(at("tank/hidden"), None);
        assert_eq!(at("tank/blank"), None);
    }

    #[test]
    fn lvm_name_splits_into_group_and_volume() {
        assert_eq!(split_lvm("vg0/data").unwrap(), ("vg0", "data"));
        // A name that is not `group/volume` is refused, not guessed at.
        assert!(split_lvm("data").is_err());
        assert!(split_lvm("vg0/").is_err());
        assert!(split_lvm("/data").is_err());
    }

    #[test]
    fn lvm_create_needs_a_group_size_and_volume() {
        assert_eq!(
            lvm_create("vg0", "data", "10G"),
            vec!["-n", "data", "-L", "10G", "vg0"]
        );
    }

    #[test]
    fn lvm_remove_confirms_and_names_the_path() {
        assert_eq!(lvm_remove("vg0", "data"), vec!["-y", "vg0/data"]);
    }
}
