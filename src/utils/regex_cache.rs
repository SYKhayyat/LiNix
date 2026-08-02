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
    CACHE.insert(pattern.to_string(), built.clone());
    built
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_pattern_is_compiled_once() {
        let a = compiled(r"^linix-cache-test-(\d+)$").unwrap();
        let b = compiled(r"^linix-cache-test-(\d+)$").unwrap();
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
