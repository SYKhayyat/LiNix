//! One connection pool per policy, for the life of the process.
//!
//! A `reqwest::Client` **is** the connection pool, the TLS session cache and the resolver
//! cache. Eight places in this tree built one per request, so every OSV advisory GET, every
//! registry query and every asset download paid a full TCP handshake, a full TLS handshake and
//! a re-parse of the root store — to a host the previous request had just finished talking to.
//! Clients built here are cached by the policy that distinguishes them, and cloning one is an
//! `Arc` bump, so a caller that asks per request still gets the same pool.
//!
//! The key is the whole policy and not just the user agent: a client that would follow an
//! HTTPS→HTTP redirect must never be handed to a caller that refuses one (SEC2), and a client
//! with no timeout must never be handed to an API call.

use crate::core::{Error, Result};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::time::Duration;

/// Everything that makes two clients not interchangeable.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Policy {
    user_agent: String,
    /// `true` follows a redirect that leaves HTTPS. Only downloads that carry an explicit
    /// `@allow_http` may say true (SEC2).
    allow_downgrade: bool,
    /// `0` means no whole-request timeout, which is the only correct answer for a
    /// multi-gigabyte release asset and the wrong one for a JSON API call.
    timeout_secs: u64,
}

static POOL: Lazy<DashMap<Policy, reqwest::Client>> = Lazy::new(DashMap::new);

/// The shared client for this policy, building it the first time it is asked for.
pub fn client(
    user_agent: &str,
    allow_downgrade: bool,
    timeout_secs: u64,
) -> Result<reqwest::Client> {
    let key = Policy {
        user_agent: user_agent.to_string(),
        allow_downgrade,
        timeout_secs,
    };
    if let Some(existing) = POOL.get(&key) {
        return Ok(existing.clone());
    }
    let built = build(&key)?;
    POOL.insert(key, built.clone());
    Ok(built)
}

/// A pooled client for an API call: refuses a scheme downgrade, bounded by the configured
/// network timeout.
pub fn api(user_agent: &str, timeout_secs: u64) -> Result<reqwest::Client> {
    // A literal 0 reaches reqwest as "time out after zero seconds" — every request fails
    // instantly — rather than "no timeout", so an API caller's 0 is raised to 1 second here
    // and a caller that genuinely wants no bound asks `client` directly.
    client(user_agent, false, timeout_secs.max(1))
}

fn build(policy: &Policy) -> Result<reqwest::Client> {
    let redirect = if policy.allow_downgrade {
        reqwest::redirect::Policy::default()
    } else {
        // The binding requirement is that the *final* download is HTTPS; checking each hop is
        // the cheapest correct form and also catches a downgrade in the middle of a chain that
        // ends back on HTTPS.
        reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" {
                return attempt.error("redirected to a non-HTTPS URL");
            }
            if attempt.previous().len() >= 10 {
                return attempt.stop();
            }
            attempt.follow()
        })
    };
    let mut builder = reqwest::Client::builder()
        .user_agent(policy.user_agent.clone())
        .redirect(redirect);
    if policy.timeout_secs > 0 {
        builder = builder.timeout(Duration::from_secs(policy.timeout_secs));
    }
    builder.build().map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asking_twice_for_one_policy_builds_one_client() {
        let _ = client("linix-pool-a", false, 15).unwrap();
        let after_first = POOL.len();
        for _ in 0..20 {
            let _ = client("linix-pool-a", false, 15).unwrap();
        }
        assert_eq!(
            POOL.len(),
            after_first,
            "twenty more asks for the same policy built more clients — the pool is not pooling"
        );
    }

    #[test]
    fn a_client_that_would_follow_a_downgrade_is_never_handed_to_one_that_refuses() {
        let before = POOL.len();
        let _strict = client("linix-pool-b", false, 15).unwrap();
        let _loose = client("linix-pool-b", true, 15).unwrap();
        assert_eq!(
            POOL.len() - before,
            2,
            "the two redirect policies collapsed into one client — SEC2 would be enforced by \
             whichever caller happened to ask first"
        );
    }

    #[test]
    fn the_timeout_is_part_of_the_key() {
        let before = POOL.len();
        let _bounded = client("linix-pool-c", false, 15).unwrap();
        let _unbounded = client("linix-pool-c", false, 0).unwrap();
        assert_eq!(POOL.len() - before, 2);
    }

    #[test]
    fn an_api_client_never_asks_reqwest_for_a_zero_second_timeout() {
        // reqwest reads a zero-second timeout as "fail instantly", not "no bound", so an API
        // caller handed a configured 0 must not pass it through.
        let before = POOL.len();
        let _ = api("linix-pool-d", 0).unwrap();
        let after_zero = POOL.len();
        let _ = api("linix-pool-d", 1).unwrap();
        assert_eq!(
            POOL.len(),
            after_zero,
            "a 0-second API timeout was not raised to 1 — it built a distinct, instantly-failing \
             client"
        );
        assert_eq!(after_zero - before, 1);
    }
}
