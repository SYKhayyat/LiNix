use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result,
};
use async_trait::async_trait;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub struct BtrfsBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    /// The kernel's mount table. A field so a test can hand it a fixture — the alternative is
    /// a check that can only run on a Linux box that already has btrfs, which is every box
    /// this backend was never tested on.
    pub mounts_file: std::path::PathBuf,
    /// Where a declared mount is recorded. A field for the same reason, and the stakes are
    /// higher: this is the file a wrong edit stops the machine booting from.
    pub fstab_file: std::path::PathBuf,
}

/// One btrfs filesystem, as the mount table describes it.
///
/// `prefix` is the `subvol=` mount option, and it is not decoration: `btrfs subvolume list`
/// reports a path relative to the *filesystem* root, while `install` was handed a path on the
/// *mounted* tree. On a root mounted at `subvol=/@`, the two never name the same thing without
/// it — and a name `list` reports differently from the one `install` was given is a package
/// `sync` re-creates on every run, for ever.
///
/// `device` is what tells one filesystem from another. Two btrfs filesystems can each hold a
/// subvolume called `data`, and one filesystem can be mounted twice — so a subvolume's identity
/// is the device it lives on plus its path from that filesystem's root, never the path alone.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BtrfsMount {
    device: String,
    point: String,
    prefix: String,
}

/// Every btrfs filesystem in a mount table.
fn btrfs_mounts_in(table: &str) -> Vec<BtrfsMount> {
    let mut out = Vec::new();
    for line in table.lines() {
        let mut f = line.split_whitespace();
        let (dev, point, fstype, opts) = (f.next(), f.next(), f.next(), f.next());
        if fstype != Some("btrfs") {
            continue;
        }
        let (Some(dev), Some(point)) = (dev, point) else {
            continue;
        };
        // /proc/mounts octal-escapes whitespace in the mount point.
        let point = point.replace("\\040", " ").replace("\\011", "\t");
        let prefix = opts
            .unwrap_or("")
            .split(',')
            .find_map(|o| o.strip_prefix("subvol="))
            .unwrap_or("/")
            .trim_start_matches('/')
            .to_string();
        out.push(BtrfsMount {
            device: dev.to_string(),
            point,
            prefix,
        });
    }
    out.sort();
    out.dedup();
    out
}

/// One `btrfs subvolume list` report, as `(path from the filesystem root, name `install` would
/// accept)`. The first is the identity, the second is what a user reads.
fn subvolume_paths(mount: &str, prefix: &str, output: &str) -> Vec<(String, String)> {
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
        let name = if under.is_empty() {
            mount.to_string()
        } else {
            format!("{}/{}", mount.trim_end_matches('/'), under)
        };
        out.push((rel.trim_start_matches('/').to_string(), name));
    }
    out
}

/// The `subvol=` an fstab entry must carry for a subvolume declared at `path`, and the mount
/// point it hangs under.
///
/// The inverse of [`subvolume_paths`], and it exists for the same reason: fstab names a
/// subvolume relative to the *filesystem* root, while a declaration names it on the *mounted*
/// tree. An entry built from the declared path mounts nothing on a filesystem whose own root is
/// a subvolume — there is no `subvol=/mnt/data/srv`; `subvol=/@/srv` is that same object.
///
/// Returns the `subvol=` **and the mount point of the filesystem holding it**. The second is
/// not a convenience: `btrfs filesystem show` answers for a *filesystem* and refuses a
/// subvolume — `not a valid btrfs filesystem: /mnt/fs/canary`, measured — so the UUID an fstab
/// entry needs can only be asked for at the mount point, never at the declared path.
///
/// `None` when the path is not under any btrfs mount, and when it *is* one: the root of a
/// mounted filesystem is not a subvolume this backend created, and writing an fstab entry for
/// it would rewrite the line that mounts the machine.
fn subvol_arg(mounts: &[BtrfsMount], path: &str) -> Option<(String, String)> {
    let path = path.trim_end_matches('/');
    let mount = mounts
        .iter()
        .filter(|m| {
            path.strip_prefix(m.point.trim_end_matches('/'))
                .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|m| m.point.trim_end_matches('/').len())?;
    let rest = path
        .strip_prefix(mount.point.trim_end_matches('/'))?
        .trim_start_matches('/');
    let prefix = mount.prefix.trim_matches('/');
    let subvol = if prefix.is_empty() {
        format!("/{}", rest)
    } else {
        format!("/{}/{}", prefix, rest)
    };
    Some((subvol, mount.point.clone()))
}

/// The filesystem UUID in a `btrfs filesystem show` report.
///
/// It is a token *within* the first line, not the start of one: the real report reads
/// `Label: none  uuid: 3b5f…`. This looked for a line beginning `uuid:`, found none, and failed
/// every time — which nobody saw, because the only caller is the `@mount` path and no
/// declaration could carry `@mount` until Q18 was ruled.
fn fs_uuid_in(output: &str) -> Option<String> {
    let mut fields = output.split_whitespace();
    while let Some(field) = fields.next() {
        if field == "uuid:" {
            return fields.next().map(str::to_string);
        }
        if let Some(uuid) = field.strip_prefix("uuid:").filter(|u| !u.is_empty()) {
            return Some(uuid.to_string());
        }
    }
    None
}

/// The mount point field of one fstab line, or `None` for a comment or a blank.
fn fstab_mount_point(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    line.split_whitespace().nth(1)
}

/// The `subvol=` of one fstab line, when the line mounts btrfs.
fn fstab_subvol(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut fields = line.split_whitespace();
    if fields.nth(2) != Some("btrfs") {
        return None;
    }
    fields
        .next()?
        .split(',')
        .find_map(|o| o.strip_prefix("subvol="))
}

/// The option field of this subvolume's fstab entry, without the `subvol=` that addresses it.
///
/// The inverse of what [`fstab_with`] writes, so a declared `@mount_options` can be compared
/// against the entry on disk (Q19). Without it a changed `@mount_options` is invisible: the
/// subvolume exists and is mounted where the line says, so `sync` finds nothing to do and the
/// old options survive every run — the same shape as a `@quota` that never re-applies, one file
/// over.
fn fstab_options(content: &str, subvol: &str) -> Option<String> {
    let line = content.lines().find(|l| fstab_subvol(l) == Some(subvol))?;
    let field = line.split_whitespace().nth(3)?;
    let rest: Vec<&str> = field
        .split(',')
        .filter(|o| !o.starts_with("subvol="))
        .collect();
    Some(rest.join(","))
}

/// The referenced-size limit in a `btrfs qgroup show -r -f --raw` report, in bytes.
///
/// The column is found by its header rather than by counting: `btrfs-progs` has added columns
/// across versions, and a hardcoded index would silently read `excl` on the version that did.
/// `none` is btrfs saying there is no limit, and it is not a number — reporting it as one would
/// make a declared `@quota=` compare against a word.
fn qgroup_limit_in(output: &str) -> Option<u64> {
    let mut lines = output.lines();
    let column = lines
        .by_ref()
        .find(|l| l.split_whitespace().any(|c| c == "max_rfer"))?
        .split_whitespace()
        .position(|c| c == "max_rfer")?;
    lines
        .filter(|l| !l.trim_start().starts_with("---"))
        .find_map(|l| {
            l.split_whitespace()
                .nth(column)
                .and_then(|v| v.parse().ok())
        })
}

/// `content` with one entry for this subvolume and no other.
///
/// Both matches are dropped and not just one: a declaration that moves a subvolume to a new
/// mount point must leave no entry at the old one, and a declaration that puts a different
/// subvolume at an occupied mount point must not leave two lines fighting over it.
///
/// **Field-wise, never `contains`.** The line this replaced dropped every fstab line with the
/// mount point anywhere in it, so declaring `/mnt` would have deleted `/mnt/data`, `/mnt/home`
/// and any comment that mentioned the path.
fn fstab_with(content: &str, uuid: &str, subvol: &str, mount_point: &str, options: &str) -> String {
    let mut lines: Vec<&str> = content
        .lines()
        .filter(|l| fstab_mount_point(l) != Some(mount_point) && fstab_subvol(l) != Some(subvol))
        .collect();
    let entry = format!(
        "UUID={} {} btrfs subvol={},{} 0 0",
        uuid, mount_point, subvol, options
    );
    lines.push(&entry);
    lines.join("\n") + "\n"
}

/// `content` with this subvolume's entry gone, and the mount point it named.
///
/// Removal has to find the entry by subvolume, because a `remove` is handed names and never the
/// options the line carried. Leaving it behind would be worse than untidy: the next boot would
/// try to mount a subvolume that no longer exists, and a machine that fails to mount an fstab
/// entry stops in the initramfs rather than starting.
fn fstab_without(content: &str, subvol: &str) -> (String, Option<String>) {
    let point = content
        .lines()
        .find(|l| fstab_subvol(l) == Some(subvol))
        .and_then(fstab_mount_point)
        .map(str::to_string);
    let kept: Vec<&str> = content
        .lines()
        .filter(|l| fstab_subvol(l) != Some(subvol))
        .collect();
    let out = if kept.is_empty() {
        String::new()
    } else {
        kept.join("\n") + "\n"
    };
    (out, point)
}

impl BtrfsBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "btrfs".to_string(),
            mounts_file: std::path::PathBuf::from("/proc/mounts"),
            fstab_file: std::path::PathBuf::from("/etc/fstab"),
        }
    }

    fn btrfs_mounts(&self) -> Vec<BtrfsMount> {
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
        fs_uuid_in(&output).ok_or_else(|| {
            Error::Other(format!(
                "could not read the btrfs filesystem UUID for {} — an fstab entry needs it, \
                 and `btrfs filesystem show` did not report one",
                path
            ))
        })
    }

    /// The `subvol=` for a declared path and the filesystem it lives on, refusing rather than
    /// guessing.
    fn subvol_for(&self, path: &str) -> Result<(String, String)> {
        subvol_arg(&self.btrfs_mounts(), path).ok_or_else(|| {
            Error::Validation(format!(
                "`btrfs:{}` is not inside a mounted btrfs filesystem, so there is no subvolume \
                 to mount. Mount the filesystem first, and name a path under it.",
                path
            ))
        })
    }

    /// The declared entry, written into fstab so the mount survives a reboot.
    fn update_fstab(
        &self,
        uuid: &str,
        subvol: &str,
        mount_point: &str,
        options: &str,
    ) -> Result<()> {
        // A machine with no fstab yet is one whose fstab is empty, not an error: the declaration
        // is the first entry rather than a reason to refuse the mount.
        let content = fs::read_to_string(&self.fstab_file).unwrap_or_default();
        let updated = fstab_with(&content, uuid, subvol, mount_point, options);
        crate::utils::file::persist(&self.fstab_file, &updated)?;
        Ok(())
    }

    /// Where this subvolume is mounted right now, by the kernel's account — every place, because
    /// btrfs will happily mount one subvolume at several.
    fn current_mounts_of(&self, subvol: &str) -> Vec<String> {
        let subvol = subvol.trim_matches('/');
        self.btrfs_mounts()
            .into_iter()
            .filter(|m| m.prefix.trim_matches('/') == subvol)
            .map(|m| m.point)
            .collect()
    }

    /// Drops this subvolume's entry, and says where it had been mounted.
    fn drop_from_fstab(&self, subvol: &str) -> Result<Option<String>> {
        let Ok(content) = fs::read_to_string(&self.fstab_file) else {
            return Ok(None);
        };
        let (updated, point) = fstab_without(&content, subvol);
        if updated != content {
            crate::utils::file::persist(&self.fstab_file, &updated)?;
        }
        Ok(point)
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

            if let Some(quota_size) = spec.options.one("quota") {
                let _ = self.core.ensure_qgroups(path, sudo).await;
                self.core
                    .executor
                    .run("btrfs", &["qgroup", "limit", quota_size, path], sudo)
                    .await?;
            }

            if let Some(mount_point) = spec.options.one("mount") {
                if !Path::new(mount_point).exists() {
                    self.core
                        .executor
                        .run("mkdir", &["-p", mount_point], sudo)
                        .await?;
                }

                let (subvol, fs_point) = self.core.subvol_for(path)?;
                // The filesystem, not the subvolume: `btrfs filesystem show` refuses the latter.
                let uuid = self.core.get_fs_uuid(&fs_point).await?;
                let options = spec
                    .options
                    .one("mount_options")
                    .unwrap_or("defaults")
                    .to_string();

                // Moving a declared mount has to move it. btrfs is happy to hold one subvolume
                // at two places, so writing the new entry and mounting it would leave the old
                // mount live and the machine in a state no declaration describes, until someone
                // rebooted. Best-effort: a busy mount point is a reason to warn, not a reason to
                // fail an install that otherwise worked.
                for stale in self.core.current_mounts_of(&subvol) {
                    if stale.trim_end_matches('/') == mount_point.trim_end_matches('/') {
                        continue;
                    }
                    info!(
                        "BTRFS: releasing {} — {} is declared at {}",
                        stale, path, mount_point
                    );
                    if self
                        .core
                        .executor
                        .run("umount", &[&stale], sudo)
                        .await
                        .is_err()
                    {
                        warn!(
                            "BTRFS: {} is still mounted at {}, which no declaration asks for — \
                             something is using it",
                            path, stale
                        );
                    }
                }

                let core_ref = self.core.clone();
                let mount_str = mount_point.to_string();

                tokio::task::spawn_blocking(move || {
                    core_ref.update_fstab(&uuid, &subvol, &mount_str, &options)
                })
                .await
                .map_err(|e| Error::Other(e.to_string()))??;

                // `mount POINT` reads the entry just written, so the running machine and the
                // next boot mount the same thing — the alternative is a mount that works until
                // someone reboots.
                self.core
                    .executor
                    .run("mount", &[mount_point], sudo)
                    .await?;
            }
        }
        Ok(())
    }

    async fn remove(
        &self,
        names: &[String],
        sudo: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        for name in names {
            // The declared mount goes before the subvolume does, in that order: a mounted
            // subvolume cannot be deleted, and an fstab entry outliving its subvolume is a
            // machine that stops in the initramfs at the next boot.
            if let Ok((subvol, _)) = self.core.subvol_for(name) {
                let core_ref = self.core.clone();
                let point = tokio::task::spawn_blocking(move || core_ref.drop_from_fstab(&subvol))
                    .await
                    .map_err(|e| Error::Other(e.to_string()))??;
                if let Some(point) = point {
                    info!("BTRFS: unmounting {} before deleting {}", point, name);
                    let _ = self.core.executor.run("umount", &[&point], sudo).await;
                }
            }
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
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        // One subvolume, one package — keyed by the filesystem it lives on and its path from
        // that filesystem's root, because `@mount` makes the same object reachable by two
        // paths. Declaring `btrfs:/mnt/fs/data @mount=/srv` leaves it listed under `/mnt/fs/data`
        // *and* under `/srv`, and the second name is undeclared: `remove-orphans` would offer
        // to delete the subvolume the user just declared, under its other name.
        let mut best: std::collections::BTreeMap<(String, String), (usize, String, String)> =
            Default::default();
        let mut mounted_at: std::collections::BTreeMap<(String, String), String> =
            Default::default();
        for mount in self.core.btrfs_mounts() {
            let Ok(output) = self
                .core
                .executor
                .run_output("btrfs", &["subvolume", "list", &mount.point], false)
                .await
            else {
                // One filesystem this process cannot read is not "no subvolumes anywhere".
                // The others still answer, and the alternative — a `?` here — is how asking
                // `/` on a machine with an ext4 root made this whole backend report an error.
                continue;
            };
            for (rel, name) in subvolume_paths(&mount.point, &mount.prefix, &output) {
                // A mount whose own `subvol=` IS this subvolume is where it is mounted. That is
                // the second name being collapsed below, read as the fact it actually carries,
                // so `sync` can tell a declared `@mount=` that happened from one that did not.
                if mount.prefix.trim_matches('/') == rel.trim_matches('/') {
                    mounted_at.insert(
                        (mount.device.clone(), rel.clone()),
                        mount.point.trim_end_matches('/').to_string(),
                    );
                }
                let key = (mount.device.clone(), rel);
                // The mount closest to the filesystem root wins — the shortest `subvol=`, not
                // the shortest name. That is the path a declaration was written against:
                // `btrfs:/mnt/fs/data @mount=/srv` mounts the object a second time at `/srv`,
                // and answering `/srv` would make `sync` believe the declared path was absent
                // and re-create it on every run, which is the 2026-07-30 bug from the other
                // side. Ties go to the shortest mount point, then lexicographically, so two
                // runs on an unchanged machine answer the same.
                let rank = (mount.prefix.len(), mount.point.clone(), name);
                best.entry(key)
                    .and_modify(|held| {
                        if rank < *held {
                            *held = rank.clone();
                        }
                    })
                    .or_insert(rank);
            }
        }
        Ok(best
            .into_iter()
            .map(|(key, (_, _, name))| {
                let mut p = Package::new(name, "btrfs");
                if let Some(point) = mounted_at.get(&key) {
                    p.properties.insert("mount".to_string(), point.clone());
                }
                p
            })
            .collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    /// Through the listing, like `zfs:` and `lvm:` — never `Path::exists`.
    ///
    /// Two things turn on it. A plain directory is not a subvolume, and answering "installed"
    /// for one leaves `sync` believing a declaration it never carried out is satisfied; and the
    /// planner reads *this* answer, not `list_installed`, when it asks whether a spec still
    /// needs doing — so a `Package` built here by hand carries no `mount` property and a
    /// declared `@mount=` that never happened stays invisible for ever.
    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let name = name.trim_end_matches('/');
        let Some(mut p) = self
            .list_installed()
            .await?
            .into_iter()
            .find(|p| p.name.trim_end_matches('/') == name)
        else {
            return Ok(None);
        };
        // The quota and the fstab options are read **here and not in `list_installed`**, because
        // each is a question about one named subvolume and the listing walks every subvolume on
        // every btrfs filesystem. Asking there would fan one `sync` out into a subprocess per
        // subvolume — the shape the planner's own dependency walk is capped at one level to
        // avoid. The planner asks `info`, which is the only caller that needs these.
        if let Ok(out) = self
            .core
            .executor
            .run_output(
                "btrfs",
                &["qgroup", "show", "-r", "-f", "--raw", name],
                false,
            )
            .await
        {
            let limit = qgroup_limit_in(&out).map(|b| b.to_string());
            p.properties.insert(
                "quota".into(),
                limit.unwrap_or_else(|| crate::backends::storage::NO_LIMIT.to_string()),
            );
        }
        if let Ok((subvol, _)) = self.core.subvol_for(name) {
            if let Ok(content) = fs::read_to_string(&self.core.fstab_file) {
                if let Some(opts) = fstab_options(&content, &subvol) {
                    p.properties.insert("mount_options".to_string(), opts);
                }
            }
        }
        Ok(Some(p))
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(BtrfsBackendCore::new(exec.clone()));
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
        let stem = format!(
            "shall-btrfs-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let f = std::env::temp_dir().join(format!("{}-mounts", stem));
        std::fs::write(&f, mounts).expect("fixture mount table");
        BtrfsBackendCore {
            executor: exec,
            name: "btrfs".to_string(),
            mounts_file: f,
            fstab_file: std::env::temp_dir().join(format!("{}-fstab", stem)),
        }
    }

    /// The limit is found by its header, not by counting columns: `btrfs-progs` has changed what
    /// it prints across versions, and an index would read `excl` on the version that added one.
    #[test]
    fn the_qgroup_limit_is_read_by_name_and_none_is_not_a_number() {
        let with_limit = "qgroupid         rfer         excl     max_rfer \n\
                          --------         ----         ----     -------- \n\
                          0/257            16384        16384    10737418240\n";
        assert_eq!(qgroup_limit_in(with_limit), Some(10737418240));

        // `none` is btrfs saying there is no limit. It is not a size, and a declared `@quota=`
        // compared against it would be satisfied by a word.
        let unlimited = "qgroupid         rfer         excl     max_rfer \n\
                         --------         ----         ----     -------- \n\
                         0/257            16384        16384    none\n";
        assert_eq!(qgroup_limit_in(unlimited), None);

        // A column order this code did not choose. The header moves, the answer does not.
        let reordered = "qgroupid     max_rfer         rfer\n\
                         --------     --------         ----\n\
                         0/257        5368709120       16384\n";
        assert_eq!(qgroup_limit_in(reordered), Some(5368709120));

        // Quotas disabled: btrfs says so on stderr and prints no table at all.
        assert_eq!(qgroup_limit_in(""), None);
        assert_eq!(
            qgroup_limit_in("ERROR: can't list qgroups: quotas not enabled\n"),
            None
        );
    }

    /// The inverse of what `fstab_with` writes, so a changed `@mount_options` can be seen. The
    /// `subvol=` is dropped because it is the address, not an option the user declared — leaving
    /// it in would make every entry compare unequal to the options that produced it.
    #[test]
    fn the_fstab_options_read_back_as_the_ones_that_were_declared() {
        for declared in ["defaults", "noatime", "noatime,compress=zstd"] {
            let written = fstab_with("", "abc", "/data", "/srv", declared);
            assert_eq!(
                fstab_options(&written, "/data").as_deref(),
                Some(declared),
                "round trip through the entry changed {}",
                declared
            );
        }
        // A subvolume with no entry has no options, which is not the same as `defaults`: the
        // planner must be able to tell "not written" from "written with the default".
        assert_eq!(fstab_options("", "/data"), None);
        assert_eq!(
            fstab_options(
                &fstab_with("", "abc", "/data", "/srv", "defaults"),
                "/other"
            ),
            None
        );
    }

    /// `info` is what the planner asks, so it is what has to carry the geometry. Both reads are
    /// per-subvolume and live here rather than in `list_installed`, which walks every subvolume
    /// on every filesystem — asking there would spawn a subprocess per subvolume on every sync.
    #[tokio::test]
    async fn info_reports_the_quota_and_the_mount_options_a_declaration_can_drift_from() {
        let core = core_with(
            "/dev/sdb1 /mnt/fs btrfs rw,relatime,subvol=/ 0 0\n",
            &[
                (
                    "btrfs subvolume list /mnt/fs",
                    "ID 256 gen 8 top level 5 path data\n",
                ),
                (
                    "btrfs qgroup show -r -f --raw /mnt/fs/data",
                    "qgroupid         rfer         excl     max_rfer \n\
                     --------         ----         ----     -------- \n\
                     0/256            16384        16384    10737418240\n",
                ),
            ],
        );
        std::fs::write(
            &core.fstab_file,
            "UUID=abc /srv btrfs subvol=/data,noatime 0 0\n",
        )
        .expect("fixture fstab");
        let q = BtrfsQueryable {
            core: Arc::new(core),
        };
        let p = q
            .info("/mnt/fs/data")
            .await
            .unwrap()
            .expect("the subvolume is listed");
        assert_eq!(
            p.properties.get("quota").map(String::as_str),
            Some("10737418240")
        );
        assert_eq!(
            p.properties.get("mount_options").map(String::as_str),
            Some("noatime")
        );
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
            vec![BtrfsMount {
                device: "/dev/sdb1".to_string(),
                point: "/mnt/my disk".to_string(),
                prefix: "sub".to_string(),
            }],
            "an escaped space in a mount point is a space, and the subvol= prefix loses its slash"
        );
    }

    fn mounts(table: &str) -> Vec<BtrfsMount> {
        btrfs_mounts_in(table)
    }

    /// Real `btrfs filesystem show` output. The UUID is a token inside the first line, and the
    /// code that read it wanted a line that *started* with `uuid:` — so it never found one, and
    /// nothing noticed because its only caller was unreachable.
    #[test]
    fn the_filesystem_uuid_is_read_out_of_the_real_report() {
        let labelled = "Label: 'data'  uuid: 3b5f9c21-0f38-4a7c-9c2f-1b7d0a5e6c44\n\
                        \tTotal devices 1 FS bytes used 144.00KiB\n\
                        \tdevid    1 size 512.00MiB used 88.00MiB path /dev/loop0\n";
        assert_eq!(
            fs_uuid_in(labelled).as_deref(),
            Some("3b5f9c21-0f38-4a7c-9c2f-1b7d0a5e6c44")
        );
        // An unlabelled filesystem, which is what `mkfs.btrfs` with no `-L` produces.
        assert_eq!(
            fs_uuid_in("Label: none  uuid: 11111111-2222-3333-4444-555555555555\n").as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );
        // A report with no UUID at all is a refusal, not an fstab entry saying `UUID=`.
        assert_eq!(fs_uuid_in("btrfs: command not found\n"), None);
        assert_eq!(fs_uuid_in(""), None);
    }

    /// `@mount` writes an fstab entry, and fstab names a subvolume from the *filesystem* root.
    /// The declared path is on the mounted tree, so the two differ by the mount's own `subvol=`
    /// — the same offset `list` needed, in the other direction.
    fn subvol_of(mounts: &[BtrfsMount], path: &str) -> Option<String> {
        subvol_arg(mounts, path).map(|(subvol, _)| subvol)
    }

    #[test]
    fn an_fstab_entry_names_the_subvolume_from_the_filesystem_root() {
        let plain = mounts("/dev/sdb1 /mnt/fs btrfs rw,subvol=/ 0 0\n");
        assert_eq!(subvol_of(&plain, "/mnt/fs/data").as_deref(), Some("/data"));
        assert_eq!(
            subvol_of(&plain, "/mnt/fs/a/b").as_deref(),
            Some("/a/b"),
            "a nested subvolume keeps every component"
        );

        // The filesystem the subvolume lives on comes back with it, because the UUID an fstab
        // entry needs can only be asked for there: `btrfs filesystem show /mnt/fs/data` answers
        // `not a valid btrfs filesystem` (measured 2026-07-31), and asking it at the declared
        // path is why `@mount=` failed on its first real run.
        assert_eq!(
            subvol_arg(&plain, "/mnt/fs/data")
                .map(|(_, fs)| fs)
                .as_deref(),
            Some("/mnt/fs")
        );

        // The ordinary Linux root: `/` mounted at `subvol=/@`. An entry built from the declared
        // path would say `subvol=/srv`, which is a different object from `/@/srv` — or nothing
        // at all, and a boot that stops on a mount that cannot be found.
        let offset = mounts("/dev/sda2 / btrfs rw,subvol=/@ 0 0\n");
        assert_eq!(subvol_of(&offset, "/srv").as_deref(), Some("/@/srv"));

        // The longest mount wins, not the first: `/` is a prefix of everything.
        let nested = mounts(
            "/dev/sda2 / btrfs rw,subvol=/@ 0 0\n\
             /dev/sdb1 /mnt/fs btrfs rw,subvol=/ 0 0\n",
        );
        assert_eq!(subvol_of(&nested, "/mnt/fs/data").as_deref(), Some("/data"));
    }

    /// The two refusals. Neither is a guess: an fstab entry for a path this backend cannot place
    /// would mount the wrong object, and the filesystem root is the one line a machine boots by.
    #[test]
    fn a_path_that_is_not_under_a_btrfs_mount_has_no_fstab_entry() {
        let m = mounts("/dev/sdb1 /mnt/fs btrfs rw,subvol=/ 0 0\n");
        assert_eq!(subvol_arg(&m, "/home/me/data"), None, "another filesystem");
        assert_eq!(
            subvol_arg(&m, "/mnt/fs"),
            None,
            "the root of a mounted filesystem is not a subvolume this backend made, and writing \
             an entry for it would rewrite the line that mounts the machine"
        );
        assert_eq!(subvol_arg(&m, "/mnt/fsx/data"), None, "a name, not a path");
    }

    /// The line this replaced dropped every fstab line *containing* the mount point, so
    /// declaring `/mnt` deleted `/mnt/data`, `/mnt/home`, and any comment naming the path.
    #[test]
    fn writing_an_entry_leaves_the_neighbours_alone() {
        let fstab = "# /etc/fstab: keep /mnt/data safe\n\
                     UUID=aaa / ext4 defaults 0 1\n\
                     UUID=bbb /mnt/data btrfs subvol=/data,defaults 0 0\n\
                     UUID=ccc /mnt/datastore ext4 defaults 0 2\n";
        let out = fstab_with(fstab, "bbb", "/srv", "/mnt", "defaults");
        assert!(out.contains("# /etc/fstab: keep /mnt/data safe"));
        assert!(out.contains("UUID=ccc /mnt/datastore"));
        assert!(out.contains("UUID=bbb /mnt/data btrfs subvol=/data"));
        assert!(out.contains("UUID=bbb /mnt btrfs subvol=/srv,defaults 0 0"));
    }

    /// One declaration, one entry — whichever half changed.
    #[test]
    fn re_declaring_replaces_rather_than_doubles() {
        let first = fstab_with(
            "UUID=aaa / ext4 defaults 0 1\n",
            "bbb",
            "/srv",
            "/mnt/x",
            "defaults",
        );
        // The same subvolume at a new mount point leaves nothing behind at the old one.
        let moved = fstab_with(&first, "bbb", "/srv", "/mnt/y", "defaults");
        assert_eq!(moved.matches("subvol=/srv").count(), 1);
        assert!(!moved.contains("/mnt/x"));
        // A different subvolume at an occupied mount point replaces the tenant.
        let swapped = fstab_with(&moved, "bbb", "/other", "/mnt/y", "noatime");
        assert_eq!(swapped.matches(" /mnt/y ").count(), 1);
        assert!(!swapped.contains("subvol=/srv"));
        assert!(swapped.contains("subvol=/other,noatime"));
        assert!(swapped.contains("UUID=aaa / ext4"), "the root is untouched");
    }

    /// Removal has to take the entry with it. An fstab line naming a subvolume that no longer
    /// exists is not untidy — it is a boot that stops in the initramfs.
    #[test]
    fn removing_a_subvolume_takes_its_fstab_entry_and_names_the_mount() {
        let fstab = "UUID=aaa / ext4 defaults 0 1\n\
                     UUID=bbb /srv btrfs subvol=/data,defaults 0 0\n";
        let (out, point) = fstab_without(fstab, "/data");
        assert_eq!(
            point.as_deref(),
            Some("/srv"),
            "so the caller can unmount it"
        );
        assert_eq!(out, "UUID=aaa / ext4 defaults 0 1\n");

        // A subvolume that was never mounted has no entry, and that is not an error.
        let (unchanged, none) = fstab_without(fstab, "/nothing");
        assert_eq!(unchanged, fstab);
        assert_eq!(none, None);
    }

    /// `@mount` gives one subvolume a second path, and the second one is undeclared —
    /// `remove-orphans` would offer to destroy the volume the user had just declared.
    ///
    /// The surviving name must be the declared one and not merely the shorter one. A mount at
    /// `/srv` is shorter than `/mnt/fs/data`, and answering `/srv` would leave the declaration
    /// looking unfulfilled: `sync` would create the subvolume again on every run, which is the
    /// bug this backend was fixed for on 2026-07-30, arriving from the other direction.
    #[tokio::test]
    async fn a_subvolume_reachable_twice_is_listed_once_by_its_declared_name() {
        let core = core_with(
            "/dev/sdb1 /mnt/fs btrfs rw,subvol=/ 0 0\n\
             /dev/sdb1 /srv btrfs rw,subvol=/data 0 0\n",
            &[
                (
                    "btrfs subvolume list /mnt/fs",
                    "ID 256 gen 8 top level 5 path data\n",
                ),
                (
                    "btrfs subvolume list /srv",
                    "ID 256 gen 8 top level 5 path data\n",
                ),
            ],
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
            vec!["/mnt/fs/data".to_string()],
            "one object, one package, named the way the declaration named it"
        );
    }

    /// The collapsed second name is not thrown away — it is *where the subvolume is mounted*,
    /// and `sync` needs it to tell a `@mount=` that took effect from one that did not.
    #[tokio::test]
    async fn a_mounted_subvolume_reports_where_it_is_mounted() {
        let core = core_with(
            "/dev/sdb1 /mnt/fs btrfs rw,subvol=/ 0 0\n\
             /dev/sdb1 /srv btrfs rw,subvol=/data 0 0\n",
            &[
                (
                    "btrfs subvolume list /mnt/fs",
                    "ID 256 gen 8 top level 5 path data\n\
                     ID 257 gen 8 top level 5 path other\n",
                ),
                (
                    "btrfs subvolume list /srv",
                    "ID 256 gen 8 top level 5 path data\n",
                ),
            ],
        );
        let q = BtrfsQueryable {
            core: Arc::new(core),
        };
        let pkgs = q.list_installed().await.unwrap();
        let data = pkgs.iter().find(|p| p.name == "/mnt/fs/data").unwrap();
        assert_eq!(
            data.properties.get("mount").map(String::as_str),
            Some("/srv")
        );
        // A subvolume nobody mounted reports no mountpoint, and that is a *fact* — the mount
        // table answered and this subvolume is in none of it. The planner reads it as "not
        // where the line says", which is what makes a `@mount=` that failed converge on the
        // next run instead of being invisible for ever.
        let other = pkgs.iter().find(|p| p.name == "/mnt/fs/other").unwrap();
        assert_eq!(other.properties.get("mount"), None);
    }

    /// `info` is what the planner asks, so it has to answer the same way `list` does. It used
    /// to answer `Path::exists`, which calls any directory on any filesystem a subvolume — and
    /// a `sync` told the declaration is already satisfied never creates the subvolume at all.
    #[tokio::test]
    async fn a_plain_directory_is_not_an_installed_subvolume() {
        let core = core_with(
            "/dev/sdb1 /mnt/fs btrfs rw,subvol=/ 0 0\n\
             /dev/sdb1 /srv btrfs rw,subvol=/data 0 0\n",
            &[
                (
                    "btrfs subvolume list /mnt/fs",
                    "ID 256 gen 8 top level 5 path data\n",
                ),
                (
                    "btrfs subvolume list /srv",
                    "ID 256 gen 8 top level 5 path data\n",
                ),
            ],
        );
        let q = BtrfsQueryable {
            core: Arc::new(core),
        };
        // The temp directory exists on every machine that runs this suite, and it is not a
        // subvolume anywhere.
        let dir = std::env::temp_dir();
        assert!(dir.exists(), "fixture assumption");
        assert!(q.info(dir.to_str().unwrap()).await.unwrap().is_none());

        // And the real one answers WITH its properties, which is the half the planner reads.
        let found = q.info("/mnt/fs/data").await.unwrap().expect("declared");
        assert_eq!(
            found.properties.get("mount").map(String::as_str),
            Some("/srv")
        );
    }

    /// Moving a declared mount has to release the old one. btrfs holds a subvolume at as many
    /// places as you ask, so writing the new fstab entry and mounting it would leave the machine
    /// mounted somewhere no declaration names, until a reboot cleared it.
    #[test]
    fn every_live_mount_of_a_subvolume_is_found() {
        let table = "/dev/sdb1 /mnt/fs btrfs rw,subvol=/ 0 0\n\
                     /dev/sdb1 /srv btrfs rw,subvol=/data 0 0\n\
                     /dev/sdb1 /old btrfs rw,subvol=/data 0 0\n\
                     /dev/sdb1 /other btrfs rw,subvol=/elsewhere 0 0\n";
        let core = core_with(table, &[]);
        let mut points = core.current_mounts_of("/data");
        points.sort();
        assert_eq!(points, vec!["/old".to_string(), "/srv".to_string()]);
        // The leading slash is the fstab spelling and the mount table's is not — the same
        // subvolume either way, and a comparison that missed that would release nothing.
        assert_eq!(
            core.current_mounts_of("data"),
            core.current_mounts_of("/data")
        );
        assert!(core.current_mounts_of("/nowhere").is_empty());
    }

    /// Same path on two filesystems is two subvolumes, and collapsing them would hide one.
    #[tokio::test]
    async fn the_same_name_on_two_filesystems_is_two_subvolumes() {
        let core = core_with(
            "/dev/sdb1 /mnt/a btrfs rw,subvol=/ 0 0\n\
             /dev/sdc1 /mnt/b btrfs rw,subvol=/ 0 0\n",
            &[
                (
                    "btrfs subvolume list /mnt/a",
                    "ID 256 gen 8 top level 5 path data\n",
                ),
                (
                    "btrfs subvolume list /mnt/b",
                    "ID 256 gen 8 top level 5 path data\n",
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
        assert_eq!(names, vec!["/mnt/a/data", "/mnt/b/data"]);
    }
}
