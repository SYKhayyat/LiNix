//! What each manager has installed, asked once per run.
//!
//! Answering "is `jq` installed?" by listing every package the manager has is what nearly every
//! backend in this tree does — `info` is `list_installed()` plus a `find` in eighteen of them —
//! and the callers ask it once per *declared* package. Measured on Windows, `linix check drift`
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

use crate::core::{Package, Result};
use dashmap::DashMap;
use std::future::Future;
use std::sync::Arc;

/// One manager's listing, and whether it has been fetched yet.
type Slot = Arc<tokio::sync::Mutex<Option<Vec<Package>>>>;

#[derive(Default)]
pub struct InstalledListings {
    by_backend: DashMap<String, Slot>,
}

impl InstalledListings {
    pub fn new() -> Self {
        Self::default()
    }

    /// This manager's installed set, fetching it only the first time it is asked for.
    ///
    /// The slot's lock is held across the fetch on purpose: two concurrent askers must produce
    /// one subprocess, not two. Different managers hold different slots, so this never
    /// serialises the fan-out across backends.
    pub async fn once<F>(&self, backend: &str, fetch: F) -> Result<Vec<Package>>
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
        // A failure is not cached: a manager that could not answer this time may answer next
        // time, and remembering "it errored" would turn one transient failure into the run's
        // permanent verdict.
        let fresh = fetch.await?;
        *slot = Some(fresh.clone());
        Ok(fresh)
    }

    /// Forget everything. Called after any mutating command, because that is the only thing
    /// during a run that can change what is installed.
    pub fn forget_all(&self) {
        self.by_backend.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn pkg(name: &str) -> Package {
        Package::new(name, "test")
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
