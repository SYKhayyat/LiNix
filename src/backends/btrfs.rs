use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result,
};
use async_trait::async_trait;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info};

pub struct BtrfsBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    /// The kernel's mount table. A field so a test can hand it a fixture — the alternative is
    /// a check that can only run on a Linux box that already has btrfs, which is every box
    /// this backend was never tested on.
    pub mounts_file: std::path::PathBuf,
}

/// Every btrfs filesystem in a mount table, as `(mount point, subvolume prefix)`.
///
/// The prefix is the `subvol=` mount option, and it is not decoration: `btrfs subvolume list`
/// reports a path relative to the *filesystem* root, while `install` was handed a path on the
/// *mounted* tree. On a root mounted at `subvol=/@`, the two never name the same thing without
/// it — and a name `list` reports differently from the one `install` was given is a package
/// `sync` re-creates on every run, for ever.
fn btrfs_mounts_in(table: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in table.lines() {
        let mut f = line.split_whitespace();
        let (_dev, point, fstype, opts) = (f.next(), f.next(), f.next(), f.next());
        if fstype != Some("btrfs") {
            continue;
        }
        let Some(point) = point else { continue };
        // /proc/mounts octal-escapes whitespace in the mount point.
        let point = point.replace("\\040", " ").replace("\\011", "\t");
        let prefix = opts
            .unwrap_or("")
            .split(',')
            .find_map(|o| o.strip_prefix("subvol="))
            .unwrap_or("/")
            .trim_start_matches('/')
            .to_string();
        out.push((point, prefix));
    }
    out.sort();
    out.dedup();
    out
}

/// One `btrfs subvolume list` report, turned into names `install` would accept.
fn subvolume_paths(mount: &str, prefix: &str, output: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in output.lines() {
        let Some(rel) = line.split(" path ").nth(1) else {
            continue;
        };
        let rel = rel.trim();
        // Only what is reachable *through this mount*. On a filesystem mounted at
        // `subvol=/@`, a sibling subvolume `@home` exists and has no path from here, so
        // reporting it would name something no verb could act on.
        //
        // A PATH prefix, not a string prefix: `@home` starts with `@` and is not under it.
        // My first version used `strip_prefix` alone and reported the sibling as `/home`,
        // which is a real directory on the same machine and the wrong one.
        let under = if prefix.is_empty() {
            rel
        } else if rel == prefix {
            ""
        } else if let Some(rest) = rel.strip_prefix(prefix).filter(|r| r.starts_with('/')) {
            rest
        } else {
            continue;
        };
        let under = under.trim_start_matches('/');
        out.push(if under.is_empty() {
            mount.to_string()
        } else {
            format!("{}/{}", mount.trim_end_matches('/'), under)
        });
    }
    out
}

impl BtrfsBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "btrfs".to_string(),
            mounts_file: std::path::PathBuf::from("/proc/mounts"),
        }
    }

    fn btrfs_mounts(&self) -> Vec<(String, String)> {
        std::fs::read_to_string(&self.mounts_file)
            .map(|t| btrfs_mounts_in(&t))
            .unwrap_or_default()
    }

    async fn ensure_qgroups(&self, path: &str, sudo: bool) -> Result<()> {
        debug!("BTRFS: Ensuring qgroups are enabled for {}", path);
        self.executor
            .run("btrfs", &["quota", "enable", path], sudo)
            .await?;
        Ok(())
    }

    async fn get_fs_uuid(&self, path: &str) -> Result<String> {
        let output = self
            .executor
            .run_output("btrfs", &["filesystem", "show", path], false)
            .await?;
        for line in output.lines() {
            if let Some(uuid_part) = line.trim().strip_prefix("uuid:") {
                return Ok(uuid_part.trim().to_string());
            }
        }
        Err(Error::Other(format!(
            "Could not determine BTRFS UUID for {}",
            path
        )))
    }

    /// Manages /etc/fstab entries for declarative subvolume mounting.
    fn update_fstab(
        &self,
        uuid: &str,
        subvol_path: &str,
        mount_point: &str,
        options: &str,
    ) -> Result<()> {
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
        crate::utils::file::persist(fstab_path, &final_content)?;

        Ok(())
    }
}

#[async_trait]
impl BackendCore for BtrfsBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && self.executor.command_exists_sync("btrfs")
    }
    fn probes(&self) -> Vec<String> {
        vec!["btrfs".into()]
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
                self.core
                    .executor
                    .run("btrfs", &["subvolume", "create", path], sudo)
                    .await?;
            }

            if let Some(quota_size) = spec.options.get("quota") {
                let _ = self.core.ensure_qgroups(path, sudo).await;
                self.core
                    .executor
                    .run("btrfs", &["qgroup", "limit", quota_size, path], sudo)
                    .await?;
            }

            if let Some(mount_point) = spec.options.get("mount") {
                if !Path::new(mount_point).exists() {
                    self.core
                        .executor
                        .run("mkdir", &["-p", mount_point], sudo)
                        .await?;
                }

                let uuid = self.core.get_fs_uuid(path).await?;
                let custom_options = spec
                    .options
                    .get("options")
                    .map(|s| s.as_str())
                    .unwrap_or("defaults");

                let core_ref = self.core.clone();
                let uuid_str = uuid.clone();
                let path_str = path.clone();
                let mount_str = mount_point.clone();
                let opt_str = custom_options.to_string();

                tokio::task::spawn_blocking(move || {
                    core_ref.update_fstab(&uuid_str, &path_str, &mount_str, &opt_str)
                })
                .await
                .map_err(|e| Error::Other(e.to_string()))??;

                self.core
                    .executor
                    .run("mount", &[mount_point], sudo)
                    .await?;
            }
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            if Path::new(name).exists() {
                info!("BTRFS: Deleting subvolume {}", name);
                self.core
                    .executor
                    .run("btrfs", &["subvolume", "delete", name], sudo)
                    .await?;
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
        let mut packages = Vec::new();
        for (point, prefix) in self.core.btrfs_mounts() {
            let Ok(output) = self
                .core
                .executor
                .run_output("btrfs", &["subvolume", "list", &point], false)
                .await
            else {
                // One filesystem this process cannot read is not "no subvolumes anywhere".
                // The others still answer, and the alternative — a `?` here — is how asking
                // `/` on a machine with an ext4 root made this whole backend report an error.
                continue;
            };
            for name in subvolume_paths(&point, &prefix, &output) {
                packages.push(Package::new(name, "btrfs"));
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        if !Path::new(name).exists() {
            return Ok(None);
        }
        let p = Package::new(name, "btrfs");
        Ok(Some(p))
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(BtrfsBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(BtrfsInstallable { core: core.clone() }))
            .with_queryable(Arc::new(BtrfsQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::executor::{DryRunOutput, MockExecutor};
    use dashmap::DashMap;

    /// A core whose mount table is a fixture and whose `btrfs` calls are canned.
    fn core_with(mounts: &str, responses: &[(&str, &str)]) -> BtrfsBackendCore {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        for (cmd, out) in responses {
            mock.set_response(
                cmd,
                Ok(DryRunOutput {
                    stdout: out.as_bytes().to_vec(),
                    stderr: vec![],
                }
                .into()),
            );
        }
        let exec = CommandExecutor::with_layer(true, false, mock, vfs, Arc::new(DashMap::new()));
        // A distinct file per test: the tests run in parallel in one process.
        let f = std::env::temp_dir().join(format!(
            "linix-btrfs-mounts-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&f, mounts).expect("fixture mount table");
        BtrfsBackendCore {
            executor: exec,
            name: "btrfs".to_string(),
            mounts_file: f,
        }
    }

    /// `list_installed` asked `btrfs subvolume list -p /` until 2026-07-30 — one filesystem,
    /// the one mounted at `/`, whatever the declaration said.
    ///
    /// So `btrfs:/mnt/data/vol` was created by `install` and never seen by `list`, and on a
    /// machine whose root is not btrfs the query failed outright. A name `list` does not return
    /// is a package `sync` believes is absent: it re-creates it on every run, for ever. The
    /// backend was excused from every harness as "a snapshot provider, not an install target",
    /// which is why nothing ever noticed.
    #[tokio::test]
    async fn a_subvolume_is_listed_by_the_path_install_was_given() {
        let core = core_with(
            "/dev/sda1 / ext4 rw,relatime 0 0\n\
             /dev/sdb1 /mnt/data btrfs rw,relatime,subvol=/ 0 0\n",
            &[(
                "btrfs subvolume list /mnt/data",
                "ID 256 gen 8 top level 5 path vol\n",
            )],
        );
        let q = BtrfsQueryable {
            core: Arc::new(core),
        };
        let names: Vec<String> = q
            .list_installed()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            names,
            vec!["/mnt/data/vol".to_string()],
            "`install` was handed /mnt/data/vol, so `list` has to say the same string"
        );
    }

    /// The `subvol=` offset, which is the normal layout on a btrfs root (openSUSE, Fedora and
    /// Garuda all mount `/` at `subvol=/@`). Without it every name is wrong by one component.
    #[tokio::test]
    async fn a_mount_at_a_subvolume_reports_paths_relative_to_that_mount() {
        let core = core_with(
            "/dev/sda2 / btrfs rw,relatime,subvol=/@ 0 0\n",
            &[(
                "btrfs subvolume list /",
                "ID 256 gen 8 top level 5 path @\n\
                 ID 257 gen 9 top level 256 path @/srv\n\
                 ID 258 gen 9 top level 5 path @home\n",
            )],
        );
        let q = BtrfsQueryable {
            core: Arc::new(core),
        };
        let names: Vec<String> = q
            .list_installed()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        // `@` is the mount itself; `@/srv` is under it; `@home` is a sibling subvolume with no
        // path through this mount and must not be named, because no verb could act on it.
        assert_eq!(names, vec!["/".to_string(), "/srv".to_string()]);
    }

    /// Two filesystems. Asking `/` answered about one of them and silently omitted the other.
    #[tokio::test]
    async fn every_mounted_btrfs_is_asked_not_just_the_root() {
        let core = core_with(
            "/dev/sda2 / btrfs rw,subvol=/ 0 0\n\
             /dev/sdb1 /mnt/tank btrfs rw,subvol=/ 0 0\n",
            &[
                (
                    "btrfs subvolume list /",
                    "ID 256 gen 8 top level 5 path a\n",
                ),
                (
                    "btrfs subvolume list /mnt/tank",
                    "ID 256 gen 8 top level 5 path b\n",
                ),
            ],
        );
        let q = BtrfsQueryable {
            core: Arc::new(core),
        };
        let mut names: Vec<String> = q
            .list_installed()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        names.sort();
        assert_eq!(names, vec!["/a".to_string(), "/mnt/tank/b".to_string()]);
    }

    /// A mount table with no btrfs in it is not an error and not a subvolume — it is nothing.
    /// This is the case on every Windows and macOS host that runs this suite.
    #[test]
    fn a_table_with_no_btrfs_yields_no_mounts() {
        assert!(btrfs_mounts_in("/dev/sda1 / ext4 rw 0 0\nproc /proc proc rw 0 0\n").is_empty());
        assert_eq!(
            btrfs_mounts_in("/dev/sdb1 /mnt/my\\040disk btrfs rw,subvol=/sub 0 0\n"),
            vec![("/mnt/my disk".to_string(), "sub".to_string())],
            "an escaped space in a mount point is a space, and the subvol= prefix loses its slash"
        );
    }
}
