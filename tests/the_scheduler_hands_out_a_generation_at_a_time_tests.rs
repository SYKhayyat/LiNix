//! **The execution loop's own arithmetic** — what it counts as a wave, when it calls a node
//! ready, how much work a resumed run says it has, and what it does with a graph it cannot
//! schedule at all.
//!
//! Every property here is about the loop rather than about a package, and each is invisible from
//! the outside: three of the four leave a correct-looking run behind when they are wrong. A node
//! dispatched with a dependency outstanding still reports `Ok`; a wave counter that over-counts
//! only ever makes the latency rule complain about a run that was fine; a resumed transaction
//! that reports the whole graph as remaining files a shape nobody had. They are asserted on the
//! numbers the engine records, not on the log it prints, because a warning nothing can read is
//! the shape of a gate that cannot fail.
//!
//! **Two mock backends, deliberately.** Batching groups a ready set by manager, so two
//! independent nodes on one manager are one command and one task — which collapses exactly the
//! interleaving these tests exist to produce. Two managers means two tasks in flight, and
//! `join_next` takes one at a time, so "something else is still running" is a fact of the
//! arrangement rather than a race the test hopes for.

use petgraph::stable_graph::StableDiGraph;
use shall::backends::BackendRegistry;
use shall::core::{GraphAction, PackageSpec, Transaction, TransactionConfig};
use std::sync::Arc;
use std::time::Duration;

use crate::mock_providers::recording_backend::{
    capabilities, shared_log, CallLog, RecordingBackend,
};
use crate::mock_providers::TestKernel;

fn install(name: &str, backend: &str) -> GraphAction {
    GraphAction::Install(PackageSpec {
        name: name.into(),
        backend: backend.into(),
        options: Default::default(),
        requires: vec![],
        present: true,
    })
}

fn two_managers(
    log: &CallLog,
) -> (
    Arc<RecordingBackend>,
    Arc<RecordingBackend>,
    Arc<BackendRegistry>,
) {
    let first = RecordingBackend::named("mock-one", log).build();
    let second = RecordingBackend::named("mock-two", log).build();
    let mut registry = BackendRegistry::new();
    registry.register(capabilities(&first));
    registry.register(capabilities(&second));
    (first, second, Arc::new(registry))
}

/// Enough concurrency for both managers to be in flight at once, and no retries to wait out.
///
/// **`total_timeout` is 20s rather than the default hour, and that is a gate rather than
/// impatience.** Two of this engine's mutations do not change an answer, they stop the loop
/// terminating — `batches` returning one empty batch dispatches nothing for ever, and
/// `attempt += 1` read as `*=` leaves the counter at nought. Against a one-hour bound those are
/// reported as *timeouts*, which is neither caught nor survived and is how a mutant hides in a
/// shard's exit code. Against this one they are ordinary failures. Every command here is a mock
/// and returns instantly, so twenty seconds is four orders of magnitude of headroom.
fn side_by_side(auto_rollback: bool) -> TransactionConfig {
    TransactionConfig {
        max_concurrent: 4,
        max_retries: 0,
        auto_rollback,
        node_timeout: Duration::from_secs(10),
        total_timeout: Duration::from_secs(20),
        ..TransactionConfig::default()
    }
}

/// A wave is work handed out after the engine went quiet, not every pass that dispatched
/// something.
///
/// Two independent two-node chains, one per manager. The scheduler dispatches eagerly, so the
/// second chain's root is still running when the first chain's child is handed out — and
/// counting that as a wave counts the scheduler's own overlap against it. Under the correct
/// rule this graph takes at most its depth in waves however the two chains interleave; a rule
/// that counted passes takes one per dispatch and reports a plan more serial than its shape.
#[tokio::test]
async fn overlapping_chains_do_not_each_count_as_a_wave() {
    let kernel = TestKernel::new().await;
    let log = shared_log();
    let (_one, _two, registry) = two_managers(&log);

    let mut graph = StableDiGraph::new();
    let root_one = graph.add_node(install("pkg-one-root", "mock-one"));
    let child_one = graph.add_node(install("pkg-one-child", "mock-one"));
    let root_two = graph.add_node(install("pkg-two-root", "mock-two"));
    let child_two = graph.add_node(install("pkg-two-child", "mock-two"));
    graph.add_edge(root_one, child_one, ());
    graph.add_edge(root_two, child_two, ());

    let mut tx = Transaction::with_config(
        graph,
        registry,
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        side_by_side(false),
    );
    tx.execute().await.expect("four independent installs");

    let shape = tx
        .last_scheduling
        .expect("a run that reached closure records its shape");
    assert_eq!(shape.packages, 4);
    assert_eq!(
        shape.depth, 2,
        "two chains of two: the longest of them is what bounds the waves"
    );
    assert!(
        shape.waves <= shape.depth,
        "the engine went idle {} times over a graph {} levels deep — it is counting passes \
         rather than generations, which reports the scheduler's own overlap as serialisation",
        shape.waves,
        shape.depth
    );
}

/// A node runs when its **last** dependency finishes, not its first.
///
/// The two roots are on different managers, so they finish as two separate joins and the child's
/// count comes down in two steps. Reading either step as "ready" dispatches the child with a
/// dependency still outstanding — and then dispatches it a second time when the other one lands,
/// so the manager is asked to install the same package twice. Both halves are asserted: one
/// command for the child, and it is the last of the three.
#[tokio::test]
async fn a_node_waits_for_its_last_dependency_and_runs_once() {
    let kernel = TestKernel::new().await;
    let log = shared_log();
    let (one, _two, registry) = two_managers(&log);

    let mut graph = StableDiGraph::new();
    let root_one = graph.add_node(install("pkg-first-root", "mock-one"));
    let root_two = graph.add_node(install("pkg-second-root", "mock-two"));
    let child = graph.add_node(install("pkg-child", "mock-one"));
    graph.add_edge(root_one, child, ());
    graph.add_edge(root_two, child, ());

    let mut tx = Transaction::with_config(
        graph,
        registry,
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        side_by_side(false),
    );
    tx.execute().await.expect("three installs");

    let calls = one.calls();
    let child_calls: Vec<&String> = calls.iter().filter(|c| c.contains("pkg-child")).collect();
    assert_eq!(
        child_calls.len(),
        1,
        "the child was handed to a manager {} times: {:?}",
        child_calls.len(),
        calls
    );
    assert_eq!(
        calls.len(),
        3,
        "three nodes, three commands, and the child last: {:?}",
        calls
    );
    assert!(
        calls[2].contains("pkg-child"),
        "the child ran before one of the two roots it requires: {:?}",
        calls
    );
}

/// A resumed transaction is measured on the work it has left, not on the work somebody else
/// already did.
///
/// The first run installs the root and fails on the child. The second run over the same
/// transaction has one node to do and a one-level graph to do it in — and if the count of
/// remaining work were the whole graph plus what was already finished, the latency rule would be
/// judging every resume against a shape no run had.
#[tokio::test]
async fn a_resumed_run_counts_only_what_it_has_left() {
    let kernel = TestKernel::new().await;
    let log = shared_log();
    let first = RecordingBackend::named("mock-one", &log).build();
    let second = RecordingBackend::named("mock-two", &log)
        .failing("pkg-child")
        .build();
    let mut registry = BackendRegistry::new();
    registry.register(capabilities(&first));
    registry.register(capabilities(&second));

    let mut graph = StableDiGraph::new();
    let root = graph.add_node(install("pkg-root", "mock-one"));
    let child = graph.add_node(install("pkg-child", "mock-two"));
    graph.add_edge(root, child, ());

    let mut tx = Transaction::with_config(
        graph,
        Arc::new(registry),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        // No rollback: this run is the setup for the next one, and compensating the root would
        // take away the very thing that makes the second run a resume.
        side_by_side(false),
    );
    tx.execute()
        .await
        .expect_err("the child fails, leaving the root done");
    assert!(
        tx.last_scheduling.is_none(),
        "a run that returned early went idle fewer times than its graph has levels, so it has \
         no shape to report"
    );

    second.let_it_succeed("pkg-child");
    tx.execute().await.expect("the resumed run finishes it");

    let shape = tx.last_scheduling.expect("the resumed run reached closure");
    assert_eq!(
        shape.packages, 1,
        "one node was left; the graph has two and one of them was already done"
    );
    assert_eq!(
        shape.depth, 1,
        "the remaining node has no unfinished dependency, so what is left is one level deep"
    );
}

/// A retry waits before it runs, and the first attempt does not.
///
/// **`attempt > 1` is the whole of that sentence, and reversing it costs nothing observable in
/// the answer.** A run whose manager fails once and works on the retry still ends `Ok`, still
/// records two commands, and still reports one retry — because the counter and the loop are not
/// what the gate controls. What it controls is whether the backoff and the manager-lock verdict
/// happen at all, and the only reading of that is the clock: a retry that waited took at least
/// the initial backoff, and one that did not took none of it.
///
/// The bound is a lower one, against a backoff four times larger, because a mock command returns
/// instantly and the failure being guarded against is *no wait*, not a short one.
#[tokio::test]
async fn a_retry_waits_the_backoff_and_the_first_attempt_does_not() {
    use std::time::Instant;

    let kernel = TestKernel::new().await;
    let log = shared_log();
    let backend = RecordingBackend::named("mock-flaky", &log)
        .flaky_once("pkg-retried")
        .build();
    let mut registry = BackendRegistry::new();
    registry.register(capabilities(&backend));

    let mut graph = StableDiGraph::new();
    graph.add_node(install("pkg-retried", "mock-flaky"));

    const BACKOFF: Duration = Duration::from_millis(400);
    let mut tx = Transaction::with_config(
        graph,
        Arc::new(registry),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        TransactionConfig {
            max_retries: 1,
            initial_backoff: BACKOFF,
            ..side_by_side(false)
        },
    );

    let started = Instant::now();
    let results = tx
        .execute_with_telemetry()
        .await
        .expect("the second attempt succeeds");
    let took = started.elapsed();

    assert_eq!(
        backend.calls(),
        vec![
            "mock-flaky install pkg-retried".to_string(),
            "mock-flaky install pkg-retried".to_string(),
        ],
        "the manager should have been asked twice — once failing, once not"
    );
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].retries, 1,
        "one attempt after the first is one retry"
    );
    assert!(
        took >= BACKOFF / 2,
        "the whole run took {took:?} against a {BACKOFF:?} backoff, so the retry did not wait — \
         which also means no failure on this path is ever asked whether another manager is \
         holding the lock"
    );
}

/// A graph with no schedulable node is refused by name rather than spun on.
///
/// The planner rejects a cycle long before the engine sees one, so this is the backstop for a
/// graph built some other way. It is reached the moment nothing is ready and nothing is running,
/// which is the one state the loop cannot make progress out of — and the alternative to naming
/// it is a process that turns the loop for ever against a condition that will not change.
#[tokio::test]
async fn a_graph_nothing_can_start_is_refused_by_name() {
    let kernel = TestKernel::new().await;
    let log = shared_log();
    let (_one, _two, registry) = two_managers(&log);

    let mut graph = StableDiGraph::new();
    let a = graph.add_node(install("pkg-loop-a", "mock-one"));
    let b = graph.add_node(install("pkg-loop-b", "mock-one"));
    graph.add_edge(a, b, ());
    graph.add_edge(b, a, ());

    let mut tx = Transaction::with_config(
        graph,
        registry,
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        side_by_side(false),
    );

    let err = tx
        .execute()
        .await
        .expect_err("nothing in this graph can ever be dispatched");
    let said = err.to_string();
    assert!(
        said.contains("Cycle"),
        "the refusal has to name the shape, or it is indistinguishable from a failed install: \
         {said}"
    );
}
