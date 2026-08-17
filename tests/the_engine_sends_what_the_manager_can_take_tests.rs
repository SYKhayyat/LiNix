//! **What the executor decides before a manager is invoked** — the version it strips, the verb
//! it picks, and what a rollback puts back.
//!
//! Four branches in `execute_batch_with_retry` and two in `rollback` read a *capability* and
//! change the command line accordingly. Every one of them survived the mutation sweep, and the
//! reason is one fact about the suite rather than six oversights: the batching tests drive the
//! kernel's registry, where `pins_version()` and `supports_purge()` each have exactly one answer
//! available, so no test could put the branch under load in both directions.
//!
//! The one that matters most is the version strip. It shipped a real defect — `brew.rs` built
//! `pkg-a@1.0`, a formula name that does not exist, so a rollback failed on a Mac and passed in
//! every test, because a mock matches any string. Its guard is asserted here in all three of the
//! states it distinguishes: a manager that pins, a manager that does not, and a "version" that
//! is not a version.

use petgraph::stable_graph::StableDiGraph;
use shall::app::sync::guard::{GuardScope, Reaped};
use shall::backends::BackendRegistry;
use shall::core::{GraphAction, PackageSpec, Transaction, TransactionConfig};
use std::sync::Arc;
use std::time::Duration;

use crate::mock_providers::recording_backend::{capabilities, shared_log, RecordingBackend};
use crate::mock_providers::TestKernel;

/// Why every transaction here authorises its own removals: none of them is testing the guard,
/// and threading a real config and registry through to mint a token proves nothing about it.
const NOT_THE_SUBJECT: &str = "these tests drive the executor's argv decisions, not the guard";

fn spec(name: &str, backend: &str, version: Option<&str>) -> PackageSpec {
    let mut options = shall::config::grammar::Options::default();
    if let Some(v) = version {
        options.set("version", v);
    }
    PackageSpec {
        name: name.into(),
        backend: backend.into(),
        options,
        requires: vec![],
        present: true,
    }
}

/// The retry loop is not the subject of any test in this file, so nothing here waits one out.
fn one_attempt(purge: bool) -> TransactionConfig {
    TransactionConfig {
        max_retries: 0,
        purge,
        node_timeout: Duration::from_secs(30),
        ..TransactionConfig::default()
    }
}

fn registry_of(backends: &[&Arc<RecordingBackend>]) -> Arc<BackendRegistry> {
    let mut registry = BackendRegistry::new();
    for b in backends {
        registry.register(capabilities(b));
    }
    Arc::new(registry)
}

/// A version reaches the command line of a manager that can be asked for one.
///
/// The control for the two below it. Without it, "the version was stripped" and "the version was
/// never sent" are the same observation, and a gate that strips unconditionally passes.
#[tokio::test]
async fn a_manager_that_pins_is_sent_the_version() {
    let kernel = TestKernel::new().await;
    let log = shared_log();
    let backend = RecordingBackend::named("mock-pinning", &log)
        .pinning()
        .build();

    let mut graph = StableDiGraph::new();
    graph.add_node(GraphAction::Install(spec(
        "pkg-pinned",
        "mock-pinning",
        Some("1.6"),
    )));

    let mut tx = Transaction::with_config(
        graph,
        registry_of(&[&backend]),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        one_attempt(false),
    );
    tx.execute().await.expect("the install succeeds");

    assert_eq!(
        backend.calls(),
        vec!["mock-pinning install pkg-pinned@1.6"],
        "a manager that can be asked for a version must be asked for the one that was declared"
    );
}

/// A version does not reach the command line of a manager that cannot be asked for one.
///
/// Stripped rather than refused, and **named**: refusing would end a rollback with the package
/// uninstalled, which is worse than putting it back at whatever the manager offers.
#[tokio::test]
async fn a_manager_that_cannot_pin_is_not_sent_the_version() {
    let kernel = TestKernel::new().await;
    let log = shared_log();
    let backend = RecordingBackend::named("mock-floating", &log).build();

    let mut graph = StableDiGraph::new();
    graph.add_node(GraphAction::Install(spec(
        "pkg-floating",
        "mock-floating",
        Some("1.6"),
    )));

    let mut tx = Transaction::with_config(
        graph,
        registry_of(&[&backend]),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        one_attempt(false),
    );
    tx.execute().await.expect("the install succeeds");

    assert_eq!(
        backend.calls(),
        vec!["mock-floating install pkg-floating"],
        "`pkg-floating@1.6` is the shape of the formula name that does not exist"
    );
}

/// `@version=latest` is not a version to strip, on the manager that cannot pin one.
///
/// The guard is `concrete_version`, not "is there a version key". `latest` and `*` name whatever
/// the manager offers, which is what an unpinnable manager was going to install anyway — so
/// stripping them changes nothing and warning about them says a version was lost when none was.
#[tokio::test]
async fn latest_is_not_a_version_to_strip() {
    let kernel = TestKernel::new().await;
    let log = shared_log();
    let backend = RecordingBackend::named("mock-floating", &log).build();

    let mut graph = StableDiGraph::new();
    graph.add_node(GraphAction::Install(spec(
        "pkg-rolling",
        "mock-floating",
        Some("latest"),
    )));

    let mut tx = Transaction::with_config(
        graph,
        registry_of(&[&backend]),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        one_attempt(false),
    );
    tx.execute().await.expect("the install succeeds");

    assert_eq!(
        backend.calls(),
        vec!["mock-floating install pkg-rolling@latest"],
        "the strip is for a concrete version the manager cannot honour, and `latest` is not one"
    );
}

/// Purge happens when the run asks for it **and** the manager draws the distinction — the whole
/// matrix, because either half alone reads as the same pass.
///
/// A deleted module line says "stop installing this", which is not the sentence "destroy how I
/// had it set up". So a run that did not ask must not purge, on a manager that could; and a run
/// that did ask gets an ordinary removal from a manager that cannot, rather than a refusal.
#[tokio::test]
async fn purging_needs_both_the_run_to_ask_and_the_manager_to_offer() {
    for (run_asks, manager_offers, expected) in [
        (false, true, "mock-remover remove pkg-gone"),
        (true, true, "mock-remover purge pkg-gone"),
        (true, false, "mock-remover remove pkg-gone"),
        (false, false, "mock-remover remove pkg-gone"),
    ] {
        let kernel = TestKernel::new().await;
        let log = shared_log();
        let mut backend = RecordingBackend::named("mock-remover", &log);
        if manager_offers {
            backend = backend.purging();
        }
        let backend = backend.holding("pkg-gone", Some("3.0")).build();

        let mut graph = StableDiGraph::new();
        graph.add_node(GraphAction::Remove {
            name: "pkg-gone".into(),
            backend: "mock-remover".into(),
        });

        let mut tx = Transaction::with_config(
            graph,
            registry_of(&[&backend]),
            kernel.app.journal.clone(),
            kernel.app.diagnostics.clone(),
            kernel.app.config.clone(),
            one_attempt(run_asks),
        )
        .guarded_by(Reaped::for_reason(GuardScope::Sync, NOT_THE_SUBJECT));
        tx.execute().await.expect("the removal succeeds");

        assert_eq!(
            backend.calls(),
            vec![expected.to_string()],
            "run asked to purge: {run_asks}; manager offers purge: {manager_offers}"
        );
    }
}

/// A rollback puts a removed package back at the version it was on.
///
/// Two decisions, one after the other, and each of them silently survives being reversed. The
/// first is whether to reinstate at all — `Prior::Absent` means nothing was there to lose, and
/// every other prior means something was. The second is the version: dropping it reinstates the
/// package at whatever is newest, so a rolled-back removal quietly loses the pin and comes back
/// as a version nobody declared.
#[tokio::test]
async fn a_rolled_back_removal_comes_back_at_the_version_it_was_on() {
    let kernel = TestKernel::new().await;
    let log = shared_log();
    let keeper = RecordingBackend::named("mock-keeper", &log)
        .holding("pkg-keep", Some("2.1.0"))
        .build();
    let doomed = RecordingBackend::named("mock-doomed", &log)
        .failing("pkg-doomed")
        .build();

    // The removal runs first and succeeds; the install behind it fails, which is what calls the
    // rollback. The edge is what makes that order a fact rather than a race.
    let mut graph = StableDiGraph::new();
    let removed = graph.add_node(GraphAction::Remove {
        name: "pkg-keep".into(),
        backend: "mock-keeper".into(),
    });
    let fails = graph.add_node(GraphAction::Install(spec(
        "pkg-doomed",
        "mock-doomed",
        None,
    )));
    graph.add_edge(removed, fails, ());

    let mut tx = Transaction::with_config(
        graph,
        registry_of(&[&keeper, &doomed]),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        one_attempt(false),
    )
    .guarded_by(Reaped::for_reason(GuardScope::Sync, NOT_THE_SUBJECT));

    tx.execute()
        .await
        .expect_err("the second node fails, which is what this test is for");

    assert_eq!(
        keeper.calls(),
        vec![
            "mock-keeper remove pkg-keep".to_string(),
            "mock-doomed install pkg-doomed".to_string(),
            "mock-keeper install pkg-keep@2.1.0".to_string(),
        ],
        "the rollback must reinstall what the run removed, at the version it found there"
    );
}
