//! Compiled regexes, kept.
//!
//! `Regex::new` builds and optimises an automaton. Eleven places in this tree called it inside
//! the loop that used the result — per snapshot listed, per service row parsed, per package
//! line, per diagnostic rule per failure — so the same handful of patterns were recompiled
//! thousands of times in one command.
//!
//! Patterns that are literals in the source belong in a `Lazy` beside their use. This is for
//! the ones that come from **configuration**: a snapshot definition's `list_pattern`, a service
//! adapter's row pattern, an onboarder's extraction rule. Those cannot be `Lazy` — they are not
//! known until a file is read — but they are just as fixed for the length of a run.

use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::Arc;

static CACHE: Lazy<dashmap::DashMap<String, std::result::Result<Arc<Regex>, String>>> =
    Lazy::new(dashmap::DashMap::new);

/// How many distinct patterns this cache will hold before it stops growing.
///
/// **The cache is per *process*, and until `watch` existed every process was one command.** A
/// command reads a bounded set of configuration — a handful of snapshot definitions, service
/// adapters and onboarder rules — so "never evicted" and "bounded" were the same thing and the
/// cache was free.
///
/// `watch` is the only caller that makes the process long-lived: it reconciles on a tick,
/// forever, re-reading configuration that a `--pull` may have just changed. There the two come
/// apart, and never-evicted becomes a slow leak — one automaton per pattern the config has ever
/// held, for as long as the daemon runs.
///
/// The number is generous on purpose. This tree's whole configuration surface is tens of
/// patterns; a machine that has legitimately seen a thousand distinct ones is a machine whose
/// config is being rewritten by something, and the cache going cold is the right outcome there
/// rather than unbounded memory.
const CAPACITY: usize = 1024;

/// This pattern, compiled once per process.
///
/// A pattern that will not compile is remembered as an error too: re-attempting a compile that
/// has already failed costs the same as one that succeeds, and the caller gets the same message
/// either way.
pub fn compiled(pattern: &str) -> std::result::Result<Arc<Regex>, String> {
    if let Some(hit) = CACHE.get(pattern) {
        return hit.clone();
    }
    let built = Regex::new(pattern).map(Arc::new).map_err(|e| e.to_string());
    // Cleared rather than evicted one-by-one, deliberately: an LRU needs a recency order, which
    // means a second structure and a write on every *read* — and reads are the hot path this
    // cache exists for. Dropping everything at the ceiling costs one recompile per live pattern,
    // once, on a boundary a normal command never reaches at all.
    if CACHE.len() >= CAPACITY {
        CACHE.clear();
    }
    CACHE.insert(pattern.to_string(), built.clone());
    built
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_pattern_is_compiled_once() {
        let a = compiled(r"^shall-cache-test-(\d+)$").unwrap();
        let b = compiled(r"^shall-cache-test-(\d+)$").unwrap();
        assert!(
            Arc::ptr_eq(&a, &b),
            "the second ask built a second automaton"
        );
    }

    #[test]
    fn a_pattern_that_will_not_compile_says_so_every_time() {
        let first = compiled(r"([unclosed");
        let second = compiled(r"([unclosed");
        assert!(first.is_err());
        assert_eq!(first.err(), second.err(), "the message must not drift");
    }

    #[test]
    fn a_cached_regex_still_matches() {
        let re = compiled(r"^v(\d+)\.(\d+)$").unwrap();
        assert!(re.is_match("v1.2"));
        assert!(!re.is_match("1.2"));
    }
}

#[cfg(test)]
mod bound_tests {
    use super::*;

    /// The cache is bounded, and the bound is reachable.
    ///
    /// `watch` is the only caller that makes this process long-lived — a `sync` exits and takes
    /// its cache with it — so the leak this bound prevents is invisible to every other command
    /// and to every test that does not look for it.
    #[test]
    fn the_cache_stops_growing() {
        for i in 0..(CAPACITY + 50) {
            let _ = compiled(&format!(r"^shall-bound-test-{i}-(\d+)$"));
        }
        assert!(
            CACHE.len() <= CAPACITY,
            "the cache holds {} patterns against a ceiling of {}; a `watch` that runs for a \
             week accumulates one automaton per pattern its config has ever held",
            CACHE.len(),
            CAPACITY
        );
        // And it is still a cache afterwards: the ceiling must not turn it into a no-op, which
        // would trade a slow leak for recompiling on every line of every listing.
        let a = compiled(r"^shall-bound-still-caching-(\d+)$").unwrap();
        let b = compiled(r"^shall-bound-still-caching-(\d+)$").unwrap();
        assert!(
            Arc::ptr_eq(&a, &b),
            "past the ceiling the cache stopped caching"
        );
    }
}
