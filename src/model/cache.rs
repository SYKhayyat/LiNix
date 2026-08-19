//! Finding and removing an artifact's cached copy when a download-backend package is removed
//! (K4). Shall knows the file only where it fetched it itself, so this covers the *download*
//! backends (`github:`/`web:`/`appimage:`) — the ordinary package managers own their own caches
//! and clean them their own way.
//!
//! **The match is exact, by filename, and only regular files are deleted.** A cache clean is a
//! removal, and this repo's flagship bug was a removal that reached further than anyone meant —
//! so the search never deletes a directory, never matches on a prefix, and is bounded in depth.
//! A file named exactly like the artifact that was fetched is the only thing it will touch.

use std::collections::HashSet;
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

/// Every regular file at or below `root` (bounded depth) whose file name is one of `wanted`.
///
/// **A set, not a name.** The rule each file is judged by is unchanged — exact name, regular
/// files only, bounded depth — but the walk is done once for however many names are being
/// looked for rather than once per name. A package with five cached artifacts used to cost
/// five full crawls of `~/.cache` *and* `/var/cache`; twenty such packages cost a hundred
/// crawls of the same two trees looking for a different name each time. `planner.rs` states
/// the shape one layer up: *"asking per package would be one subprocess each; asking per
/// backend is one, and the answer is a set."*
fn matches_in(root: &Path, wanted: &HashSet<&str>) -> Vec<PathBuf> {
    if wanted.is_empty() {
        return Vec::new();
    }
    walkdir::WalkDir::new(root)
        .max_depth(MAX_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| wanted.contains(e.file_name().to_string_lossy().as_ref()))
        .map(|e| e.into_path())
        .collect()
}

/// Find every cached copy of `basename` across the user's `extra` dirs and the standard ones.
/// Pure discovery — nothing is deleted here, so it is testable without touching real caches.
pub fn find_cached(basename: &str, extra: &[PathBuf]) -> Vec<PathBuf> {
    find_cached_set(std::slice::from_ref(&basename), extra)
}

/// The same, for several names in one pass over each root.
pub fn find_cached_set(basenames: &[&str], extra: &[PathBuf]) -> Vec<PathBuf> {
    let wanted: HashSet<&str> = basenames
        .iter()
        .copied()
        .filter(|b| !b.is_empty())
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut roots: Vec<PathBuf> = extra.to_vec();
    roots.extend(standard_cache_dirs());
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for hit in matches_in(&root, &wanted) {
            seen.insert(hit);
        }
    }
    seen.into_iter().collect()
}

/// Delete every cached copy of every name in `basenames`, from one pass over each root.
///
/// **There is no one-name sibling of this, deliberately.** There used to be, and it was the
/// whole of I9: `teardown` called it once per cached artifact, so a release deploying five
/// files crawled `~/.cache` and `/var/cache` five times over, looking for a different name each
/// time. Taking a set is what makes that impossible to write again.
///
/// Returns the paths actually removed; a delete that fails is warned about and skipped rather
/// than aborting the removal that triggered it.
///
/// **The crawl runs on the blocking pool.** `MAX_DEPTH` bounds the *extent* of the walk and
/// says why — *"shallow enough that `/var/cache` is not a full-disk walk"* — but it is not a
/// bound on the cost: four levels of `~/.cache` and `/var/cache` on a working machine is
/// thousands to tens of thousands of `stat` calls, and this is reached from the removal path,
/// where a parked runtime worker stalls the whole wave rather than one task (II.52). This is
/// the case `core::blocking::off_the_runtime`'s own doc describes.
pub async fn clean_cached_set(basenames: &[&str], extra: &[PathBuf]) -> Vec<PathBuf> {
    let owned: Vec<String> = basenames.iter().map(|b| (*b).to_string()).collect();
    let extra = extra.to_vec();
    let found = crate::core::off_the_runtime(move || {
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        find_cached_set(&refs, &extra)
    })
    .await
    .unwrap_or_default();

    let mut removed = Vec::new();
    for path in found {
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {
                tracing::info!("cleaned cached copy at {}", path.display());
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
