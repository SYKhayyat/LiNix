//! One piece of unfinishable work must not leave every other piece unfinished.
//!
//! `sync` stops at the first failure and rolls back, and must keep doing so: a plan is one
//! change to one machine, so a member that fails makes the whole plan wrong. Recovery is the
//! opposite shape — each entry is a separate piece of work a run that already died left behind.
//! `heal` used to get that property from a hand-rolled serial loop beside the transaction
//! engine, which cost it every other property the engine has: measured on one host in one
//! minute, `sync --dry-run` ran at 3.9x overlap over 2 waves and `heal` at **0.2x over 27 waves
//! for 27 commands**, which is the definition of serial.
//!
//! So the engine grew the mode instead, and this pins both halves of it: what still stops, and
//! what now carries on.

use linix::core::{GraphAction, PackageSpec, Transaction, TransactionConfig};
use petgraph::stable_graph::StableDiGraph;

use crate::mock_providers::TestKernel;

fn spec(backend: &str, name: &str, requires: &[&str]) -> PackageSpec {
    PackageSpec {
        name: name.into(),
        backend: backend.into(),
        options: Default::default(),
        requires: requires.iter().map(|s| s.to_string()).collect(),
        present: true,
    }
}

/// A node fails, a node that needs it, and a node that has nothing to do with either. The
/// unrelated one is the point: it is the twenty-nine other interrupted operations on a machine
/// where one of them can never be finished.
fn graph_with_one_doomed_node() -> (
    StableDiGraph<GraphAction, ()>,
    petgraph::stable_graph::NodeIndex,
) {
    let mut graph = StableDiGraph::new();
    let unrelated = graph.add_node(GraphAction::Install(spec("brew", "unrelated", &[])));
    // No manager by this name is registered, so the engine fails the node without inventing an
    // argv this test would then be asserting about.
    let doomed = graph.add_node(GraphAction::Install(spec("nosuchpm", "doomed", &[])));
    let waiting = graph.add_node(GraphAction::Install(spec(
        "brew",
        "waiting",
        &["nosuchpm:doomed"],
    )));
    graph.add_edge(doomed, waiting, ());
    (graph, unrelated)
}

fn recovery_config() -> TransactionConfig {
    TransactionConfig {
        auto_rollback: false,
        continue_on_error: true,
        max_retries: 0,
        ..TransactionConfig::patient()
    }
}

#[tokio::test]
async fn recovery_finishes_the_work_a_doomed_entry_is_not_holding_up() {
    let kernel = TestKernel::new().await;
    let (graph, _) = graph_with_one_doomed_node();

    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        recovery_config(),
    );
    let results = tx
        .execute_with_telemetry()
        .await
        .expect("recovery reports per entry rather than failing as a whole");

    let outcome = |name: &str| {
        &results
            .iter()
            .find(|r| r.package_name == name)
            .unwrap_or_else(|| panic!("{name} is missing from the report entirely"))
            .result
    };

    assert_eq!(
        results.len(),
        3,
        "every node is accounted for, including the one nobody ran: an entry that vanishes \
         from the report is the shape of bug this whole file exists for — {:?}",
        results.iter().map(|r| &r.package_name).collect::<Vec<_>>()
    );
    assert!(
        outcome("unrelated").is_ok(),
        "the doomed entry stopped an unrelated one: {:?}",
        outcome("unrelated")
    );
    assert!(outcome("doomed").is_err());

    // Not "waiting failed". It was never attempted, and the report has to say which — telling
    // someone a package failed when no command was ever run for it is the misattribution this
    // engine is supposed to be free of.
    let blocked = outcome("waiting")
        .as_ref()
        .expect_err("a node whose dependency failed cannot have succeeded")
        .to_string();
    assert!(
        blocked.contains("not attempted") && blocked.contains("nosuchpm:doomed"),
        "a skipped node must name the one that stopped it, got: {blocked}"
    );
}

/// The other half, and the one that must not have moved. A `sync` is one change to one machine.
#[tokio::test]
async fn a_sync_still_stops_at_the_first_failure() {
    let kernel = TestKernel::new().await;
    let (graph, _) = graph_with_one_doomed_node();

    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        TransactionConfig {
            max_retries: 0,
            ..TransactionConfig::patient()
        },
    );
    assert!(
        tx.execute_with_telemetry().await.is_err(),
        "the default is still all-or-nothing; `continue_on_error` is opt-in and recovery is \
         the only thing that opts in"
    );
    assert!(
        !TransactionConfig::patient().continue_on_error,
        "the default must be the sync's, or every plan quietly becomes best-effort"
    );
}
