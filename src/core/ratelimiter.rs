use crate::core::{Error, Result};
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Jitter, Quota, RateLimiter as GovRateLimiter};
use std::num::NonZeroU32;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::{debug, warn};

type Governor = GovRateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// A permit issuer that does not exist until a permit is asked for.
///
/// **Built on first use, not in the constructor.** A backend's `new` runs for every subcommand,
/// including the ones that touch no network at all — `github`'s ran on `shall path` and cost
/// 200ms building a clock for an API budget the run never spent (AU3). Anything a rate limiter
/// costs, it costs the first request; a run with no requests pays nothing.
///
/// `Arc<OnceLock<_>>` rather than `OnceLock` inside a clone: the cell is what the clones share,
/// so two backends holding copies of one quota still hold ONE quota. A per-clone cell would
/// silently double every limit here.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<OnceLock<Governor>>,
    quota: Quota,
    description: String,
}

impl RateLimiter {
    pub fn new(requests_per_minute: u32, description: &str) -> Self {
        // The clamp is what makes the `expect` below unreachable: a caller-supplied 0 is a
        // configuration mistake, not a request to block every call forever.
        let rpm = requests_per_minute.max(1);
        let quota = Quota::per_minute(NonZeroU32::new(rpm).expect("RPM is guaranteed > 0"));

        Self {
            inner: Arc::new(OnceLock::new()),
            quota,
            description: description.to_string(),
        }
    }

    /// The issuer, built now if this is the first permit anyone has asked this limiter for.
    fn governor(&self) -> &Governor {
        self.inner
            .get_or_init(|| GovRateLimiter::direct(self.quota))
    }

    /// Whether a permit has ever been asked for, and so whether the issuer exists.
    ///
    /// Public because the cost this avoids is invisible from the outside — a startup budget can
    /// measure that the total is small, but only this can say the limiter is the reason.
    pub fn is_engaged(&self) -> bool {
        self.inner.get().is_some()
    }

    pub fn github() -> Self {
        // GitHub allows 60 requests per hour for unauthenticated IPs.
        Self::new(1, "GitHub (Unauthenticated)")
    }

    pub fn github_authenticated() -> Self {
        // GitHub allows 5,000 requests per hour for authenticated users; ~80/min stays inside
        // the window even if every minute is used to the limit.
        Self::new(80, "GitHub (Authenticated)")
    }

    pub fn vscode_marketplace() -> Self {
        Self::new(30, "VS Code Marketplace")
    }

    pub async fn wait(&self) -> Result<()> {
        debug!("RateLimiter [{}]: Waiting for permit...", self.description);
        self.governor().until_ready().await;
        Ok(())
    }

    pub async fn execute<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        // Jitter desynchronizes parallel workers that would otherwise all wake on the same
        // permit boundary and burst.
        let jitter = Jitter::up_to(Duration::from_millis(150));

        self.governor().until_ready_with_jitter(jitter).await;

        match f().await {
            Ok(val) => Ok(val),
            Err(e) => {
                // Read off the variant, not off the rendered message: `format!("{:?}")` also
                // matched any error that happened to contain "429" — a version string, a
                // package name — and missed a real rate limit whose text said neither.
                if matches!(e, Error::RateLimit(_)) {
                    warn!(
                        "RateLimiter [{}]: Remote API returned 429 (Too Many Requests). Local limits may need tightening.",
                        self.description
                    );
                }
                Err(e)
            }
        }
    }

    pub fn description(&self) -> &str {
        &self.description
    }
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("description", &self.description)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The comment on `inner` is a promise, and this is that promise as a test.** It says a
    /// per-clone cell "would silently double every limit here" — silently being the problem, so
    /// the shape has to be asserted rather than described. Two backends holding clones of one
    /// quota must hold ONE issuer.
    #[tokio::test]
    async fn clones_share_one_permit_issuer() {
        let a = RateLimiter::new(600, "shared");
        let b = a.clone();
        assert!(!a.is_engaged());
        assert!(!b.is_engaged());

        a.wait().await.unwrap();

        assert!(a.is_engaged());
        assert!(
            b.is_engaged(),
            "the clone built an issuer of its own, so the two of them hold twice the quota the \
             caller asked for and nothing says so"
        );
    }

    /// `AU3`: a backend's `new` runs for every subcommand, including the ones that touch no
    /// network at all. `github`'s cost 200 ms building a clock for a budget the run never spent.
    #[test]
    fn a_limiter_nobody_asked_a_permit_of_is_never_built() {
        let l = RateLimiter::new(60, "unused");
        assert!(!l.is_engaged(), "constructing the limiter built its issuer");
        assert_eq!(l.description(), "unused");
    }

    /// A caller-supplied zero is a configuration mistake, not a request to block every call for
    /// ever. The clamp in `new` is what makes its `expect` unreachable — without it this panics
    /// at construction rather than failing a request.
    #[tokio::test]
    async fn zero_requests_per_minute_is_clamped_rather_than_fatal() {
        let l = RateLimiter::new(0, "zero");
        tokio::time::timeout(Duration::from_secs(5), l.wait())
            .await
            .expect("a limiter clamped from zero would not issue even its first permit")
            .unwrap();
    }

    /// **The limiter limits, which is the only reason it is a dependency.**
    ///
    /// Asserted by the second permit *not* arriving rather than by waiting a minute for it: at
    /// one request per minute the burst is one, so the first permit is immediate and the second
    /// cannot be. A build where the quota did nothing passes every other test in this file.
    #[tokio::test]
    async fn the_permit_after_the_burst_does_not_arrive_at_once() {
        let l = RateLimiter::new(1, "one per minute");
        tokio::time::timeout(Duration::from_millis(500), l.wait())
            .await
            .expect("the first permit was not immediate")
            .unwrap();

        assert!(
            tokio::time::timeout(Duration::from_millis(300), l.wait())
                .await
                .is_err(),
            "a one-per-minute limiter issued two permits inside 300 ms, so the quota is not \
             being enforced at all"
        );
    }

    /// And the other direction, without which the test above passes against a limiter that
    /// blocks everything: inside its burst, a generous quota does not make the caller wait.
    #[tokio::test]
    async fn a_generous_quota_issues_its_burst_without_waiting() {
        let l = RateLimiter::new(600, "600 per minute");
        tokio::time::timeout(Duration::from_secs(2), async {
            for _ in 0..5 {
                l.wait().await.unwrap();
            }
        })
        .await
        .expect("five permits from a 600/minute limiter did not arrive within two seconds");
    }

    /// **A token changes the local budget as well as the remote one**, which is why handing the
    /// harness containers a `GITHUB_TOKEN` turned the `github:` lifecycle from a rate-limit
    /// casualty into an ordinary install: 1 request per minute against 80.
    #[tokio::test]
    async fn the_authenticated_github_limiter_is_not_the_anonymous_one() {
        assert_eq!(
            RateLimiter::github().description(),
            "GitHub (Unauthenticated)"
        );
        assert_eq!(
            RateLimiter::github_authenticated().description(),
            "GitHub (Authenticated)"
        );

        // Behaviour, not the label. The authenticated quota issues a burst the other cannot.
        let auth = RateLimiter::github_authenticated();
        tokio::time::timeout(Duration::from_secs(2), async {
            for _ in 0..5 {
                auth.wait().await.unwrap();
            }
        })
        .await
        .expect("the authenticated limiter stalled inside its own burst");

        let anon = RateLimiter::github();
        anon.wait().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(300), anon.wait())
                .await
                .is_err(),
            "the anonymous limiter issued a second permit at once, so the two are the same \
             budget and the token buys nothing locally"
        );
    }

    /// `execute` returns what the closure returned, and hands a failure back unchanged. The
    /// `RateLimit` arm logs and must not swallow or reclassify — the class it carries is the
    /// one `VI.11` is about.
    #[tokio::test]
    async fn execute_passes_the_value_through_and_the_error_unchanged() {
        let l = RateLimiter::new(600, "execute");

        let v: u32 = l.execute(|| async { Ok(7u32) }).await.unwrap();
        assert_eq!(v, 7);

        let e = l
            .execute(|| async { Err::<(), _>(Error::RateLimit("429".into())) })
            .await
            .expect_err("execute swallowed a failure");
        assert!(
            matches!(e, Error::RateLimit(_)),
            "execute reclassified the failure it was handed: {e:?}"
        );
    }
}
