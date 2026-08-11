//! `src/verbs/` was declared `mod verbs;` in `main.rs` — private to the binary, absent from
//! `lib.rs` — so **~8,500 lines of real logic could not be linked to by any of the ninety-odd
//! test binaries.** The lock/unlock ledger, `check_health`, the failure-attribution classifier
//! and `reconcile` itself could only be exercised by spawning the program, and four of the ten
//! files had no `#[cfg(test)]` block either, so in practice they were exercised by nothing.
//!
//! That was the module boundary in the wrong place, and it is also what blocked `F-3`'s
//! preferred fix: `app/profile.rs` could not call `verbs::sync::reconcile` to stop being a
//! second reconcile loop, because `app/` cannot reach a module private to the binary.
//!
//! This file is the proof the boundary moved, and it pays for itself by testing two pure
//! functions that had no coverage at all.

use shall::core::{ManagedPackage, StateRegistry};
use shall::verbs::plan::unverified_packages;
use shall::verbs::upgrade::upgrade_excluded;
use std::collections::HashMap;
use std::path::PathBuf;

fn managed(backend: &str, name: &str, opts: &[(&str, &str)]) -> ManagedPackage {
    ManagedPackage {
        name: name.into(),
        backend: backend.into(),
        version: None,
        installed_at: 0,
        expires_at: None,
        options: opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        source: "sync".into(),
        is_transient: false,
        session_id: None,
    }
}

/// `upgrade --except` decides what a bulk upgrade leaves alone, and got that decision wrong
/// silently: an unmatched exclusion upgrades the package the user asked to hold back.
#[test]
fn except_matches_a_bare_name_a_qualified_name_and_neither_more() {
    assert!(upgrade_excluded(&["jq".into()], "apt", "jq"));
    assert!(upgrade_excluded(&["apt:jq".into()], "apt", "jq"));

    // Case-insensitively on the bare name — `winget` and `choco` answer with title case, so a
    // user typing what `list` printed must still be excluded.
    assert!(upgrade_excluded(&["JQ".into()], "apt", "jq"));

    // But the qualified form is exact: `apt:jq` must not hold back `brew:jq`, because those are
    // two packages and the user named one of them.
    assert!(!upgrade_excluded(&["apt:jq".into()], "brew", "jq"));
    assert!(!upgrade_excluded(&["jqq".into()], "apt", "jq"));
    assert!(!upgrade_excluded(&[], "apt", "jq"));
}

/// `@unverified` has to stay visible after the install, or the flag was not a real decision.
/// This reads the *recorded option* rather than asking the backend, so a backend that gains
/// the flag is listed without editing anything — which is exactly the property worth pinning.
#[test]
fn unverified_is_read_from_what_was_recorded_not_from_the_backend() {
    let mut state = StateRegistry::new(PathBuf::from("unused.json"));
    state.packages = vec![
        managed("github", "fd", &[("unverified", "true")]),
        managed("apt", "jq", &[]),
        // The option present and false is not the option absent, and must not be listed.
        managed("brew", "rg", &[("unverified", "false")]),
        // A backend nothing has taught this function about, carrying the flag: still listed.
        managed("some-future-backend", "thing", &[("unverified", "true")]),
    ];

    let flagged: Vec<(String, String)> = unverified_packages(&state);
    assert_eq!(
        flagged,
        vec![
            ("github".to_string(), "fd".to_string()),
            ("some-future-backend".to_string(), "thing".to_string()),
        ],
        "the list is what was recorded, in registry order"
    );
    let _ = HashMap::<String, String>::new();
}
