//! `--dry-run` as a property of the run, not a habit of each verb.
//!
//! The flag was consulted per verb for as long as it has existed, and the result is the shape
//! this module deletes: `uninstall` checked it, `unmanage` checked it, `module create` checked
//! it — and `activate`, `deactivate`, `lock`, `git init` and `config init` did not. A preview
//! of "what happens if I switch to Work" left you switched to Work and printed nothing. That is
//! not five mistakes; it is one rule ("remember to check the flag") enforced by nothing, which
//! is the same finding as the guard that covered nine removal paths out of eleven.
//!
//! So the check moves to where the *write* happens. A verb added tomorrow inherits it by
//! calling the writer everything else calls, rather than by remembering a convention.
//!
//! **Why a process-wide value rather than a parameter.** `--dry-run` is a top-level flag parsed
//! once, before any command runs, and it applies to the whole process — there is no run in
//! which one write is a preview and another is not. Threading it to every write site would be
//! the per-verb habit again with a longer signature: a new call site would still have to be
//! given the flag by hand, and forgetting is exactly the failure being fixed. It is set once
//! from `main`, never from library code, and the setter is idempotent for a given run.

use std::sync::atomic::{AtomicBool, Ordering};

static DRY_RUN: AtomicBool = AtomicBool::new(false);

/// Record this process's `--dry-run` mode. Called once, from `main`, before dispatch.
pub fn set(on: bool) {
    DRY_RUN.store(on, Ordering::SeqCst);
}

/// Is this run a preview?
pub fn active() -> bool {
    DRY_RUN.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_not_a_preview_unless_something_says_so() {
        // The default matters more than it looks: a library caller that never sets the flag
        // must write for real, or `cargo test` and every embedding of this crate would
        // silently perform nothing.
        assert!(!active());
        set(true);
        assert!(active());
        set(false);
        assert!(!active());
    }
}
