use crate::core::{Error, Result};
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;

pub fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let dir = path.parent().ok_or_else(|| {
        let err = std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Target path has no parent directory",
        );
        Error::Io(err.to_string())
    })?;

    if !dir.exists() {
        fs::create_dir_all(dir).map_err(Error::from)?;
    }

    let mut temp_file = NamedTempFile::new_in(dir).map_err(Error::from)?;
    temp_file
        .write_all(content.as_bytes())
        .map_err(Error::from)?;
    temp_file.flush().map_err(Error::from)?;
    temp_file.as_file().sync_all().map_err(Error::from)?;
    temp_file.persist(path).map_err(Error::from)?;

    Ok(())
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path).map_err(Error::from)?;
    }
    Ok(())
}

pub fn read_lines_filtered(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(path).map_err(Error::from)?;
    Ok(content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect())
}

pub fn force_remove(path: &Path) -> Result<()> {
    if path.exists() {
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(Error::from)?;
        } else {
            fs::remove_file(path).map_err(Error::from)?;
        }
    }
    Ok(())
}

/// Put a downloaded artifact's executable on the user's PATH, refusing to destroy a file
/// LiNix did not deploy.
///
/// `~/.local/bin` is shared with the user and with every other tool that installs there, so
/// deploying by name alone means a package called `fd` silently replaces whatever `fd` the
/// user already had. `ShimManager` has always refused that; the download backends each
/// hand-rolled a symlink that did not, so the same directory had opposite answers depending on
/// which backend got there first.
///
/// A destination counts as LiNix's when it is absent, when it is a symlink pointing inside
/// `owned_root` (the backend's own install directory), or when it is the exact path this
/// backend recorded deploying last time — which is what identifies a copy, since a copy
/// carries no pointer home.
pub async fn deploy_executable(
    src: &Path,
    dest: &Path,
    owned_root: &Path,
    recorded: Option<&str>,
) -> Result<()> {
    if !is_ours(dest, owned_root, recorded).await {
        return Err(Error::Validation(format!(
            "refusing to deploy `{}`: {} already exists and LiNix did not create it. Move or \
             rename that file yourself if you want it managed here.",
            dest.file_name().unwrap_or_default().to_string_lossy(),
            dest.display()
        )));
    }

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(Error::from)?;
    }
    // The old entry must be gone before the new one is made: symlink and copy both fail onto
    // an existing path, and a dangling symlink reports as absent to `try_exists`.
    if tokio::fs::symlink_metadata(dest).await.is_ok() {
        tokio::fs::remove_file(dest).await.map_err(Error::from)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = tokio::fs::metadata(src).await.map_err(Error::from)?;
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(src, perms)
            .await
            .map_err(Error::from)?;
        tokio::fs::symlink(src, dest).await.map_err(Error::from)?;
    }

    #[cfg(windows)]
    {
        // No symlink: it needs a privilege the user may not have, and the copy is what the
        // Windows backends already did.
        tokio::fs::copy(src, dest).await.map_err(Error::from)?;
    }

    Ok(())
}

async fn is_ours(dest: &Path, owned_root: &Path, recorded: Option<&str>) -> bool {
    let Ok(meta) = tokio::fs::symlink_metadata(dest).await else {
        return true; // absent
    };
    if recorded.is_some_and(|r| Path::new(r) == dest) {
        return true;
    }
    if meta.file_type().is_symlink() {
        if let Ok(target) = tokio::fs::read_link(dest).await {
            return target.starts_with(owned_root);
        }
    }
    false
}

/// Delete a file or directory a backend deployed, reporting whether it is actually gone.
///
/// An already-absent path counts as removed: the caller's goal is "not on disk", and
/// `NotFound` means that goal is met. Any other error means the file is still there and
/// still on the user's PATH, which the caller must not record as a clean removal.
pub async fn remove_deployed_path(path: impl AsRef<Path>) -> std::result::Result<(), String> {
    let path = path.as_ref();
    let meta = match tokio::fs::symlink_metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {}", path.display(), e)),
    };
    let outcome = if meta.is_dir() {
        tokio::fs::remove_dir_all(path).await
    } else {
        tokio::fs::remove_file(path).await
    };
    match outcome {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {}", path.display(), e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// An artifact directory with one executable in it, and the bin dir it deploys into.
    async fn fixture() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let owned = dir.path().join("artifacts");
        let bin = dir.path().join("bin");
        tokio::fs::create_dir_all(&owned).await.unwrap();
        tokio::fs::create_dir_all(&bin).await.unwrap();
        let src = owned.join("fd");
        tokio::fs::write(&src, b"#!/bin/sh\ntrue\n").await.unwrap();
        (dir, src, bin)
    }

    #[tokio::test]
    async fn it_deploys_into_an_empty_bin_dir() {
        let (_d, src, bin) = fixture().await;
        let dest = bin.join("fd");
        deploy_executable(&src, &dest, src.parent().unwrap(), None)
            .await
            .unwrap();
        assert!(tokio::fs::symlink_metadata(&dest).await.is_ok());
    }

    #[tokio::test]
    async fn it_refuses_to_replace_a_file_linix_did_not_deploy() {
        // `~/.local/bin` is shared with the user. Deploying by name alone would make a
        // package called `fd` silently destroy whatever `fd` they already had.
        let (_d, src, bin) = fixture().await;
        let dest = bin.join("fd");
        tokio::fs::write(&dest, b"the user's own fd").await.unwrap();

        let err = deploy_executable(&src, &dest, src.parent().unwrap(), None)
            .await
            .unwrap_err();
        assert!(format!("{}", err).contains("did not create it"), "{}", err);
        // And it is still theirs.
        assert_eq!(
            tokio::fs::read(&dest).await.unwrap(),
            b"the user's own fd".to_vec()
        );
    }

    #[tokio::test]
    async fn it_replaces_the_path_this_backend_recorded_last_time() {
        // The upgrade case: same declaration, new version. A copy carries no pointer home,
        // so the recorded path is what identifies it as ours.
        let (_d, src, bin) = fixture().await;
        let dest = bin.join("fd");
        tokio::fs::write(&dest, b"an older deploy").await.unwrap();

        let recorded = dest.to_string_lossy().to_string();
        deploy_executable(&src, &dest, src.parent().unwrap(), Some(&recorded))
            .await
            .unwrap();
        assert_ne!(
            tokio::fs::read(&dest).await.unwrap_or_default(),
            b"an older deploy".to_vec()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn it_replaces_a_symlink_that_points_into_its_own_artifacts() {
        let (_d, src, bin) = fixture().await;
        let old = src.parent().unwrap().join("fd-old");
        tokio::fs::write(&old, b"old").await.unwrap();
        let dest = bin.join("fd");
        tokio::fs::symlink(&old, &dest).await.unwrap();

        deploy_executable(&src, &dest, src.parent().unwrap(), None)
            .await
            .unwrap();
        assert_eq!(tokio::fs::read_link(&dest).await.unwrap(), src);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn it_refuses_a_symlink_that_points_somewhere_else() {
        // Another tool's symlink is not ours to replace, even though it is a symlink.
        let (_d, src, bin) = fixture().await;
        let elsewhere = bin.join("some-other-tool");
        tokio::fs::write(&elsewhere, b"x").await.unwrap();
        let dest = bin.join("fd");
        tokio::fs::symlink(&elsewhere, &dest).await.unwrap();

        assert!(deploy_executable(&src, &dest, src.parent().unwrap(), None)
            .await
            .is_err());
    }
}
