//! What each manager has installed, asked once per run.
//!
//! Answering "is `jq` installed?" by listing every package the manager has is what nearly every
//! backend in this tree does — `info` is `list_installed()` plus a `find` in eighteen of them —
//! and the callers ask it once per *declared* package. Measured on Windows, `shall check drift`
//! cost ~247 ms more for every additional declaration, on a command whose whole job is to
//! compare two lists it could have fetched once. On Ubuntu the same shape produced exactly
//! `declared + 1` `dpkg-query` invocations.
//!
//! The listing is the same answer every time within one command, so it is fetched once and
//! reused. A mutation is the one thing that can change it, and every mutation goes through
//! `CommandExecutor::run`, which forgets these.
//!
//! **Per executor, not per process.** Every backend of one `App` shares that `App`'s executor,
//! so the memo is scoped to the run — which is also what keeps one test's mock listing out of
//! the next test's, in a suite where a hundred `App`s live in one process.
//!
//! **And optionally per machine, across runs** (`installed_cache_secs`, off by default). Asking
//! once per run is the whole win while a run lasts; the next `shall list` still pays ~3.2 s to
//! ask 24 managers the same question about a machine nothing has touched. The disk layer sits
//! behind this same seam, so a backend cannot tell the difference and no caller had to change.

use crate::core::{Package, Result};
use dashmap::DashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// One manager's listing, and whether it has been fetched yet.
/// The listing itself is behind an `Arc` as well as the slot: **`once` is asked per
/// package, not per backend**, and every hit used to clone the whole `Vec<Package>` out.
/// Measured on a 256-line winget config against a 280-package listing, that is ~71,680
/// `Package` clones to answer 256 questions.
type Listing = Arc<Vec<Package>>;
type Slot = Arc<tokio::sync::Mutex<Option<Listing>>>;

/// A listing as it sits on disk, with the moment it was taken.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedListing {
    /// Seconds since the epoch. Stored rather than read off the file's mtime, because a
    /// backup, a sync tool or a `cp -p` rewrites mtimes and none of them refreshed the answer.
    taken_at: u64,
    packages: Vec<Package>,
}

#[derive(Default)]
pub struct InstalledListings {
    by_backend: DashMap<String, Slot>,
    /// How long a listing on disk stays usable. `None` — the default — is no disk layer.
    ttl: Option<Duration>,
}

impl InstalledListings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a run of `subcommand` may be answered from disk at all.
    ///
    /// **A cached listing may inform a report; it may never source a decision that outlives the
    /// run.** The listing is up to `installed_cache_secs` old, and the whole bargain is that a
    /// stale answer costs you a stale *reading* — which the next run corrects. That bargain
    /// stops holding the moment the answer is written down:
    ///
    /// - a plan built from a listing taken before the user removed something by hand skips the
    ///   install and reports success, leaving a declared package absent and nothing saying so;
    /// - `adopt` writes a declaration for a package that is no longer there, and the next
    ///   `sync` installs it back;
    /// - `plan --out` freezes that same mistake into a file `apply` runs later.
    ///
    /// An allowlist rather than a list of the unsafe ones, because the next command added to
    /// Shall should have to say it is a reader — not discover it was assumed to be one.
    pub fn cache_may_answer(subcommand: &str) -> bool {
        matches!(
            subcommand,
            "list" | "search" | "check" | "outdated" | "info" | "why"
        )
    }

    /// Reuse listings across runs for `secs`. Zero keeps the memo run-scoped, as before.
    pub fn with_ttl(secs: u64) -> Self {
        Self {
            by_backend: DashMap::new(),
            ttl: (secs > 0).then(|| Duration::from_secs(secs)),
        }
    }

    fn cache_dir() -> PathBuf {
        crate::utils::safe_data_dir()
            .join("cache")
            .join("installed")
    }

    /// One file per manager, so a mutation through one never invalidates the others and a
    /// half-written file can only cost the manager it belongs to.
    ///
    /// The name is sanitised because a backend name reaches this from the registry, and a
    /// `..` there would write outside the cache directory.
    fn cache_file(backend: &str) -> Option<PathBuf> {
        let safe: String = backend
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        (!safe.is_empty()).then(|| Self::cache_dir().join(format!("{}.json", safe)))
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// The listing on disk, if there is one and it is younger than the TTL.
    ///
    /// Every failure here — unreadable, unparseable, a clock that moved backwards — answers
    /// `None` and asks the manager. A cache that cannot be read is a cache miss; it is never a
    /// reason to fail a command, and never a reason to report a machine as empty.
    fn read_from_disk(&self, backend: &str) -> Option<Vec<Package>> {
        let ttl = self.ttl?;
        let path = Self::cache_file(backend)?;
        let raw = std::fs::read_to_string(path).ok()?;
        let cached: CachedListing = serde_json::from_str(&raw).ok()?;
        let age = Self::now_secs().checked_sub(cached.taken_at)?;
        (age <= ttl.as_secs()).then_some(cached.packages)
    }

    fn to_disk(&self, backend: &str, packages: &[Package]) {
        if self.ttl.is_none() {
            return;
        }
        let Some(path) = Self::cache_file(backend) else {
            return;
        };
        if std::fs::create_dir_all(Self::cache_dir()).is_err() {
            return;
        }
        let entry = CachedListing {
            taken_at: Self::now_secs(),
            packages: packages.to_vec(),
        };
        if let Ok(json) = serde_json::to_string(&entry) {
            // Written through a temp file and renamed: a listing half-flushed when the process
            // is killed would otherwise be read back as a shorter machine, and a shorter
            // machine is a list of things to remove.
            //
            // The temp name carries the pid, because the rename is only atomic per writer:
            // two `shall` runs sharing one temp path write into each other's file and rename
            // the interleaving, which is the torn listing this exists to prevent, arrived at
            // by the mechanism meant to prevent it. A prompt hook and a terminal are two runs.
            let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
            if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    /// Drop every listing this machine has on disk.
    ///
    /// Public because a mutation is not the only thing that invalidates one: `shall clean-cache`
    /// exists for the case where something outside Shall changed the machine and the user knows
    /// it before the TTL does.
    pub fn forget_on_disk() -> std::io::Result<usize> {
        let dir = Self::cache_dir();
        if !dir.exists() {
            return Ok(0);
        }
        let mut dropped = 0;
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            // `.tmp` too: a run killed between the write and the rename leaves one, and the
            // command whose job is "empty the cache" is the only thing that ever looks here.
            let ours = path.extension().is_some_and(|e| e == "json" || e == "tmp");
            if ours && std::fs::remove_file(&path).is_ok() {
                dropped += 1;
            }
        }
        Ok(dropped)
    }

    /// This manager's installed set, fetching it only the first time it is asked for.
    ///
    /// The slot's lock is held across the fetch on purpose: two concurrent askers must produce
    /// one subprocess, not two. Different managers hold different slots, so this never
    /// serialises the fan-out across backends.
    /// Returns a shared handle. A caller that needs to own the packages clones it — which is
    /// what [`Queryable::list_installed`] does — and a caller that only wants to look one name up
    /// does not have to.
    pub async fn once<F>(&self, backend: &str, fetch: F) -> Result<Listing>
    where
        F: Future<Output = Result<Vec<Package>>>,
    {
        let slot = self
            .by_backend
            .entry(backend.to_string())
            .or_default()
            .clone();
        let mut slot = slot.lock().await;
        if let Some(cached) = slot.as_ref() {
            return Ok(cached.clone());
        }
        // The disk layer is consulted inside the slot lock, so two concurrent askers still
        // produce one read rather than two — the same reason the fetch is in here.
        if let Some(on_disk) = self.read_from_disk(backend) {
            let handle: Listing = Arc::new(on_disk);
            *slot = Some(handle.clone());
            return Ok(handle);
        }
        // A failure is not cached: a manager that could not answer this time may answer next
        // time, and remembering "it errored" would turn one transient failure into the run's
        // permanent verdict.
        let fresh = fetch.await?;
        self.to_disk(backend, &fresh);
        let handle: Listing = Arc::new(fresh);
        *slot = Some(handle.clone());
        Ok(handle)
    }

    /// Forget everything. Called after any mutating command, because that is the only thing
    /// during a run that can change what is installed.
    ///
    /// **The disk layer goes with it.** A memo cleared while a stale file survived would be
    /// re-read from that file on the very next question — so the invalidation that covers one
    /// and not the other is the invalidation that covers neither, which is the shape this repo
    /// has now found in the guard, in the run-scoped memos, and here.
    pub fn forget_all(&self) {
        self.by_backend.clear();
        if self.ttl.is_some() {
            let _ = Self::forget_on_disk();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn pkg(name: &str) -> Package {
        Package::new(name, "test")
    }

    /// A cached listing may inform a report; it may never source a decision that outlives the
    /// run. Asserted by name because the cost of getting it wrong is silent: a plan built on a
    /// listing taken before the user removed something skips the install and reports success.
    ///
    /// The allowlist is checked in both directions — every reader can be served, and every
    /// command that writes its answer down cannot — so a verb added to one list and not the
    /// other fails here rather than in somebody's manifest.
    #[test]
    fn only_a_command_that_just_reports_may_be_answered_from_disk() {
        for reader in ["list", "search", "check", "outdated", "info", "why"] {
            assert!(
                InstalledListings::cache_may_answer(reader),
                "`{reader}` only reports, and pays for a listing it could have had"
            );
        }
        for writer in [
            "sync",
            "install",
            "uninstall",
            "upgrade",
            "adopt",
            "plan",
            "heal",
            "rollback",
            "purge-undeclared",
        ] {
            assert!(
                !InstalledListings::cache_may_answer(writer),
                "`{writer}` writes down what it was told, and was told by a cache"
            );
        }
    }

    #[tokio::test]
    async fn a_manager_is_asked_once_however_often_it_is_listed() {
        let memo = InstalledListings::new();
        let calls = AtomicUsize::new(0);

        for _ in 0..25 {
            let got = memo
                .once("apt", async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(vec![pkg("jq")])
                })
                .await
                .unwrap();
            assert_eq!(got.len(), 1);
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "twenty-five listings reached the manager — this is the `declared + 1` shape the \
             memo exists to remove"
        );
    }

    #[tokio::test]
    async fn two_managers_do_not_share_one_answer() {
        let memo = InstalledListings::new();
        let apt = memo
            .once("apt", async { Ok(vec![pkg("jq")]) })
            .await
            .unwrap();
        let npm = memo
            .once("npm", async { Ok(vec![pkg("prettier"), pkg("eslint")]) })
            .await
            .unwrap();
        assert_eq!(apt.len(), 1);
        assert_eq!(npm.len(), 2);
    }

    #[tokio::test]
    async fn a_failure_is_not_remembered_as_an_answer() {
        let memo = InstalledListings::new();
        let first = memo
            .once("apt", async {
                Err(crate::core::Error::Other("the index was locked".into()))
            })
            .await;
        assert!(first.is_err());
        let second = memo
            .once("apt", async { Ok(vec![pkg("jq")]) })
            .await
            .unwrap();
        assert_eq!(
            second.len(),
            1,
            "one transient failure became the run's permanent answer"
        );
    }

    /// The disk layer's tests share one `SHALL_DATA_DIR`, so they run as one test rather than
    /// racing each other over the same directory.
    #[tokio::test]
    async fn a_listing_survives_a_run_only_when_asked_for_and_only_until_it_is_stale() {
        let dir = std::env::temp_dir().join(format!("shall-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("SHALL_DATA_DIR", &dir);

        // Off by default: a second "run" asks the manager again.
        let calls = AtomicUsize::new(0);
        for _ in 0..2 {
            let run = InstalledListings::new();
            run.once("apt", async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![pkg("jq")])
            })
            .await
            .unwrap();
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a listing crossed runs with the cache off — the default must never persist"
        );

        // On: the second run answers from disk without reaching the manager.
        let calls = AtomicUsize::new(0);
        let mut seen: std::sync::Arc<Vec<Package>> = Default::default();
        for _ in 0..3 {
            let run = InstalledListings::with_ttl(600);
            seen = run
                .once("apt", async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(vec![pkg("jq"), pkg("ripgrep")])
                })
                .await
                .unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "three runs, one listing");
        assert_eq!(seen.len(), 2, "the cached answer must be the whole answer");
        assert_eq!(seen[0].name, "jq");

        // A TTL of one second, with the entry written far enough in the past to be stale:
        // expiry is what bounds how wrong a cached machine can be.
        let stale = InstalledListings::with_ttl(1);
        let path = InstalledListings::cache_file("apt").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let mut entry: CachedListing = serde_json::from_str(&raw).unwrap();
        entry.taken_at -= 60;
        std::fs::write(&path, serde_json::to_string(&entry).unwrap()).unwrap();
        let refetched = AtomicUsize::new(0);
        stale
            .once("apt", async {
                refetched.fetch_add(1, Ordering::SeqCst);
                Ok(vec![pkg("jq")])
            })
            .await
            .unwrap();
        assert_eq!(
            refetched.load(Ordering::SeqCst),
            1,
            "an expired listing was served — the TTL bounds nothing"
        );

        // A mutation drops the disk layer too, not just the memo. A `forget_all` that cleared
        // one and left the other would re-read the pre-mutation answer immediately.
        let mutating = InstalledListings::with_ttl(600);
        mutating
            .once("apt", async { Ok(vec![pkg("jq")]) })
            .await
            .unwrap();
        assert!(InstalledListings::cache_file("apt").unwrap().exists());
        mutating.forget_all();
        assert!(
            !InstalledListings::cache_file("apt").unwrap().exists(),
            "the memo was forgotten and the file that would answer the next question was not"
        );

        // A backend name can never escape the cache directory.
        let escaped = InstalledListings::cache_file("../../evil").unwrap();
        assert_eq!(
            escaped.parent(),
            Some(InstalledListings::cache_dir()).as_deref()
        );

        // Unreadable is a miss, never a failure and never an empty machine.
        let corrupt = InstalledListings::with_ttl(600);
        std::fs::create_dir_all(InstalledListings::cache_dir()).unwrap();
        std::fs::write(InstalledListings::cache_file("npm").unwrap(), "{ not json").unwrap();
        let got = corrupt
            .once("npm", async { Ok(vec![pkg("prettier")]) })
            .await
            .unwrap();
        assert_eq!(got.len(), 1, "a corrupt cache file must read as a miss");

        std::env::remove_var("SHALL_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_mutation_makes_the_next_listing_real() {
        let memo = InstalledListings::new();
        let before = memo.once("apt", async { Ok(vec![]) }).await.unwrap();
        assert!(before.is_empty());
        memo.forget_all();
        let after = memo
            .once("apt", async { Ok(vec![pkg("jq")]) })
            .await
            .unwrap();
        assert_eq!(
            after.len(),
            1,
            "the listing taken before the install was still being served after it"
        );
    }
}
