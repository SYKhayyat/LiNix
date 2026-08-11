//! Finding and removing an artifact's cached copy when a download-backend package is removed
//! (K4). Shall knows the file only where it fetched it itself, so this covers the *download*
//! backends (`github:`/`web:`/`appimage:`) — the ordinary package managers own their own caches
//! and clean them their own way.
//!
//! **The match is exact, by filename, and only regular files are deleted.** A cache clean is a
//! removal, and this repo's flagship bug was a removal that reached further than anyone meant —
//! so the search never deletes a directory, never matches on a prefix, and is bounded in depth.
//! A file named exactly like the artifact that was fetched is the only thing it will touch.

use std::path::{Path, PathBuf};

/// How deep to look inside a cache directory. Deep enough to reach a manager's own
/// `~/.cache/<tool>/<file>` layout, shallow enough that `/var/cache` is not a full-disk walk.
const MAX_DEPTH: usize = 4;

/// The cache locations Shall searches, in addition to any the user pointed it at. Order does not
/// matter — every match is removed — but the list is the standard XDG/system spots plus the two
/// download backends keep their own trees under.
pub fn standard_cache_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.trim().is_empty() {
            dirs.push(PathBuf::from(xdg));
        }
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".cache"));
    }
    #[cfg(unix)]
    dirs.push(PathBuf::from("/var/cache"));
    dirs
}

/// Every regular file at or below `root` (bounded depth) whose file name equals `basename`.
fn matches_in(root: &Path, basename: &str) -> Vec<PathBuf> {
    if basename.is_empty() {
        return Vec::new();
    }
    walkdir::WalkDir::new(root)
        .max_depth(MAX_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.file_name().to_string_lossy() == basename)
        .map(|e| e.into_path())
        .collect()
}

/// Find every cached copy of `basename` across the user's `extra` dirs and the standard ones.
/// Pure discovery — nothing is deleted here, so it is testable without touching real caches.
pub fn find_cached(basename: &str, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = std::collections::BTreeSet::new();
    let mut roots: Vec<PathBuf> = extra.to_vec();
    roots.extend(standard_cache_dirs());
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for hit in matches_in(&root, basename) {
            seen.insert(hit);
        }
    }
    seen.into_iter().collect()
}

/// Delete every cached copy of `basename`. Returns the paths actually removed; a delete that
/// fails is warned about and skipped rather than aborting the removal that triggered this.
pub async fn clean_cached(basename: &str, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for path in find_cached(basename, extra) {
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {
                tracing::info!("cleaned cached copy of {} at {}", basename, path.display());
                removed.push(path);
            }
            Err(e) => tracing::warn!("could not clean cached {}: {}", path.display(), e),
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn an_exact_filename_at_depth_is_found_and_a_near_miss_is_not() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("tool").join("v1");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("fd-v10.2.0.tar.gz"), b"x").unwrap();
        // A near miss (prefix, different suffix) must not match — exact name only.
        std::fs::write(nested.join("fd-v10.2.0.tar.gz.part"), b"x").unwrap();

        let hits = find_cached(
            "fd-v10.2.0.tar.gz",
            std::slice::from_ref(&dir.path().to_path_buf()),
        );
        assert_eq!(hits.len(), 1);
        assert!(hits[0].ends_with("fd-v10.2.0.tar.gz"));
    }

    #[test]
    fn a_directory_named_like_the_artifact_is_never_deleted() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("payload.bin")).unwrap();
        let hits = find_cached(
            "payload.bin",
            std::slice::from_ref(&dir.path().to_path_buf()),
        );
        assert!(
            hits.is_empty(),
            "a directory must never be a cache-clean target"
        );
    }

    #[test]
    fn an_empty_basename_matches_nothing() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x"), b"x").unwrap();
        assert!(find_cached("", std::slice::from_ref(&dir.path().to_path_buf())).is_empty());
    }
}
