use crate::core::{Error, Result};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, info};

/// A shim is the linix binary itself, deployed under the target's name: on startup linix
/// reads `current_exe()`'s filename and re-dispatches when it is not its own
/// (`attempt_shim_hijack`). The shim's NAME is therefore the entire mechanism.
pub struct ShimManager {
    bin_dir: PathBuf,
}

impl ShimManager {
    /// The directory comes from `Config::bin_dir` and from nowhere else. A constructor that
    /// resolved `~/.local/bin` itself is a second answer to "where do shims go", and it is
    /// the answer a sandbox cannot move.
    pub async fn with_bin_dir(bin_dir: PathBuf) -> Result<Self> {
        if !tokio::fs::try_exists(&bin_dir).await.unwrap_or(false) {
            debug!("Creating shim directory at {:?}", bin_dir);
            fs::create_dir_all(&bin_dir).await.map_err(Error::from)?;
        }

        Ok(Self { bin_dir })
    }

    /// Whether `path` is a shim LiNix deployed, i.e. the linix binary under another name:
    /// the same file as the running binary (the hard-link path) or a byte-identical copy
    /// of it (the cross-filesystem fallback).
    ///
    /// `bin_dir` is `~/.local/bin`, which LiNix shares with the user and with every other
    /// tool that installs there. Without this test, removal deletes by NAME alone, so a
    /// managed package called `jq` makes every sync delete whatever `~/.local/bin/jq` is —
    /// a file LiNix never created and does not own.
    async fn is_deployed_shim(path: &Path) -> bool {
        let Ok(current_exe) = std::env::current_exe() else {
            return false;
        };
        let (Ok(shim_meta), Ok(exe_meta)) = (
            tokio::fs::symlink_metadata(path).await,
            tokio::fs::metadata(&current_exe).await,
        ) else {
            return false;
        };
        // A shim is a hard link or a copy — never a symlink (create_shim documents why).
        if shim_meta.file_type().is_symlink() || shim_meta.len() != exe_meta.len() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if shim_meta.dev() == exe_meta.dev() && shim_meta.ino() == exe_meta.ino() {
                return true;
            }
        }
        // Same size but not the same inode: either the copy fallback, or an unrelated file
        // that happens to match. Only the bytes can tell them apart.
        match (
            tokio::fs::read(path).await,
            tokio::fs::read(&current_exe).await,
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    pub async fn create_shim(&self, binary_name: &str) -> Result<()> {
        #[allow(unused_mut)] // mutated only under cfg(windows)
        let mut target_path = self.bin_dir.join(binary_name);

        // A "linix" shim would overwrite linix itself with itself — and on the copy path,
        // truncate the running binary.
        if binary_name == "linix" {
            return Ok(());
        }

        #[cfg(windows)]
        {
            if target_path.extension().is_none_or(|ext| ext != "exe") {
                target_path.set_extension("exe");
            }
        }

        let current_exe = tokio::task::spawn_blocking(std::env::current_exe)
            .await
            .map_err(|e| Error::Other(e.to_string()))?
            .map_err(|e| Error::Io(format!("Failed to locate linix binary: {}", e)))?;

        // Remove first: hard_link/copy onto an existing path fails, and a dangling symlink
        // reports as non-existent to `try_exists`, hence the explicit `is_symlink` check.
        if tokio::fs::try_exists(&target_path).await.unwrap_or(false) || target_path.is_symlink() {
            // S4: only overwrite a file LiNix itself deployed. `bin_dir` is `~/.local/bin`,
            // shared with the user and every other tool; a same-named binary they put there is
            // an unmanaged file, and deploying a shim must not silently destroy it — the same
            // ownership rule `remove_shim` already follows. Redeploying LiNix's own shim is
            // fine (it hashes identical to the linix binary).
            if !Self::is_deployed_shim(&target_path).await {
                return Err(Error::Validation(format!(
                    "refusing to deploy the `{}` shim: {:?} already exists and LiNix did not \
                     create it. Move or rename that file yourself if you want the shim there.",
                    binary_name, target_path
                )));
            }
            fs::remove_file(&target_path).await.map_err(Error::from)?;
        }

        info!(
            "Deploying shim for '{}' -> {:?}",
            binary_name, target_path
        );

        #[cfg(unix)]
        {
            // Hard link, never a symlink: `current_exe()` resolves symlinks, so a symlinked
            // shim would report the name "linix" and dispatch to itself instead of the
            // shimmed tool. Copy is the fallback since a link cannot cross filesystems.
            if let Err(e) = fs::hard_link(&current_exe, &target_path).await {
                debug!(
                    "Hard link failed ({}), falling back to copy...",
                    e
                );
                fs::copy(&current_exe, &target_path)
                    .await
                    .map_err(Error::from)?;
            }
        }

        #[cfg(windows)]
        {
            fs::copy(&current_exe, &target_path)
                .await
                .map_err(Error::from)?;
        }

        Ok(())
    }

    pub async fn reconcile_shims(&self, package_name: &str, should_exist: bool) -> Result<()> {
        if should_exist {
            self.create_shim(package_name).await
        } else {
            self.remove_shim(package_name).await
        }
    }

    pub async fn remove_shim(&self, binary_name: &str) -> Result<()> {
        #[allow(unused_mut)] // mutated only under cfg(windows)
        let mut target_path = self.bin_dir.join(binary_name);

        #[cfg(windows)]
        {
            if target_path.extension().is_none() {
                target_path.set_extension("exe");
            }
        }

        let present =
            tokio::fs::try_exists(&target_path).await.unwrap_or(false) || target_path.is_symlink();
        if !present {
            return Ok(());
        }
        if !Self::is_deployed_shim(&target_path).await {
            debug!(
                "{:?} is not a LiNix shim — leaving it alone.",
                target_path
            );
            return Ok(());
        }
        debug!("Removing shim {:?}", target_path);
        fs::remove_file(&target_path).await.map_err(Error::from)?;
        info!("Successfully removed shim '{}'", binary_name);
        Ok(())
    }

    /// Returns a list of all shims currently managed in the local bin directory.
    pub async fn list_shims(&self) -> Result<Vec<String>> {
        let mut shims = Vec::new();
        if !tokio::fs::try_exists(&self.bin_dir).await.unwrap_or(false) {
            return Ok(shims);
        }

        let mut entries = fs::read_dir(&self.bin_dir).await.map_err(Error::from)?;

        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            let metadata = entry.metadata().await.map_err(Error::from)?;

            if metadata.is_file() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if name != "linix"
                        && name != "linix.exe"
                        && Self::is_deployed_shim(&path).await
                    {
                        #[cfg(windows)]
                        {
                            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                shims.push(stem.to_string());
                            }
                        }
                        #[cfg(unix)]
                        {
                            shims.push(name.to_string());
                        }
                    }
                }
            }
        }
        Ok(shims)
    }

    pub fn get_bin_dir(&self) -> &Path {
        &self.bin_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// `bin_dir` is `~/.local/bin`, shared with the user and every other tool. Removal
    /// used to match on FILENAME alone, so a managed package named `jq` made every sync
    /// delete whatever `~/.local/bin/jq` happened to be. `reconcile_all_shims` calls
    /// `remove_shim` for every managed package that is not shimmed, so this ran on each
    /// successful sync, with no guard and no confirmation.
    #[tokio::test]
    async fn remove_shim_never_deletes_a_file_linix_did_not_deploy() {
        let tmp = tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let mgr = ShimManager::with_bin_dir(bin.clone()).await.unwrap();

        let victim = bin.join("jq");
        tokio::fs::write(&victim, b"#!/bin/sh\necho the user's own jq\n")
            .await
            .unwrap();

        mgr.remove_shim("jq").await.unwrap();

        assert!(
            victim.exists(),
            "sync deleted a file LiNix never created: {:?}",
            victim
        );
    }

    /// S4: the create path had the same blind spot the remove path used to — it deleted
    /// whatever sat at `~/.local/bin/<name>` before deploying. A managed package named `jq`
    /// would then clobber the user's own `jq` on the next sync. Deploy must refuse, not
    /// destroy, an unmanaged file.
    #[tokio::test]
    async fn create_shim_refuses_to_clobber_a_file_linix_did_not_deploy() {
        let tmp = tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let mgr = ShimManager::with_bin_dir(bin.clone()).await.unwrap();

        // Windows shims carry `.exe`, so the file in the way is `jq.exe` there. Naming the
        // victim `jq` on Windows tests a path `create_shim` never touches.
        let victim = bin.join(if cfg!(windows) { "jq.exe" } else { "jq" });
        let contents = b"#!/bin/sh\necho the user's own jq\n";
        tokio::fs::write(&victim, contents).await.unwrap();

        let result = mgr.create_shim("jq").await;

        assert!(result.is_err(), "create_shim must refuse to overwrite a user's file");
        // And it must not have touched the file on its way to refusing.
        let after = tokio::fs::read(&victim).await.unwrap();
        assert_eq!(after, contents, "the user's file was modified despite the refusal");
    }

    #[tokio::test]
    async fn remove_shim_deletes_a_real_deployed_shim() {
        let tmp = tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let mgr = ShimManager::with_bin_dir(bin.clone()).await.unwrap();

        // A shim is the linix binary under another name. The test binary stands in for it:
        // `is_deployed_shim` compares against `current_exe`, which here is the test runner.
        // Windows shims carry `.exe`, which is the name `remove_shim` will look for.
        let exe = std::env::current_exe().unwrap();
        let shim = bin.join(if cfg!(windows) { "ripgrep.exe" } else { "ripgrep" });
        tokio::fs::copy(&exe, &shim).await.unwrap();

        mgr.remove_shim("ripgrep").await.unwrap();

        assert!(!shim.exists(), "a real shim must still be removable");
    }

    #[tokio::test]
    async fn list_shims_reports_only_deployed_shims() {
        let tmp = tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let mgr = ShimManager::with_bin_dir(bin.clone()).await.unwrap();

        let exe = std::env::current_exe().unwrap();
        let deployed = bin.join(if cfg!(windows) { "ripgrep.exe" } else { "ripgrep" });
        tokio::fs::copy(&exe, &deployed).await.unwrap();
        tokio::fs::write(bin.join("my-script"), b"#!/bin/sh\n")
            .await
            .unwrap();

        let shims = mgr.list_shims().await.unwrap();

        assert!(shims.iter().any(|s| s.starts_with("ripgrep")));
        assert!(
            !shims.iter().any(|s| s.starts_with("my-script")),
            "a file LiNix never deployed is not a shim: {:?}",
            shims
        );
    }
}
