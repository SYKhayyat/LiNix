//! Removing one downloaded artifact, for the three backends that download.
//!
//! `github:`, `web:` and `appimage:` keep different state records and the same removal. Each
//! wrote the same four steps out by hand — hand the system manager back what it owns, delete
//! the deployed paths, drop the cached download, and put the record back if anything failed —
//! and each phrased the failure differently. `appimage.rs`'s own test header says it: *"its
//! removal is `web.rs`'s removal with the D5 handoff taken out"*.
//!
//! The record shapes stay where they are. What moves here is the removal, because that is the
//! part where being wrong means a file still on `PATH` and a state file that says otherwise.

use std::path::PathBuf;

use crate::backends::artifact::system_pkg;
use crate::core::executor::CommandExecutor;

/// Everything one recorded artifact put on the machine.
///
/// Empty vectors are the normal case, not a degenerate one: a download-only AppImage has no
/// `bin_path`, and a plain `web:` file has no owning system manager.
#[derive(Default)]
pub struct Deployed {
    /// `(installer, package name)` pairs this artifact was handed to a system manager under.
    ///
    /// **D5: an artifact `apt` owns is removed *through* `apt`, by the name it was recorded
    /// under.** Deleting the file would leave the package in the manager's database — drift no
    /// `sync` can see, because the manager keeps answering that it is installed.
    pub owned_by: Vec<(String, String)>,
    /// Files and links to delete: the artifact itself, and whatever was put on `PATH`.
    pub paths: Vec<String>,
    /// Basenames of the downloads to drop from the cache, when the backend cleans on remove.
    pub cached: Vec<String>,
}

impl Deployed {
    /// A path, skipping the empty string — which is how all three records spell *there is no
    /// such path* rather than using an `Option`.
    pub fn path(mut self, p: &str) -> Self {
        if !p.is_empty() {
            self.paths.push(p.to_string());
        }
        self
    }

    /// The same, for a record that does use an `Option`.
    pub fn maybe_path(self, p: Option<&String>) -> Self {
        match p {
            Some(p) => self.path(p),
            None => self,
        }
    }

    /// The system manager that owns this artifact, if one does.
    pub fn owned(mut self, installer: Option<&str>, system_package: Option<&str>) -> Self {
        if let (Some(i), Some(p)) = (installer, system_package) {
            self.owned_by.push((i.to_string(), p.to_string()));
        }
        self
    }

    /// The download to forget, named as it sits in the cache.
    pub fn cached(mut self, basename: &str) -> Self {
        if !basename.is_empty() {
            self.cached.push(basename.to_string());
        }
        self
    }

    /// The last path segment of a URL, which is how `web:` and `appimage:` name their downloads.
    ///
    /// Through the same `url_filename` the install used, or the name looked for here is not the
    /// name that was written. A URL this refuses is one no install accepted, so there is
    /// nothing cached under it to forget.
    pub fn cached_url(self, url: &str) -> Self {
        let basename = crate::utils::file::url_filename(url).unwrap_or_default();
        self.cached(&basename)
    }
}

/// Undo one artifact. The returned vector is empty when everything came off.
///
/// **Nothing is dropped from state here.** The caller holds the record and puts it back when
/// this reports anything, because a file still on disk with no state row is drift no `sync`
/// can see — the failure mode all three backends were already careful about, in three places.
pub async fn tear_down(
    deployed: &Deployed,
    executor: &CommandExecutor,
    clean_cache: bool,
    cache_dirs: &[PathBuf],
) -> Vec<String> {
    let mut errors = Vec::new();

    for (installer, system_package) in &deployed.owned_by {
        match system_pkg::remove_argv(installer, system_package) {
            Ok(argv) => {
                let (prog, args) = argv.split_first().expect("a remove argv is never empty");
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                if let Err(e) = executor.run(prog, &refs, true).await {
                    errors.push(format!("{installer} {system_package}: {e}"));
                }
            }
            Err(e) => errors.push(e.to_string()),
        }
    }

    for p in &deployed.paths {
        if let Err(e) = crate::utils::remove_deployed_path(p).await {
            errors.push(e);
        }
    }

    // Only once the machine is actually clean. A cache dropped beside a file that would not
    // delete turns the next install into a fresh download for no gain.
    if errors.is_empty() && clean_cache {
        // One pass over each cache root for all of this package's artifacts, not one per
        // artifact. A release deploying five files used to crawl `~/.cache` and `/var/cache`
        // five times over, looking for a different name each time.
        let wanted: Vec<&str> = deployed.cached.iter().map(String::as_str).collect();
        crate::model::cache::clean_cached_set(&wanted, cache_dirs).await;
    }

    errors
}

/// The sentence all three ended with, which is the one thing a caller must not paraphrase: what
/// is named here is **still there**, and the run continues believing so.
///
/// Both halves, because the three said different ones. `github:` and `appimage:` said *still
/// installed*; `web:` said *still on disk*, which understates a `.deb` that is in dpkg's
/// database as well as on the filesystem. Neither is wrong and neither is complete, so the
/// shared sentence says both and no caller loses what it used to report.
pub fn still_installed(noun: &str, failures: &[String]) -> crate::core::Error {
    crate::core::Error::Other(format!(
        "could not remove {} {}(s), still installed and still on disk: {}",
        failures.len(),
        noun,
        failures.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec() -> CommandExecutor {
        CommandExecutor::new(true, false)
    }

    #[tokio::test]
    async fn a_record_with_nothing_deployed_removes_cleanly() {
        let errors = tear_down(&Deployed::default(), &exec(), true, &[]).await;
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// The empty string is how all three records spell *no such path*, and deleting it would
    /// be a delete of the current directory.
    #[test]
    fn an_empty_path_is_not_a_path() {
        let d = Deployed::default()
            .path("")
            .path("/tmp/real")
            .maybe_path(None);
        assert_eq!(d.paths, vec!["/tmp/real".to_string()]);
    }

    #[test]
    fn an_artifact_nobody_owns_hands_nothing_back() {
        let d = Deployed::default()
            .owned(None, Some("ripgrep"))
            .owned(Some("apt"), None)
            .owned(Some("apt"), Some("ripgrep"));
        assert_eq!(d.owned_by, vec![("apt".to_string(), "ripgrep".to_string())]);
    }

    #[test]
    fn a_download_is_named_in_the_cache_by_its_last_url_segment() {
        let d = Deployed::default()
            .cached_url("https://example.com/dl/tool-1.2.3.tar.gz")
            .cached_url("");
        assert_eq!(d.cached, vec!["tool-1.2.3.tar.gz".to_string()]);
    }

    /// A removal that failed must not read as a removal that succeeded, and the noun is the
    /// only part that differs between the three.
    #[test]
    fn the_failure_sentence_says_still_installed() {
        let e = still_installed("web resource", &["a: denied".into(), "b: busy".into()]);
        let text = e.to_string();
        assert!(
            text.contains("could not remove 2 web resource(s)"),
            "{text}"
        );
        // Both halves: three backends said one or the other, and neither may be dropped.
        assert!(text.contains("still installed"), "{text}");
        assert!(text.contains("still on disk"), "{text}");
        assert!(text.contains("a: denied, b: busy"), "{text}");
    }
}
