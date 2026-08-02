//! Which managers forgive a non-zero exit code without being able to contradict it.
//!
//! `benign_exits` says "this code is not a failure". On its own that is a promise LiNix cannot
//! check: if the manager also writes the real outcome in prose, and the policy has no prose to
//! read, then every run that exits with a forgiven code is reported as a success — including
//! the ones that did nothing.
//!
//! CI 30684191791 was that, on chocolatey. `choco install bat` pulls eleven packages and
//! installs `bat` last; choco raises its exit code to 1 for a failed package only when nothing
//! has set one already, so a dependency asking for a reboot left 3010 standing over an install
//! of nothing. 3010 is forgiven here, choco's policy had no `failure_markers`, and LiNix
//! called it success. `list` could not show the package, PATH could not resolve it, and
//! `uninstall` said it was not installed — three red rows off one wrong "yes".
//!
//! The instructive part is the same as `absent_marker_coverage_tests.rs`: not that one manager
//! was wrong, but that **nothing anywhere counted the policies in that shape.** Two of them
//! were, both on Windows, and the pair was only found by reading the table by hand. So the
//! shape is a bound now, and a policy that gains a `benign_exits` entry without gaining a way
//! to contradict it turns this red instead of waiting for a runner to notice.

use linix::backends::create_default_registry;
use linix::core::exit_policy;
use std::sync::Arc;

/// Backends that forgive a code and cannot contradict it, with the reason each is allowed to.
///
/// Empty, and that is the measurement rather than the goal — `choco` and `winget` were the two
/// entries this list would have held on 2026-08-01, and both are closed.
///
/// An entry here is a **decision**, not a gap: it says the forgiven codes are unambiguous for
/// that manager, so no prose is needed to tell success from failure. Write the reason next to
/// the name. A manager whose forgiven code can stand over a failure does not belong here — it
/// belongs in `failure_markers`.
const FORGIVES_WITHOUT_CONTRADICTING: &[&str] = &[];

#[tokio::test]
async fn a_policy_that_forgives_an_exit_code_can_still_call_the_command_failed() {
    // The registry, not a hand-written list of manager names: a backend registered with a
    // policy nobody enumerated is exactly the case that goes unnoticed.
    let vfs = Arc::new(dashmap::DashMap::new());
    let mock = Arc::new(linix::core::executor::MockExecutor::new(vfs.clone()));
    let exec = linix::core::CommandExecutor::with_layer(
        true,
        false,
        mock,
        vfs,
        Arc::new(dashmap::DashMap::new()),
    );
    let config = linix::config::Config::default();
    let registry = create_default_registry(
        exec,
        &config,
        Arc::new(linix::app::hooks::LuaHooks::new(&config).expect("hooks")),
    )
    .await;

    let mut forgiving: Vec<String> = Vec::new();
    let mut mute: Vec<String> = Vec::new();

    for backend in registry.all() {
        let name = backend.name().to_string();
        let policy = exit_policy::for_manager(&name);
        if policy.benign_exits.is_empty() {
            continue;
        }
        forgiving.push(name.clone());
        // Either list can do the contradicting: a manager that prefixes its errors states a
        // convention, and matching the convention catches phrasings nobody has met yet.
        if policy.failure_markers.is_empty() && policy.failure_line_prefixes.is_empty() {
            mute.push(name);
        }
    }

    forgiving.sort();
    mute.sort();

    // Printed pass or fail. A bound nobody can see is the same as no bound.
    eprintln!(
        "benign-exit contradiction: {} of {} forgiving backends can call the command failed \
         anyway\n  forgiving: {}\n  mute:      {}",
        forgiving.len() - mute.len(),
        forgiving.len(),
        forgiving.join(" "),
        mute.join(" ")
    );

    assert!(
        !forgiving.is_empty(),
        "no backend forgives any exit code, so this test is measuring an empty table rather \
         than the property it claims to measure."
    );

    let unrecorded: Vec<&String> = mute
        .iter()
        .filter(|n| !FORGIVES_WITHOUT_CONTRADICTING.contains(&n.as_str()))
        .collect();
    assert!(
        unrecorded.is_empty(),
        "these backends forgive a non-zero exit code and have no `failure_markers` or \
         `failure_line_prefixes` to contradict it: {:?}\n\nEvery run that ends on a forgiven \
         code is reported to the user as a success, including the ones that did nothing — that \
         is CI 30684191791, where choco's 3010 stood over an install that installed no \
         package. Give it the manager's own failure phrasing in \
         `src/core/exit_policy::for_manager`, measured from that manager's output and never \
         guessed, or add it to `FORGIVES_WITHOUT_CONTRADICTING` with the reason its forgiven \
         codes are unambiguous.",
        unrecorded
    );

    // The other half, and the half a list never checks about itself.
    let stale: Vec<&&str> = FORGIVES_WITHOUT_CONTRADICTING
        .iter()
        .filter(|n| !mute.iter().any(|m| m == *n))
        .collect();
    assert!(
        stale.is_empty(),
        "these backends can now contradict a forgiven exit code and are still listed as unable \
         to: {:?}\nDelete them from `FORGIVES_WITHOUT_CONTRADICTING` — a list of known gaps \
         that keeps closed ones is how a bound stops meaning anything.",
        stale
    );
}
