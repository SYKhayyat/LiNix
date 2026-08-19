//! A reader sees one moment, and never waits to get it.
//!
//! Every state file Shall owns is written whole, by atomic rename, so no reader can see half of
//! one. The exposure is *between* them: `registry.json`, `journal.jsonl` and the `locks/`
//! ledgers are separate reads, and a writer in another process — very often `shall
//! hook-reconcile`, spawned by an `apt install` nobody told Shall about — updates them one after
//! another while a reader is part-way down the list. What comes back is then a combination of
//! facts that never held at the same time: a package the registry has and the journal does not,
//! a pin the ledger records against a version the registry no longer carries.
//!
//! **The fix is not for readers to take the lock.** A `sync` holds it for as long as the package
//! managers take, which is minutes, and a `list` that queued behind it would be a program that
//! stops answering questions exactly when there is most to ask about. That trade was made once
//! already, with `watch`, and it ended with the user who followed the documented deployment
//! unable to run any other Shall on the machine.
//!
//! So a reader detects instead of excluding. It notes what the writers were doing, reads, and
//! notes again; if nothing moved, the read spanned one moment and nothing else was needed. This
//! costs two reads of two tiny files on each side and no waiting of any kind, and in the
//! overwhelmingly common case — no writer running at all — it is exactly that and nothing more.
//!
//! What it cannot do is make a *torn* read impossible for ever, and it does not pretend to: a
//! machine where a writer commits during every one of several attempts is a machine where the
//! answer is stale by the time it is printed no matter what. After [`ATTEMPTS`] tries the last
//! answer is returned rather than an error, because an advisory listing that refuses to print is
//! worse than one that is a moment behind.

use crate::core::datalock;
use crate::core::Result;

/// How many times a reader re-reads before it accepts what it has.
///
/// Three, because each retry only helps if the writer that interrupted has since finished, and a
/// writer that is still going will still be going on the fourth. Past this the honest answer is
/// the one in hand.
pub const ATTEMPTS: usize = 3;

/// Read several files as one moment.
///
/// `read` may be called more than once, so it must be a read: no writing, and no side effect
/// that a second call would double.
pub async fn stable<T, F, Fut>(mut read: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    // A writer is reading its own state under its own lock, so there is nothing to detect and
    // nothing that could move: the only process allowed to write is this one, and it is here.
    if datalock::held() {
        return read().await;
    }

    let dir = crate::utils::safe_data_dir();
    let mut answer = None;
    for attempt in 1..=ATTEMPTS {
        let before = datalock::observe(&dir);
        let value = read().await?;
        let after = datalock::observe(&dir);
        if before.spans_one_moment(after) {
            return Ok(value);
        }
        tracing::debug!(
            "a writer committed while this was being read (attempt {attempt} of {ATTEMPTS}); \
             reading again"
        );
        answer = Some(value);
    }

    tracing::debug!(
        "a writer committed during every attempt; reporting the last read, which may combine \
         facts from either side of it"
    );
    Ok(answer.expect("ATTEMPTS is at least one, so the loop ran at least once"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ordinary case, and the one that must cost nothing: nobody is writing, so the first
    /// read is the answer.
    #[tokio::test]
    async fn a_quiet_machine_reads_once() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let seen: usize = stable(|| async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(7)
        })
        .await
        .unwrap();
        assert_eq!(seen, 7);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a reader on a quiet machine must not read anything twice"
        );
    }

    /// An error from the read is the caller's error, not a reason to try again — retrying a
    /// failing read three times turns one message into three seconds of nothing.
    #[tokio::test]
    async fn a_failed_read_is_not_retried() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let outcome: Result<()> = stable(|| async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(crate::core::Error::Other("the file is gibberish".into()))
        })
        .await;
        assert!(outcome.is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
