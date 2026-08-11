//! U11, generalised (owner ruling, 2026-07-24): **a sync converges to the version that was
//! decided, not to the one that was published since.**
//!
//! The question was whether `watch` implies `--locked`. The answer was that it is not a `watch`
//! question at all — `sync` itself defaults to the recorded version, `watch` is `sync` with
//! nobody watching, and moving a version forward is a decision you ask for with `--upgrade`.
//!
//! Three modes, and this file pins all three:
//!
//! | mode | a recorded version | nothing recorded | a pin that disagrees |
//! |---|---|---|---|
//! | default | wins | resolves freely | the line wins |
//! | `--upgrade` | ignored | resolves freely | the line wins |
//! | `--locked` | wins | **error** | **error** |

use shall::app::sync::resolver::StateResolver;

use crate::mock_providers::TestKernel;

/// Write a lock recording `cargo:ripgrep` at an old version, plus a module declaring it.
async fn declare(kernel: &TestKernel, module_line: &str, locked_version: Option<&str>) {
    let root = kernel.app.config.config_root();
    std::fs::write(root.join("modules/tools.txt"), format!("{}\n", module_line)).unwrap();
    std::fs::write(root.join("profiles/Main"), "use tools\n").unwrap();
    let locks = root.join("locks");
    std::fs::create_dir_all(&locks).unwrap();
    if let Some(v) = locked_version {
        std::fs::write(
            locks.join("versions.json"),
            format!(r#"{{"locks":{{"cargo:ripgrep":"{}"}}}}"#, v),
        )
        .unwrap();
    }
}

async fn version_of(kernel: &TestKernel, upgrade: bool, locked: bool) -> Option<String> {
    let mut r = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), locked).await;
    if upgrade {
        r = r.upgrading();
    }
    let state = r.resolve_model().await.expect("resolves");
    state
        .packages
        .values()
        .flatten()
        .find(|s| s.name == "ripgrep")
        .and_then(|s| s.options.one("version").map(str::to_string))
}

/// The ruling, in one assertion: an ordinary `sync` takes the recorded version. Before this,
/// a fresh machine installed whatever upstream had published that morning.
#[tokio::test]
async fn an_ordinary_sync_takes_the_recorded_version() {
    let kernel = TestKernel::new().await;
    declare(&kernel, "cargo:ripgrep", Some("14.1.0")).await;
    assert_eq!(
        version_of(&kernel, false, false).await.as_deref(),
        Some("14.1.0"),
        "the default did not converge to the recorded version"
    );
}

/// `--upgrade` is the explicit opt-in that moving forward requires.
#[tokio::test]
async fn upgrade_ignores_what_was_recorded() {
    let kernel = TestKernel::new().await;
    declare(&kernel, "cargo:ripgrep", Some("14.1.0")).await;
    assert_eq!(
        version_of(&kernel, true, false).await,
        None,
        "--upgrade still pinned the recorded version"
    );
}

/// A machine that has never run `shall lock` has nothing to converge to, and that is the
/// ordinary state of a fresh install — not an error. This is the difference between the new
/// default and strict `--locked`.
#[tokio::test]
async fn nothing_recorded_is_not_an_error_by_default() {
    let kernel = TestKernel::new().await;
    declare(&kernel, "cargo:ripgrep", None).await;
    assert_eq!(version_of(&kernel, false, false).await, None);
}

/// Strict mode is unchanged and still means what it meant: reproduce this machine exactly, so
/// a package nobody locked is a gap in the reproduction.
#[tokio::test]
async fn locked_still_refuses_a_package_that_was_never_recorded() {
    let kernel = TestKernel::new().await;
    declare(&kernel, "cargo:ripgrep", None).await;
    let err = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), true)
        .await
        .resolve_model()
        .await
        .expect_err("strict mode must refuse an unlocked package");
    assert!(err.to_string().contains("missing from locks"), "{}", err);
}

/// A version you typed is a decision; the lock is a record of one. Outside strict mode the
/// line wins — quietly overriding what someone wrote would be the worst of both.
#[tokio::test]
async fn a_hand_written_pin_beats_the_lock_outside_strict_mode() {
    let kernel = TestKernel::new().await;
    declare(&kernel, "cargo:ripgrep@version=13.0.0", Some("14.1.0")).await;
    assert_eq!(
        version_of(&kernel, false, false).await.as_deref(),
        Some("13.0.0"),
        "the lock overrode a version the user wrote"
    );
}

/// ...and under strict mode the same disagreement is an error, because a reproduction that
/// silently picks one of two answers has reproduced neither.
#[tokio::test]
async fn a_pin_that_disagrees_with_the_lock_is_an_error_under_locked() {
    let kernel = TestKernel::new().await;
    declare(&kernel, "cargo:ripgrep@version=13.0.0", Some("14.1.0")).await;
    let err = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), true)
        .await
        .resolve_model()
        .await
        .expect_err("a mismatch must be reported");
    assert!(err.to_string().contains("version mismatch"), "{}", err);
}

// ============================================================================
// Q53: THE FOURTH MODE — A MANAGER THAT CANNOT BE ASKED FOR A VERSION
// ============================================================================

/// **A recorded version is never fed back to a manager that cannot replay it.**
///
/// This file's three modes above are about *which* version wins. This is about whether a
/// version can be sent at all, and it is a different question: `brew` reports versions happily
/// and accepts none, because Homebrew's `name@version` is a different formula's *name*. Feeding
/// a record back there built `tokei@14.0.0`, which exists nowhere, and the sync failed for ever
/// on a pin nobody typed (`S85`).
///
/// The record itself stays — reading a version is all drift detection needs, and `check` is
/// where that comparison lives now.
#[tokio::test]
async fn a_recorded_version_is_not_replayed_on_a_manager_that_cannot_take_one() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();
    std::fs::write(root.join("modules/tools.txt"), "brew:ripgrep\n").unwrap();
    std::fs::write(root.join("profiles/Main"), "use tools\n").unwrap();
    let locks = root.join("locks");
    std::fs::create_dir_all(&locks).unwrap();
    std::fs::write(
        locks.join("versions.json"),
        r#"{"locks":{"brew:ripgrep":"14.1.0"}}"#,
    )
    .unwrap();

    let state = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false)
        .await
        .resolve_model()
        .await
        .expect("resolves");
    let pinned = state
        .packages
        .values()
        .flatten()
        .find(|s| s.name == "ripgrep")
        .and_then(|s| s.options.one("version").map(str::to_string));
    assert_eq!(
        pinned, None,
        "a version brew cannot install was injected anyway, which is how `tokei@14.0.0` got built"
    );
}

/// The lockfile is still written for that manager, because the two jobs are not the same job:
/// *replay* needs the manager to accept a version, *detect drift* needs it only to report one.
/// A test that only checked the injection above would be satisfied by deleting the record.
#[tokio::test]
async fn the_record_survives_even_where_it_cannot_be_replayed() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();
    let locks = root.join("locks");
    std::fs::create_dir_all(&locks).unwrap();
    let body = r#"{"locks":{"brew:ripgrep":"14.1.0"}}"#;
    std::fs::write(locks.join("versions.json"), body).unwrap();
    std::fs::write(root.join("modules/tools.txt"), "brew:ripgrep\n").unwrap();
    std::fs::write(root.join("profiles/Main"), "use tools\n").unwrap();

    let _ = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false)
        .await
        .resolve_model()
        .await
        .expect("resolves");

    let after = std::fs::read_to_string(locks.join("versions.json")).unwrap();
    assert!(
        after.contains("14.1.0"),
        "resolving dropped the record it cannot replay: {after}"
    );
}
